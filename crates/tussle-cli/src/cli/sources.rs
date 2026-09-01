//! Bootstrap the default macOS source set and surface permission caveats.

use anyhow::{Context, Result};
use tussle_core::Source;
use tussle_core::sources::accessibility::{self, Accessibility};
use tussle_core::sources::nsuserkeyequivalents::AppMenuOverrides;
use tussle_core::sources::symbolichotkeys::SymbolicHotkeys;

/// Build the default macOS source set.
///
/// Each source is constructed with paths/configuration the CLI looks up via
/// `dirs`; `tussle-core` itself stays filesystem-agnostic.
pub(super) fn default_sources(
    ax_timeout: f32,
    ax_concurrency: usize,
    app_filter: Vec<String>,
) -> Result<Vec<Box<dyn Source>>> {
    let prefs = dirs::preference_dir().context("could not locate user preferences directory")?;

    Ok(vec![
        Box::new(SymbolicHotkeys::new(
            prefs.join("com.apple.symbolichotkeys.plist"),
        )),
        Box::new(AppMenuOverrides::new(prefs.clone())),
        Box::new(Accessibility::new(ax_timeout, ax_concurrency).with_bundle_filter(app_filter)),
    ])
}

/// Print a one-line stderr note when Accessibility permission is missing,
/// since it silently truncates per-app menu enumeration.
pub(super) fn warn_if_no_accessibility() {
    if !accessibility::is_trusted() {
        eprintln!(
            "note: tussle does not currently have Accessibility permission, \
             so app menu shortcuts will be missing. Grant access in \
             System Settings → Privacy & Security → Accessibility, then re-run."
        );
    }
}
