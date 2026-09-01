//! Per-app menu shortcut enumeration via the macOS Accessibility API.
//!
//! Walks every running app's main menu bar and status items using
//! `AXUIElement` queries, extracting any menu item with a key equivalent.
//! Each match becomes a `Binding` with `BindingSource::AppMenuItem`.
//!
//! Requires the host process to have Accessibility permission
//! (System Settings → Privacy & Security → Accessibility). The first call
//! that exercises an `AX*` API triggers macOS's permission prompt.

#[cfg(target_os = "macos")]
mod macos;

use crate::{Binding, ScanError};

use super::Source;

/// Source backed by the macOS Accessibility API.
#[derive(Debug, Clone)]
pub struct Accessibility {
    /// Per-app `AXUIElementSetMessagingTimeout`, in seconds. Values `<= 0`
    /// leave the system default in place. Tight values (e.g. 1.0) prevent a
    /// single non-responsive app from stalling the whole scan. Positive
    /// values are capped at 60 seconds by the macOS implementation.
    pub messaging_timeout: f32,
    /// Defensive cap on the number of apps walked in parallel. `0` uses
    /// the implementation's hard cap of 128. Default 128 keeps typical
    /// sessions in one batch while bounding thread creation for larger ones.
    pub max_concurrency: usize,
    /// Optional case-insensitive substring filter on bundle id / app name.
    /// Empty = scan every running app. Non-empty = retain only apps whose
    /// `bundle_id` or `app_name` contains at least one of these substrings
    /// (OR semantics). Pushed down before walking menus, so scanning
    /// "rustrover" out of 80 running apps walks just one.
    pub bundle_filter: Vec<String>,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            messaging_timeout: 1.0,
            max_concurrency: 128,
            bundle_filter: Vec::new(),
        }
    }
}

impl Accessibility {
    pub fn new(messaging_timeout: f32, max_concurrency: usize) -> Self {
        Self {
            messaging_timeout,
            max_concurrency,
            bundle_filter: Vec::new(),
        }
    }

    /// Restrict the scan to apps whose `bundle_id` or `app_name`
    /// case-insensitively contains any of `filter`. Empty `filter` clears
    /// the restriction.
    pub fn with_bundle_filter(mut self, filter: Vec<String>) -> Self {
        self.bundle_filter = filter;
        self
    }
}

impl Source for Accessibility {
    fn name(&self) -> &'static str {
        "accessibility"
    }

    fn scan(&self) -> Result<Vec<Binding>, ScanError> {
        #[cfg(target_os = "macos")]
        {
            macos::scan(
                self.messaging_timeout,
                self.max_concurrency,
                &self.bundle_filter,
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok(Vec::new())
        }
    }
}

/// Whether the host process currently has Accessibility permission.
///
/// On non-macOS platforms always returns `true` (no permission concept).
pub fn is_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::is_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
