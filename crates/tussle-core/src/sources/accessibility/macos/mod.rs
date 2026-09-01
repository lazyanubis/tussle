//! macOS implementation of the Accessibility-API menu enumerator.

mod ax;
mod menu_walker;
mod modifiers;
mod running_apps;

use std::thread;

use crate::{Binding, BindingSource, ScanError};

const MAX_CONCURRENT_APP_WALKS: usize = 128;
const MAX_RUNNING_APPS: usize = 2048;
const MAX_TOTAL_BINDING_STORAGE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn scan(
    messaging_timeout: f32,
    max_concurrency: usize,
    bundle_filter: &[String],
) -> Result<Vec<Binding>, ScanError> {
    if !is_trusted() {
        tracing::warn!("Accessibility permission missing — skipping menu enumeration");
        return Ok(Vec::new());
    }

    // Each app's walk is a sequence of synchronous AX IPC calls — wallclock
    // is dominated by waiting for the target app's main thread to respond,
    // not by CPU. Spawning one OS thread per app lets the waits overlap;
    // since the threads are sleeping (not running), tens of concurrent
    // threads cost basically nothing. A bounded pool (rayon, threadpool)
    // would cap us at CPU-core count and serialize the rest, defeating the
    // point.
    //
    // We still apply a caller-selected cap plus an unconditional hard cap by
    // walking apps in chunks. This bounds thread creation even if a library
    // caller passes zero or an unexpectedly large value.
    let mut apps = running_apps::list_running_apps();
    if !bundle_filter.is_empty() {
        let filter_lc: Vec<String> = bundle_filter.iter().map(|s| s.to_lowercase()).collect();
        let before = apps.len();
        apps.retain(|a| {
            matches_bundle_filter(a.bundle_id.as_deref(), a.app_name.as_deref(), &filter_lc)
        });
        tracing::debug!(
            kept = apps.len(),
            dropped = before - apps.len(),
            "applied bundle filter",
        );
    }
    if apps.len() > MAX_RUNNING_APPS {
        tracing::warn!(
            found = apps.len(),
            limit = MAX_RUNNING_APPS,
            "truncating running-app scan at safety limit",
        );
        apps.truncate(MAX_RUNNING_APPS);
    }
    let chunk_size = bounded_concurrency(max_concurrency);

    let mut bindings = Vec::new();
    let mut remaining_binding_bytes = MAX_TOTAL_BINDING_STORAGE_BYTES;
    for batch in apps.chunks(chunk_size) {
        let batch_bindings: Vec<Binding> = thread::scope(|s| {
            let handles: Vec<_> = batch
                .iter()
                .filter_map(|app| {
                    thread::Builder::new()
                        .spawn_scoped(s, move || {
                            menu_walker::walk_app_menus(app, messaging_timeout)
                        })
                        .map_err(|error| {
                            tracing::warn!(
                                bundle = app.bundle_id.as_deref().unwrap_or("?"),
                                error = %error,
                                "could not spawn AX worker",
                            );
                        })
                        .ok()
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        for binding in batch_bindings {
            let storage_bytes = binding_storage_bytes(&binding);
            if storage_bytes > remaining_binding_bytes {
                tracing::warn!(
                    limit = MAX_TOTAL_BINDING_STORAGE_BYTES,
                    "stopped AX scan at total binding storage safety limit",
                );
                return Ok(bindings);
            }
            remaining_binding_bytes -= storage_bytes;
            bindings.push(binding);
        }
    }
    Ok(bindings)
}

fn binding_storage_bytes(binding: &Binding) -> usize {
    const BINDING_OVERHEAD_BYTES: usize = 256;

    let source_bytes = match &binding.source {
        BindingSource::SystemSymbolicHotkey { .. } => 0,
        BindingSource::AppMenuOverride {
            bundle_id,
            menu_item,
        } => bundle_id.len().saturating_add(menu_item.len()),
        BindingSource::AppMenuItem {
            bundle_id,
            app_name,
            menu_path,
        } => bundle_id
            .len()
            .saturating_add(app_name.as_deref().map_or(0, str::len))
            .saturating_add(
                menu_path
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            ),
    };

    BINDING_OVERHEAD_BYTES
        .saturating_add(binding.label.len())
        .saturating_add(source_bytes)
}

fn bounded_concurrency(requested: usize) -> usize {
    if requested == 0 {
        MAX_CONCURRENT_APP_WALKS
    } else {
        requested.min(MAX_CONCURRENT_APP_WALKS)
    }
}

pub(super) fn is_trusted() -> bool {
    // SAFETY: AXIsProcessTrusted is thread-safe and has no preconditions.
    unsafe { accessibility_sys::AXIsProcessTrusted() }
}

/// Whether the app's bundle id or display name contains any of the
/// (already lowercased) substrings in `filter_lc`. An empty filter
/// matches everything; that case should be short-circuited by the
/// caller, not handled here.
fn matches_bundle_filter(
    bundle_id: Option<&str>,
    app_name: Option<&str>,
    filter_lc: &[String],
) -> bool {
    let bundle_lc = bundle_id.map(str::to_lowercase);
    let name_lc = app_name.map(str::to_lowercase);
    filter_lc.iter().any(|f| {
        bundle_lc.as_deref().is_some_and(|s| s.contains(f))
            || name_lc.as_deref().is_some_and(|s| s.contains(f))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_matches_bundle_id_substring_case_insensitive() {
        let filter = vec!["rustrover".to_string()];
        assert!(matches_bundle_filter(
            Some("com.jetbrains.RustRover"),
            None,
            &filter
        ));
        assert!(matches_bundle_filter(
            Some("COM.JETBRAINS.RUSTROVER"),
            None,
            &filter
        ));
    }

    #[test]
    fn filter_matches_app_name_substring_case_insensitive() {
        let filter = vec!["chrome".to_string()];
        assert!(matches_bundle_filter(
            Some("com.google.Chrome"),
            Some("Google Chrome"),
            &filter
        ));
        assert!(matches_bundle_filter(None, Some("Google Chrome"), &filter));
    }

    #[test]
    fn filter_misses_unrelated_app() {
        let filter = vec!["webstorm".to_string()];
        assert!(!matches_bundle_filter(
            Some("com.apple.finder"),
            Some("Finder"),
            &filter
        ));
    }

    #[test]
    fn filter_or_semantics_across_multiple_terms() {
        let filter = vec!["webstorm".to_string(), "datagrip".to_string()];
        assert!(matches_bundle_filter(
            Some("com.jetbrains.WebStorm"),
            None,
            &filter
        ));
        assert!(matches_bundle_filter(
            Some("com.jetbrains.datagrip"),
            None,
            &filter
        ));
        assert!(!matches_bundle_filter(
            Some("com.apple.finder"),
            None,
            &filter
        ));
    }

    #[test]
    fn concurrency_is_never_unbounded() {
        assert_eq!(bounded_concurrency(0), MAX_CONCURRENT_APP_WALKS);
        assert_eq!(bounded_concurrency(32), 32);
        assert_eq!(bounded_concurrency(usize::MAX), MAX_CONCURRENT_APP_WALKS);
    }
}
