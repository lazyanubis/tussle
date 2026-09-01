//! Per-source hotkey config parsers.
//!
//! Every concrete source (system plist, per-app NSUserKeyEquivalents,
//! Accessibility menu walker, future Karabiner/Raycast/...) implements the
//! [`Source`] trait so callers can iterate over a heterogeneous set of
//! sources without knowing each one's filesystem requirements.

use crate::{Binding, ScanError};

mod plist_file;

pub mod accessibility;
pub mod nsuserkeyequivalents;
pub mod symbolichotkeys;

/// Single-source contract: name yourself and produce your current bindings.
///
/// Each implementor stores whatever configuration it needs (file paths,
/// directory roots, etc.) at construction time; `scan` takes no arguments
/// so callers can stash a `Vec<Box<dyn Source>>` and iterate uniformly.
pub trait Source {
    /// Stable identifier for this source, e.g. `"symbolichotkeys"`. Used for
    /// filtering on the CLI (`--source ...`) and tagging errors.
    fn name(&self) -> &'static str;

    /// Walk this source and produce its current set of bindings.
    fn scan(&self) -> Result<Vec<Binding>, ScanError>;
}
