//! Parser for `~/Library/Preferences/com.apple.symbolichotkeys.plist`.
//!
//! macOS stores user customizations of system shortcuts (Spotlight, Mission
//! Control, screenshots, ...) as numeric IDs in this plist. Each entry is
//! `{ enabled, value: { parameters: [char_code, virtual_keycode, mask], type } }`.
//!
//! macOS DEFAULTS are NOT stored in this file — they live hardcoded in the
//! system. We therefore maintain `macos_defaults()` below and merge it with
//! the plist contents to produce a complete picture: defaults overlaid by
//! whatever the user has customized or disabled.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::combo::vk_to_named;
use crate::{Binding, BindingSource, Key, KeyCombo, Modifiers, NamedKey, ScanError};

use super::Source;
use super::plist_file;

/// Reads `com.apple.symbolichotkeys.plist` and merges its contents with
/// macOS's hardcoded default table.
#[derive(Debug, Clone)]
pub struct SymbolicHotkeys {
    plist_path: PathBuf,
}

impl SymbolicHotkeys {
    /// Construct a parser pointed at the given plist path.
    pub fn new(plist_path: PathBuf) -> Self {
        Self { plist_path }
    }
}

impl Source for SymbolicHotkeys {
    fn name(&self) -> &'static str {
        "symbolichotkeys"
    }

    fn scan(&self) -> Result<Vec<Binding>, ScanError> {
        scan(&self.plist_path)
    }
}

/// `parameters` array index for the printable character code, the macOS
/// virtual keycode, and the NSEvent modifier mask, respectively.
const PARAM_CHAR: usize = 0;
const PARAM_VK: usize = 1;
const PARAM_MASK: usize = 2;

/// Sentinel value Apple writes when a parameter slot is unset.
const UNSET: i64 = 65535;
/// The real macOS table is only a few hundred entries. Bound corrupted or
/// attacker-controlled preference data before allocating the override map.
const MAX_SYMBOLIC_HOTKEY_ENTRIES: usize = 4096;

// NSEvent modifier flag bits, from `AppKit/NSEvent.h`:
//   NSEventModifierFlagShift    = 1 << 17  = 0x0002_0000
//   NSEventModifierFlagControl  = 1 << 18  = 0x0004_0000
//   NSEventModifierFlagOption   = 1 << 19  = 0x0008_0000
//   NSEventModifierFlagCommand  = 1 << 20  = 0x0010_0000
//   NSEventModifierFlagFunction = 1 << 23  = 0x0080_0000
const NS_SHIFT: u64 = 1 << 17;
const NS_CTRL: u64 = 1 << 18;
const NS_OPT: u64 = 1 << 19;
const NS_CMD: u64 = 1 << 20;
const NS_FN: u64 = 1 << 23;

/// What the user's plist says about a particular hotkey ID.
#[derive(Debug, Clone, Copy)]
enum Override {
    /// User explicitly disabled this shortcut.
    Disabled,
    /// User has the shortcut enabled but has not customized the combo —
    /// macOS uses its built-in default.
    EnabledWithDefault,
    /// User has bound this shortcut to a specific combo.
    Custom(KeyCombo),
}

/// Parse the plist and merge with macOS's default symbolic hotkey table to
/// produce the final set of bindings. Disabled entries are filtered out.
fn scan(path: &Path) -> Result<Vec<Binding>, ScanError> {
    let overrides = parse_overrides(path)?;
    let defaults = macos_defaults();

    let mut bindings = Vec::new();
    let mut handled: HashSet<u32> = HashSet::new();

    // Pass 1: every ID we know a default for. Apply overrides on top.
    for (id, default_combo) in &defaults {
        handled.insert(*id);
        let combo = match overrides.get(id) {
            Some(Override::Disabled) => continue,
            Some(Override::Custom(c)) => *c,
            Some(Override::EnabledWithDefault) | None => *default_combo,
        };
        bindings.push(emit(*id, combo));
    }

    // Pass 2: IDs the user has customized for which we have no default. Surface
    // them with a generic label so unmapped customizations remain visible.
    for (id, entry) in &overrides {
        if handled.contains(id) {
            continue;
        }
        if let Override::Custom(c) = entry {
            bindings.push(emit(*id, *c));
        }
    }

    bindings.sort_by_key(|b| {
        if let BindingSource::SystemSymbolicHotkey { id } = &b.source {
            *id
        } else {
            unreachable!("this parser only emits SystemSymbolicHotkey bindings")
        }
    });

    Ok(bindings)
}

fn emit(id: u32, combo: KeyCombo) -> Binding {
    Binding {
        combo,
        source: BindingSource::SystemSymbolicHotkey { id },
        label: label_for(id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Symbolic hotkey #{id}")),
    }
}

fn parse_overrides(path: &Path) -> Result<HashMap<u32, Override>, ScanError> {
    let value = plist_file::parse_value(path)?;

    let root = value.as_dictionary().ok_or_else(|| ScanError::Schema {
        path: path.to_path_buf(),
        message: "root is not a dictionary".into(),
    })?;

    let entries = root
        .get("AppleSymbolicHotKeys")
        .and_then(|v| v.as_dictionary())
        .ok_or_else(|| ScanError::Schema {
            path: path.to_path_buf(),
            message: "missing AppleSymbolicHotKeys dict".into(),
        })?;

    let mut map = HashMap::new();
    for (id_str, entry) in entries.iter().take(MAX_SYMBOLIC_HOTKEY_ENTRIES) {
        let Ok(id) = id_str.parse::<u32>() else {
            continue;
        };
        let Some(entry_dict) = entry.as_dictionary() else {
            continue;
        };

        let enabled = entry_dict
            .get("enabled")
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);
        if !enabled {
            map.insert(id, Override::Disabled);
            continue;
        }

        let Some(value_dict) = entry_dict.get("value").and_then(|v| v.as_dictionary()) else {
            map.insert(id, Override::EnabledWithDefault);
            continue;
        };

        let Some(params) = value_dict.get("parameters").and_then(|v| v.as_array()) else {
            continue;
        };
        if params.len() < 3 {
            continue;
        }

        let char_code = params[PARAM_CHAR].as_signed_integer().unwrap_or(UNSET);
        let vk = params[PARAM_VK].as_signed_integer().unwrap_or(UNSET);
        let mask = params[PARAM_MASK].as_signed_integer().unwrap_or(0);

        // (65535, 65535, *) is Apple's "no override" placeholder; treat as
        // enabled-with-default rather than a real custom binding.
        if char_code == UNSET && vk == UNSET {
            map.insert(id, Override::EnabledWithDefault);
            continue;
        }

        map.insert(
            id,
            Override::Custom(KeyCombo {
                modifiers: decode_modifiers(mask as u64),
                key: decode_key(char_code, vk),
            }),
        );
    }

    Ok(map)
}

/// macOS default bindings for symbolic hotkey IDs, hand-curated against
/// macOS Tahoe (26.x).
///
/// **Maintenance** (do this once per macOS major release):
///
///   1. On a fresh install of the new macOS, open System Settings → Keyboard
///      → Keyboard Shortcuts and note the default for each ID.
///   2. Cross-reference IDs with the contents of
///      `~/Library/Preferences/com.apple.symbolichotkeys.plist` after toggling
///      shortcuts in System Settings (the file fills in as you interact).
///   3. Update / add entries below; bump the version comment.
///
/// **Coverage** is intentionally partial — only IDs we're confident about
/// are listed. Unknown IDs that the user customizes are still surfaced
/// (with a generic "Symbolic hotkey #N" label) by the second pass in `scan`.
///
/// **Sources**: combos verified against Apple Support HT201236 (Mac keyboard
/// shortcuts), virtual keycodes from `<HIToolbox/Events.h>`, and System
/// Settings on a Tahoe install.
fn macos_defaults() -> Vec<(u32, KeyCombo)> {
    use NamedKey::*;
    let cmd = Modifiers::CMD;
    let shift = Modifiers::SHIFT;
    let ctrl = Modifiers::CTRL;
    let opt = Modifiers::OPT;
    let combo = |m, k| KeyCombo {
        modifiers: m,
        key: k,
    };

    vec![
        // Mission Control / Spaces
        (32, combo(ctrl, Key::Named(Up))),
        (33, combo(ctrl, Key::Named(Down))),
        (79, combo(ctrl, Key::Named(Left))),
        (81, combo(ctrl, Key::Named(Right))),
        (118, combo(ctrl, Key::Char('1'))),
        (119, combo(ctrl, Key::Char('2'))),
        (120, combo(ctrl, Key::Char('3'))),
        (121, combo(ctrl, Key::Char('4'))),
        // Screenshots
        (28, combo(shift | cmd, Key::Char('3'))),
        (29, combo(ctrl | shift | cmd, Key::Char('3'))),
        (30, combo(shift | cmd, Key::Char('4'))),
        (31, combo(ctrl | shift | cmd, Key::Char('4'))),
        (184, combo(shift | cmd, Key::Char('5'))),
        // Spotlight
        (64, combo(cmd, Key::Named(Space))),
        (65, combo(cmd | opt, Key::Named(Space))),
        // Input source switching
        (60, combo(ctrl, Key::Named(Space))),
        (61, combo(ctrl | opt, Key::Named(Space))),
    ]
}

/// Human-readable label for a known symbolic hotkey ID, or `None` if we don't
/// have a mapping yet. Labels track Apple's wording in System Settings →
/// Keyboard → Keyboard Shortcuts.
///
/// Coverage is partial; new IDs should be added as they show up in real
/// fixtures rather than guessed at.
fn label_for(id: u32) -> Option<&'static str> {
    Some(match id {
        // Keyboard navigation (Keyboard Access pane)
        7 => "Move focus to the menu bar",
        8 => "Move focus to the Dock",
        9 => "Move focus to the active or next window",
        10 => "Move focus to the window toolbar",
        11 => "Move focus to the floating window",
        12 => "Toggle keyboard access",
        13 => "Change the way Tab moves focus",
        27 => "Move focus to next window in application",
        51 => "Move focus to the window drawer",
        57 => "Move focus to the status menus",

        // Screenshots
        28 => "Save picture of screen as a file",
        29 => "Copy picture of screen to the clipboard",
        30 => "Save picture of selected area as a file",
        31 => "Copy picture of selected area to the clipboard",
        184 => "Screenshot and recording options",

        // Mission Control
        32 => "Mission Control",
        33 => "Application windows",
        36 => "Show Desktop",

        // Spotlight
        64 => "Show Spotlight search",
        65 => "Show Finder search window",

        // Input sources
        60 => "Select the previous input source",
        61 => "Select the next source in the Input menu",

        // Spaces (the duplicate IDs are the regular vs. modified-arrow forms)
        79 | 80 => "Move left a space",
        81 | 82 => "Move right a space",
        118 => "Switch to Desktop 1",
        119 => "Switch to Desktop 2",
        120 => "Switch to Desktop 3",
        121 => "Switch to Desktop 4",

        // Other system
        52 => "Toggle Dock hiding",
        59 => "Toggle VoiceOver",
        160 => "Show Launchpad",
        163 => "Show Notification Center",
        175 => "Toggle Do Not Disturb",

        // Touch Bar
        181 => "Save picture of the Touch Bar as a file",
        182 => "Copy picture of the Touch Bar to the clipboard",

        _ => return None,
    })
}

fn decode_modifiers(mask: u64) -> Modifiers {
    let mut m = Modifiers::empty();
    if mask & NS_CMD != 0 {
        m |= Modifiers::CMD;
    }
    if mask & NS_OPT != 0 {
        m |= Modifiers::OPT;
    }
    if mask & NS_CTRL != 0 {
        m |= Modifiers::CTRL;
    }
    if mask & NS_SHIFT != 0 {
        m |= Modifiers::SHIFT;
    }
    if mask & NS_FN != 0 {
        m |= Modifiers::FN;
    }
    m
}

fn decode_key(char_code: i64, vk: i64) -> Key {
    // Virtual keycode wins for keys with a canonical NamedKey, since the vk
    // is layout-independent while the char_code reflects the active layout.
    if vk != UNSET
        && (0..=u16::MAX as i64).contains(&vk)
        && let Some(named) = vk_to_named(vk as u16)
    {
        return Key::Named(named);
    }

    // Fall back to the printable character if Apple set one.
    if char_code != UNSET
        && (0..=u32::MAX as i64).contains(&char_code)
        && let Some(c) = char::from_u32(char_code as u32)
        && !c.is_control()
    {
        return Key::Char(c);
    }

    // Last resort: surface the raw vk so the caller can still see what was
    // bound, even if we don't have a name for it.
    if vk != UNSET && (0..=u16::MAX as i64).contains(&vk) {
        return Key::Virtual(vk as u16);
    }

    Key::Virtual(0)
}
