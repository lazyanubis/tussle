//! Serializable shapes for the `--json` output mode and the helper that
//! prints them. Table rendering stays inside each command so column choices
//! live next to the command that uses them.

use anyhow::Result;
use serde::Serialize;
use tussle_core::{Binding, BindingSource};

/// Render strings obtained from apps or preference files without allowing
/// terminal control sequences to reach a human-readable output stream.
/// JSON output uses a separate semantics-preserving escape pass below.
pub(super) fn escape_terminal_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_ascii_control() => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\x{:02x}", ch as u32);
            }
            ch if ch.is_control() || is_bidi_control(ch) => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\u{{{:x}}}", ch as u32);
            }
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// serde_json escapes the C0 range required by JSON, but JSON permits C1 and
/// bidirectional formatting controls as raw Unicode. Re-encode those scalars
/// as JSON `\uXXXX` escapes so printing `--json` to a terminal is safe while
/// parsed JSON values remain unchanged.
fn escape_json_terminal_controls(json: &str) -> String {
    let mut escaped = String::with_capacity(json.len());
    for ch in json.chars() {
        if (!ch.is_ascii() && ch.is_control()) || is_bidi_control(ch) {
            use std::fmt::Write;
            let _ = write!(escaped, "\\u{:04x}", ch as u32);
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

#[derive(Serialize)]
pub(super) struct BindingJson<'a> {
    combo: String,
    owner: &'a str,
    action: &'a str,
    source: SourceJson,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SourceJson {
    SystemSymbolicHotkey {
        id: u32,
    },
    AppMenuOverride {
        bundle_id: String,
        menu_item: String,
    },
    AppMenuItem {
        bundle_id: String,
        app_name: Option<String>,
        menu_path: Vec<String>,
    },
}

impl<'a> From<&'a Binding> for BindingJson<'a> {
    fn from(b: &'a Binding) -> Self {
        Self {
            combo: format!("{}", b.combo),
            owner: b.source.owner(),
            action: &b.label,
            source: match &b.source {
                BindingSource::SystemSymbolicHotkey { id } => {
                    SourceJson::SystemSymbolicHotkey { id: *id }
                }
                BindingSource::AppMenuOverride {
                    bundle_id,
                    menu_item,
                } => SourceJson::AppMenuOverride {
                    bundle_id: bundle_id.clone(),
                    menu_item: menu_item.clone(),
                },
                BindingSource::AppMenuItem {
                    bundle_id,
                    app_name,
                    menu_path,
                } => SourceJson::AppMenuItem {
                    bundle_id: bundle_id.clone(),
                    app_name: app_name.clone(),
                    menu_path: menu_path.clone(),
                },
            },
        }
    }
}

/// Print `bindings` as pretty-printed JSON to stdout.
pub(super) fn emit_json(bindings: &[Binding]) -> Result<()> {
    let rows: Vec<BindingJson> = bindings.iter().map(BindingJson::from).collect();
    let json = serde_json::to_string_pretty(&rows)?;
    println!("{}", escape_json_terminal_controls(&json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_text_preserves_normal_unicode() {
        assert_eq!(
            escape_terminal_text("访达 — New Window"),
            "访达 — New Window"
        );
    }

    #[test]
    fn terminal_text_escapes_control_and_bidi_sequences() {
        let malicious = "safe\u{1b}]52;c;payload\u{7}\n\u{009b}31m\u{202e}txt";
        let escaped = escape_terminal_text(malicious);

        assert_eq!(
            escaped,
            "safe\\x1b]52;c;payload\\x07\\n\\u{9b}31m\\u{202e}txt"
        );
        assert!(!escaped.chars().any(char::is_control));
        assert!(!escaped.chars().any(is_bidi_control));
    }

    #[test]
    fn json_escaping_is_terminal_safe_and_semantics_preserving() {
        let original = serde_json::json!({ "value": "\u{009b}31m\u{202e}txt" });
        let json = serde_json::to_string_pretty(&original).expect("JSON should serialize");
        let escaped = escape_json_terminal_controls(&json);

        assert!(escaped.contains("\\u009b"));
        assert!(escaped.contains("\\u202e"));
        assert!(!escaped.contains('\u{009b}'));
        assert!(!escaped.contains('\u{202e}'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&escaped)
                .expect("escaped JSON should remain valid"),
            original
        );
    }
}
