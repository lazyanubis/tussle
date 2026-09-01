//! `tussle who` — for a given combo (parsed or interactively captured),
//! list every source that owns it.

use anyhow::{Context, Result};
use tabled::builder::Builder;
use tabled::settings::Style;
use tussle_core::capture::{self, Captured};
use tussle_core::{Binding, KeyCombo};

use crate::cli::output::{emit_json, escape_terminal_text};
use crate::cli::sources::{default_sources, warn_if_no_accessibility};

pub fn who(
    combo_arg: Option<String>,
    as_json: bool,
    ax_timeout: f32,
    ax_concurrency: usize,
) -> Result<()> {
    let combo = match combo_arg {
        Some(text) => KeyCombo::parse(&text).with_context(|| format!("parsing combo {text:?}"))?,
        None => match capture_interactively()? {
            Some(c) => c,
            None => return Ok(()),
        },
    };

    let sources = default_sources(ax_timeout, ax_concurrency, Vec::new())?;
    warn_if_no_accessibility();

    let mut matches: Vec<Binding> = Vec::new();
    for src in &sources {
        let t_src = std::time::Instant::now();
        match src.scan() {
            Ok(found) => {
                tracing::info!(
                    source = src.name(),
                    bindings = found.len(),
                    elapsed_ms = t_src.elapsed().as_millis() as u64,
                    "source scan complete",
                );
                matches.extend(found.into_iter().filter(|b| b.combo == combo));
            }
            Err(e) => {
                let error = escape_terminal_text(&e.to_string());
                tracing::warn!(source = src.name(), error = %error, "source failed");
            }
        }
    }
    let combo_display = escape_terminal_text(&combo.to_string());
    tracing::info!(
        combo = %combo_display,
        matches = matches.len(),
        "lookup complete",
    );

    if as_json {
        return emit_json(&matches);
    }

    if matches.is_empty() {
        println!("nothing bound to {}", combo_display);
        return Ok(());
    }

    let mut builder = Builder::default();
    builder.push_record(["Owner", "Action"]);
    for b in &matches {
        builder.push_record([
            escape_terminal_text(b.source.owner()),
            escape_terminal_text(&b.label),
        ]);
    }
    println!();
    println!("{}", builder.build().with(Style::psql()));
    Ok(())
}

/// Run the interactive capture flow. Returns:
///   - `Ok(Some(combo))` for a normal hotkey to look up,
///   - `Ok(None)` when the user pressed a macOS system action — we already
///     printed the explanation and the caller should bail out cleanly,
///   - `Err(_)` on capture failure (no Input Monitoring permission, etc.).
fn capture_interactively() -> Result<Option<KeyCombo>> {
    eprintln!("Press the hotkey to look up (Ctrl+C to abort)...");
    let captured = capture::capture_one(|mods| {
        use std::io::Write;
        let mut stderr = std::io::stderr().lock();
        // \x1B[2K clears the entire line; \r returns the cursor.
        if mods.is_empty() {
            let _ = write!(stderr, "\r\x1B[2K");
        } else {
            let _ = write!(stderr, "\r\x1B[2KHolding: {mods}+");
        }
        let _ = stderr.flush();
    })
    .context("capturing keystroke")?;

    Ok(match captured {
        Captured::Combo(c) => {
            eprintln!("\r\x1B[2KCaptured: {c} — looking up...");
            Some(c)
        }
        Captured::SystemAction(action) => {
            eprintln!(
                "\r\x1B[2KCaptured: vk 0x{:02x} — '{}'.",
                action.vk,
                action.kind.name(),
            );
            eprintln!(
                "This is a macOS system action: dispatched by macOS itself, \
                 not an app-bindable hotkey. Apple does not document the \
                 0x80+ virtual-keycode range (kVK_* tops out at 0x7E)."
            );
            if let Some(hint) = action.kind.source_hint() {
                eprintln!("To change it: {hint}.");
            }
            None
        }
    })
}
