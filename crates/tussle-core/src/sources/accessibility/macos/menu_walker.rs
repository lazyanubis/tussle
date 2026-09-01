//! Walk an app's menu bars (main + status-bar extras) and harvest every
//! menu item with a key equivalent.

use accessibility_sys::{
    AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetMessagingTimeout,
    kAXMenuBarAttribute, kAXMenuItemCmdCharAttribute, kAXMenuItemCmdModifiersAttribute,
    kAXTitleAttribute,
};

use crate::{Binding, BindingSource, Key, KeyCombo};

use super::ax::{borrowed_element, copy_children, copy_element, copy_i64, copy_string};
use super::modifiers::decode_ax_modifiers;
use super::running_apps::RunningApp;

/// Hard cap on menu recursion depth to defend against pathological apps.
const MAX_MENU_DEPTH: usize = 16;
/// Hard cap on AX nodes visited per app. Depth alone does not bound a very
/// wide or cyclic menu graph supplied by a pathological process.
const MAX_MENU_NODES_PER_APP: usize = 10_000;
/// Bound retained app-controlled text (labels, paths, app identity) per app.
const MAX_BINDING_STORAGE_BYTES_PER_APP: usize = 1024 * 1024;
/// Avoid passing non-finite or excessive caller-controlled timeouts across FFI.
const MAX_MESSAGING_TIMEOUT_SECS: f32 = 60.0;

pub(super) fn walk_app_menus(app: &RunningApp, messaging_timeout: f32) -> Vec<Binding> {
    let started = std::time::Instant::now();
    let element = unsafe { AXUIElementCreateApplication(app.pid) };
    if element.is_null() {
        return Vec::new();
    }

    // Per-app timeout. Set on the application element propagates to
    // all child elements queried through it.
    if let Some(messaging_timeout) = bounded_timeout(messaging_timeout) {
        unsafe { AXUIElementSetMessagingTimeout(element, messaging_timeout) };
    }

    let mut bindings = Vec::new();
    let mut remaining_nodes = MAX_MENU_NODES_PER_APP;
    let mut remaining_binding_bytes = MAX_BINDING_STORAGE_BYTES_PER_APP;

    // Main menu bar (visible when app is frontmost).
    if let Some(menu_bar) = copy_element(element, kAXMenuBarAttribute) {
        walk_menu(
            menu_bar,
            app,
            &[],
            0,
            &mut remaining_nodes,
            &mut remaining_binding_bytes,
            &mut bindings,
        );
        unsafe { core_foundation::base::CFRelease(menu_bar as _) };
    }

    // Status-bar (NSStatusItem) dropdowns. Menubar-only apps like PixPin
    // expose their main shortcuts here, not on the regular menu bar.
    if let Some(extras) = copy_element(element, "AXExtrasMenuBar") {
        walk_menu(
            extras,
            app,
            &[],
            0,
            &mut remaining_nodes,
            &mut remaining_binding_bytes,
            &mut bindings,
        );
        unsafe { core_foundation::base::CFRelease(extras as _) };
    }

    unsafe { core_foundation::base::CFRelease(element as _) };

    tracing::debug!(
        bundle = app.bundle_id.as_deref().unwrap_or("?"),
        bindings = bindings.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "walked app",
    );
    if remaining_nodes == 0 {
        tracing::warn!(
            bundle = app.bundle_id.as_deref().unwrap_or("?"),
            limit = MAX_MENU_NODES_PER_APP,
            "stopped walking app after reaching AX node safety limit",
        );
    }
    if remaining_binding_bytes == 0 {
        tracing::warn!(
            bundle = app.bundle_id.as_deref().unwrap_or("?"),
            limit = MAX_BINDING_STORAGE_BYTES_PER_APP,
            "stopped walking app after reaching binding storage safety limit",
        );
    }
    bindings
}

/// Recursively walk a menu element. Each menu has child menu items;
/// each menu item with a submenu has a child of type `AXMenu` whose
/// children are the inner items.
fn walk_menu(
    menu: AXUIElementRef,
    app: &RunningApp,
    path: &[String],
    depth: usize,
    remaining_nodes: &mut usize,
    remaining_binding_bytes: &mut usize,
    out: &mut Vec<Binding>,
) {
    if depth > MAX_MENU_DEPTH {
        return;
    }

    let Some(children) = copy_children(menu) else {
        return;
    };

    for child in &children {
        if *remaining_nodes == 0 || *remaining_binding_bytes == 0 {
            return;
        }
        let Some(child) = borrowed_element(*child) else {
            continue;
        };
        *remaining_nodes -= 1;
        visit_item(
            child,
            app,
            path,
            depth,
            remaining_nodes,
            remaining_binding_bytes,
            out,
        );
    }
}

fn visit_item(
    item: AXUIElementRef,
    app: &RunningApp,
    path: &[String],
    depth: usize,
    remaining_nodes: &mut usize,
    remaining_binding_bytes: &mut usize,
    out: &mut Vec<Binding>,
) {
    let title = copy_string(item, kAXTitleAttribute).unwrap_or_default();
    let mut new_path: Vec<String> = path.to_vec();
    if !title.is_empty() {
        new_path.push(title.clone());
    }

    if let Some(combo) = read_key_equivalent(item) {
        let storage_bytes = binding_storage_bytes(app, &new_path, &title);
        if storage_bytes > *remaining_binding_bytes {
            *remaining_binding_bytes = 0;
            return;
        }
        *remaining_binding_bytes -= storage_bytes;
        out.push(Binding {
            combo,
            source: BindingSource::AppMenuItem {
                bundle_id: app.bundle_id.clone().unwrap_or_default(),
                app_name: app.app_name.clone(),
                menu_path: new_path.clone(),
            },
            label: title.clone(),
        });
    }

    // A menu item that opens a submenu has a child element of type
    // AXMenu whose children are the submenu's items.
    if let Some(grand) = copy_children(item) {
        for sub in &grand {
            if *remaining_nodes == 0 || *remaining_binding_bytes == 0 {
                return;
            }
            let Some(sub) = borrowed_element(*sub) else {
                continue;
            };
            walk_menu(
                sub,
                app,
                &new_path,
                depth + 1,
                remaining_nodes,
                remaining_binding_bytes,
                out,
            );
        }
    }
}

fn read_key_equivalent(item: AXUIElementRef) -> Option<KeyCombo> {
    let ch = copy_string(item, kAXMenuItemCmdCharAttribute)?;
    let first_char = ch.chars().next()?;
    let mask = copy_i64(item, kAXMenuItemCmdModifiersAttribute).unwrap_or(0);
    Some(KeyCombo {
        modifiers: decode_ax_modifiers(mask),
        key: Key::from_char(first_char),
    })
}

fn binding_storage_bytes(app: &RunningApp, path: &[String], title: &str) -> usize {
    const BINDING_OVERHEAD_BYTES: usize = 256;

    app.bundle_id
        .as_deref()
        .map_or(0, str::len)
        .saturating_add(app.app_name.as_deref().map_or(0, str::len))
        .saturating_add(
            path.iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(title.len())
        .saturating_add(BINDING_OVERHEAD_BYTES)
}

fn bounded_timeout(value: f32) -> Option<f32> {
    (value.is_finite() && value > 0.0).then(|| value.min(MAX_MESSAGING_TIMEOUT_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messaging_timeout_rejects_invalid_values_and_caps_large_ones() {
        assert_eq!(bounded_timeout(0.0), None);
        assert_eq!(bounded_timeout(-1.0), None);
        assert_eq!(bounded_timeout(f32::NAN), None);
        assert_eq!(bounded_timeout(f32::INFINITY), None);
        assert_eq!(bounded_timeout(1.5), Some(1.5));
        assert_eq!(bounded_timeout(600.0), Some(60.0));
    }
}
