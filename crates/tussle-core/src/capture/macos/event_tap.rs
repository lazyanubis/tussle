//! `CGEventTap` lifecycle: install, run the loop, return the captured event.
//!
//! This module is the seam where any future FFI rewrite (e.g. to subscribe
//! to additional event types not exposed by `core_graphics::CGEventType`)
//! will land. Everything `CGEventTap`-specific is contained here; callers
//! see only `Captured` and `Modifiers`.

use std::sync::{Arc, Mutex};

use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};

use crate::capture::Captured;
use crate::{Modifiers, ScanError};

use super::capture_error;
use super::flags::decode_cg_flags;
use super::keydown::build_captured;

/// Install a global event tap, drive the run loop until either a non-modifier
/// KeyDown arrives (returned) or the user hits Ctrl+C (errors out).
///
/// `on_modifiers_changed` is called on every FlagsChanged event so callers
/// can render live "Holding: cmd+shift…" feedback before the final key.
pub(super) fn capture_via_tap<F>(on_modifiers_changed: F) -> Result<Captured, ScanError>
where
    F: Fn(Modifiers) + Send + 'static,
{
    let captured: Arc<Mutex<Option<Captured>>> = Arc::new(Mutex::new(None));
    let captured_for_cb = Arc::clone(&captured);

    let runloop = CFRunLoop::get_current();
    let runloop_for_cb = runloop.clone();

    // Without Input Monitoring permission the event tap silently never
    // fires; CFRunLoopRun would block forever and Ctrl+C wouldn't even
    // reach us (the tap drops it before the terminal sees it). Install
    // a SIGINT handler that explicitly breaks the run loop.
    let runloop_for_signal = runloop.clone();
    let _ = ctrlc::set_handler(move || runloop_for_signal.stop());

    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![CGEventType::KeyDown, CGEventType::FlagsChanged],
        move |_proxy, etype, event| {
            let modifiers = decode_cg_flags(event.get_flags().bits());

            if matches!(etype, CGEventType::FlagsChanged) {
                on_modifiers_changed(modifiers);
                return CallbackResult::Drop;
            }

            // KeyDown of a non-modifier key — finalize and exit.
            let vk = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            let result = build_captured(vk, modifiers);
            if let Ok(mut slot) = captured_for_cb.lock() {
                *slot = Some(result);
            }
            runloop_for_cb.stop();
            CallbackResult::Drop
        },
    )
    .map_err(|_| {
        capture_error("could not install event tap (Input Monitoring permission needed?)")
    })?;

    let source = tap
        .mach_port()
        .create_runloop_source(0)
        .map_err(|_| capture_error("could not create runloop source"))?;

    unsafe {
        runloop.add_source(&source, kCFRunLoopCommonModes);
    }
    tap.enable();

    CFRunLoop::run_current();

    let captured = captured
        .lock()
        .map_err(|_| capture_error("lock poisoned"))?;
    (*captured).ok_or_else(|| capture_error("event tap exited without capturing a key"))
}
