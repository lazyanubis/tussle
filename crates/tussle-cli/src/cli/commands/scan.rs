//! `tussle scan` — list every binding every source can see.

use anyhow::{Context, Result};
use tabled::builder::Builder;
use tabled::settings::Style;
use tussle_core::{Binding, ComboToken};

use crate::cli::GroupBy;
use crate::cli::output::{emit_json, escape_terminal_text};
use crate::cli::sources::{default_sources, warn_if_no_accessibility};

pub fn scan(
    as_json: bool,
    ax_timeout: f32,
    ax_concurrency: usize,
    key_filter: Vec<String>,
    app_filter: Vec<String>,
    group_by: GroupBy,
) -> Result<()> {
    let started = std::time::Instant::now();

    // Parse `--key` tokens up front so we fail fast on a typo rather than
    // after a 1-second scan.
    let key_matchers: Vec<ComboToken> = key_filter
        .iter()
        .map(|s| ComboToken::parse(s).with_context(|| format!("parsing --key {s:?}")))
        .collect::<Result<Vec<_>>>()?;

    let sources = default_sources(ax_timeout, ax_concurrency, app_filter.clone())?;
    warn_if_no_accessibility();

    let mut bindings: Vec<Binding> = Vec::new();
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
                bindings.extend(found);
            }
            Err(e) => {
                let error = escape_terminal_text(&e.to_string());
                tracing::warn!(source = src.name(), error = %error, "source failed");
            }
        }
    }

    // App filter: also enforced at the CLI side so non-Accessibility sources
    // (SymbolicHotkeys, NSUserKeyEquivalents) honor it. Accessibility has
    // already pruned unmatched apps before walking, so this is a no-op for
    // its rows — it's here for correctness, not perf.
    //
    // Match against both `owner()` (display name, often localized — e.g.
    // "访达" on Chinese macOS) and `bundle_id()` (reverse-DNS id, always
    // English — e.g. "com.apple.finder"). Without the bundle-id leg,
    // `--app finder` would silently miss every app with a localized name
    // even though the underlying scan layer already accepts it.
    if !app_filter.is_empty() {
        let filter_lc: Vec<String> = app_filter.iter().map(|s| s.to_lowercase()).collect();
        bindings.retain(|b| {
            let owner_lc = b.source.owner().to_lowercase();
            let bundle_lc = b.source.bundle_id().map(str::to_lowercase);
            filter_lc.iter().any(|f| {
                owner_lc.contains(f) || bundle_lc.as_deref().is_some_and(|s| s.contains(f))
            })
        });
    }

    if !key_matchers.is_empty() {
        bindings.retain(|b| key_matchers.iter().any(|m| m.matches(&b.combo)));
    }

    // Stable sort by the chosen group key; tie-break by the other axis so
    // each group is internally ordered too. Lexicographic on the rendered
    // combo is enough for "same combo lands together" — exact ordering
    // across groups isn't load-bearing.
    match group_by {
        GroupBy::Combo => {
            bindings.sort_by(|a, b| {
                format!("{}", a.combo)
                    .cmp(&format!("{}", b.combo))
                    .then_with(|| a.source.owner().cmp(b.source.owner()))
            });
        }
        GroupBy::Owner => {
            bindings.sort_by(|a, b| {
                a.source
                    .owner()
                    .cmp(b.source.owner())
                    .then_with(|| format!("{}", a.combo).cmp(&format!("{}", b.combo)))
            });
        }
    }

    tracing::info!(
        bindings = bindings.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "scan complete",
    );

    if as_json {
        return emit_json(&bindings);
    }

    if bindings.is_empty() {
        println!("(no bindings found)");
        return Ok(());
    }

    let mut builder = Builder::default();
    builder.push_record(["Combo", "Owner", "Action"]);
    for b in &bindings {
        builder.push_record([
            escape_terminal_text(&b.combo.to_string()),
            escape_terminal_text(b.source.owner()),
            escape_terminal_text(&b.label),
        ]);
    }
    // Blank line so the table doesn't visually butt up against any
    // preceding stderr log lines when both share the same TTY.
    println!();
    println!("{}", builder.build().with(Style::psql()));
    Ok(())
}
