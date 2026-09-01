//! Parser for per-app `NSUserKeyEquivalents` overrides.
//!
//! Each app's `~/Library/Preferences/<bundle_id>.plist` may contain an
//! `NSUserKeyEquivalents` dictionary that the user populates via
//! System Settings → Keyboard → Keyboard Shortcuts → App Shortcuts.
//! Keys are menu item titles (`"New"`, `"Save All"`); values are NSText
//! keystroke shorthand:
//!
//!   - `@` = Command
//!   - `~` = Option
//!   - `$` = Shift
//!   - `^` = Control
//!
//! followed by the literal key character. So `@~n` denotes ⌘⌥N.

use std::path::{Path, PathBuf};

use crate::{Binding, BindingSource, Key, KeyCombo, Modifiers, ScanError};

use super::Source;
use super::plist_file;

const MAX_MENU_ITEM_BYTES: usize = 1024;
const MAX_BINDINGS_PER_PLIST: usize = 1024;
const MAX_PREFERENCE_PLISTS: usize = 10_000;
const MAX_TOTAL_OVERRIDE_BINDINGS: usize = 10_000;

/// Walks every plist in a preferences directory looking for
/// `NSUserKeyEquivalents` overrides — the per-app menu shortcut customizations
/// that the user set via System Settings → Keyboard → App Shortcuts.
#[derive(Debug, Clone)]
pub struct AppMenuOverrides {
    prefs_dir: PathBuf,
}

impl AppMenuOverrides {
    /// Construct a source that walks the given preferences directory.
    /// Typically `~/Library/Preferences`, but tests pass a fixture root.
    pub fn new(prefs_dir: PathBuf) -> Self {
        Self { prefs_dir }
    }
}

impl Source for AppMenuOverrides {
    fn name(&self) -> &'static str {
        "nsuserkeyequivalents"
    }

    fn scan(&self) -> Result<Vec<Binding>, ScanError> {
        scan(&self.prefs_dir)
    }
}

/// Parse a single `<bundle_id>.plist` for its `NSUserKeyEquivalents` dict.
///
/// Returns an empty `Vec` (not an error) when the file has no overrides —
/// most apps don't, so an empty result is the common case.
pub fn parse(path: &Path) -> Result<Vec<Binding>, ScanError> {
    let bundle_id = bundle_id_from_path(path).ok_or_else(|| ScanError::Schema {
        path: path.to_path_buf(),
        message: "filename has no bundle id stem".into(),
    })?;

    let value = plist_file::parse_value(path)?;

    let Some(root) = value.as_dictionary() else {
        // Plists with non-dictionary roots have no NSUserKeyEquivalents.
        return Ok(Vec::new());
    };

    let Some(equivs) = root
        .get("NSUserKeyEquivalents")
        .and_then(|v| v.as_dictionary())
    else {
        return Ok(Vec::new());
    };

    let mut bindings = Vec::new();
    for (menu_item, value) in equivs {
        if bindings.len() == MAX_BINDINGS_PER_PLIST {
            break;
        }
        if menu_item.len() > MAX_MENU_ITEM_BYTES {
            continue;
        }
        let Some(shorthand) = value.as_string() else {
            continue;
        };
        let Some(combo) = parse_keystroke(shorthand) else {
            continue;
        };
        bindings.push(Binding {
            combo,
            source: BindingSource::AppMenuOverride {
                bundle_id: bundle_id.clone(),
                menu_item: menu_item.clone(),
            },
            label: menu_item.clone(),
        });
    }

    bindings.sort_by(|a, b| match (&a.source, &b.source) {
        (
            BindingSource::AppMenuOverride { menu_item: am, .. },
            BindingSource::AppMenuOverride { menu_item: bm, .. },
        ) => am.cmp(bm),
        _ => std::cmp::Ordering::Equal,
    });

    Ok(bindings)
}

/// Walk every plist under `prefs_dir` and aggregate menu-item overrides.
///
/// Plists that fail to read or parse are skipped (most files in the
/// preferences directory are unrelated app preferences). Only an error
/// reading the directory itself is propagated.
fn scan(prefs_dir: &Path) -> Result<Vec<Binding>, ScanError> {
    let entries = std::fs::read_dir(prefs_dir).map_err(|source| ScanError::Io {
        path: prefs_dir.to_path_buf(),
        source,
    })?;

    let mut bindings = Vec::new();
    let mut plist_count = 0usize;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("plist") {
            continue;
        }
        if plist_count == MAX_PREFERENCE_PLISTS {
            break;
        }
        plist_count += 1;
        if let Ok(found) = parse(&path) {
            let remaining = MAX_TOTAL_OVERRIDE_BINDINGS.saturating_sub(bindings.len());
            bindings.extend(found.into_iter().take(remaining));
            if bindings.len() == MAX_TOTAL_OVERRIDE_BINDINGS {
                break;
            }
        }
    }

    Ok(bindings)
}

fn bundle_id_from_path(path: &Path) -> Option<String> {
    path.file_stem().and_then(|s| s.to_str()).map(String::from)
}

/// Parse Apple's `NSUserKeyEquivalents` keystroke shorthand into a `KeyCombo`.
///
/// Modifier prefix characters in any order: `@` (cmd), `~` (opt), `$` (shift),
/// `^` (ctrl). One trailing character names the key. Returns `None` for
/// malformed input (no key, multiple key chars, or unrecognized prefixes).
fn parse_keystroke(s: &str) -> Option<KeyCombo> {
    let mut modifiers = Modifiers::empty();
    let mut chars = s.chars().peekable();

    loop {
        match chars.peek()? {
            '@' => modifiers |= Modifiers::CMD,
            '~' => modifiers |= Modifiers::OPT,
            '$' => modifiers |= Modifiers::SHIFT,
            '^' => modifiers |= Modifiers::CTRL,
            _ => break,
        }
        chars.next();
    }

    let key_char = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let key = Key::from_char(key_char);
    if matches!(key, Key::Char(ch) if ch.is_control()) {
        return None;
    }

    Some(KeyCombo { modifiers, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keystroke_cmd_only() {
        let c = parse_keystroke("@n").unwrap();
        assert_eq!(
            c,
            KeyCombo {
                modifiers: Modifiers::CMD,
                key: Key::Char('n'),
            }
        );
    }

    #[test]
    fn parse_keystroke_cmd_opt() {
        let c = parse_keystroke("@~n").unwrap();
        assert_eq!(
            c,
            KeyCombo {
                modifiers: Modifiers::CMD | Modifiers::OPT,
                key: Key::Char('n'),
            }
        );
    }

    #[test]
    fn parse_keystroke_all_modifiers() {
        let c = parse_keystroke("@~$^x").unwrap();
        assert_eq!(
            c,
            KeyCombo {
                modifiers: Modifiers::CMD | Modifiers::OPT | Modifiers::SHIFT | Modifiers::CTRL,
                key: Key::Char('x'),
            }
        );
    }

    #[test]
    fn parse_keystroke_modifier_order_is_irrelevant() {
        assert_eq!(parse_keystroke("@$s"), parse_keystroke("$@s"));
    }

    #[test]
    fn parse_keystroke_no_key_returns_none() {
        assert_eq!(parse_keystroke("@~"), None);
    }

    #[test]
    fn parse_keystroke_extra_chars_returns_none() {
        assert_eq!(parse_keystroke("@nx"), None);
    }

    #[test]
    fn parse_keystroke_empty_returns_none() {
        assert_eq!(parse_keystroke(""), None);
    }

    #[test]
    fn parse_keystroke_rejects_unrecognized_control_character() {
        assert_eq!(parse_keystroke("\u{009b}"), None);
    }

    #[test]
    fn parse_keystroke_no_modifiers_just_key() {
        let c = parse_keystroke("a").unwrap();
        assert_eq!(
            c,
            KeyCombo {
                modifiers: Modifiers::empty(),
                key: Key::Char('a'),
            }
        );
    }
}
