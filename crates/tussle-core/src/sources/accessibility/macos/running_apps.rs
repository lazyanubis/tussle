//! Enumerate running apps that plausibly own a menu bar.
//!
//! `NSWorkspace.runningApplications` returns every process that has an
//! `NSRunningApplication` proxy — including XPC services, daemons, and
//! helper processes that have no menu bar but still respond to AX queries
//! by stalling until the messaging timeout. Two filters trim them out:
//!
//!   - `activationPolicy == Prohibited` — anything explicitly declared as
//!     "shouldn't have UI" (most XPC services, *PrivateProvider helpers).
//!   - executable path contains `/XPCServices/` — Apple-style XPC bundles
//!     that nonetheless declare `Regular` activation policy so they can
//!     host UI in their own process (notably `WebKit.WebContent` and the
//!     surrounding `WebKit.GPU`/`WebKit.Networking` siblings).

use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};

const MAX_APP_IDENTITY_BYTES: usize = 4096;

pub(super) struct RunningApp {
    pub(super) pid: i32,
    pub(super) bundle_id: Option<String>,
    pub(super) app_name: Option<String>,
}

pub(super) fn list_running_apps() -> Vec<RunningApp> {
    let workspace = NSWorkspace::sharedWorkspace();
    let apps = workspace.runningApplications();
    let mut out = Vec::with_capacity(apps.len());
    let mut skipped_prohibited = 0usize;
    let mut skipped_no_bundle = 0usize;
    let mut skipped_xpc = 0usize;
    for app in apps.iter() {
        if app.activationPolicy() == NSApplicationActivationPolicy::Prohibited {
            skipped_prohibited += 1;
            continue;
        }
        // Skip apps with no resolvable bundle. Real .app/.xpc processes
        // always expose a `bundleURL`; the rare ones that don't are
        // sandbox-locked helpers (notably `com.apple.WebKit.WebContent`,
        // whose `executableURL` is also faked to a relative path under the
        // current working directory). They have no menu bar but happily
        // soak up the AX messaging timeout.
        if app.bundleURL().is_none() {
            skipped_no_bundle += 1;
            continue;
        }
        if let Some(url) = app.executableURL()
            && let Some(path) = url.path()
            && path.to_string().contains("/XPCServices/")
        {
            skipped_xpc += 1;
            continue;
        }
        out.push(RunningApp {
            pid: app.processIdentifier(),
            bundle_id: app.bundleIdentifier().and_then(|s| {
                let value = s.to_string();
                (value.len() <= MAX_APP_IDENTITY_BYTES).then_some(value)
            }),
            app_name: app.localizedName().and_then(|s| {
                let value = s.to_string();
                (value.len() <= MAX_APP_IDENTITY_BYTES).then_some(value)
            }),
        });
    }
    tracing::debug!(
        kept = out.len(),
        skipped_prohibited,
        skipped_no_bundle,
        skipped_xpc,
        "filtered running apps",
    );
    out
}
