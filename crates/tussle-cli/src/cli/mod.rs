//! Command-line interface: argument parsing and command dispatch.

mod commands;
mod output;
mod sources;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

/// How `tussle scan` orders the rows it prints. Sort key is also the
/// "group" — equal values land next to each other, so e.g. `Combo` makes
/// any combo bound by more than one source visible at a glance.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum GroupBy {
    /// Group by combo string. Same combo across multiple owners stacks
    /// contiguously — the natural "is anything conflicting?" view.
    Combo,
    /// Group by owner. Each app's bindings appear together — the natural
    /// "what does X have?" view.
    Owner,
}

#[derive(Parser)]
#[command(name = "tussle", version, about = "macOS hotkey conflict resolver")]
struct Cli {
    /// Per-app Accessibility messaging timeout, in seconds. Caps how long
    /// a single non-responsive app can stall the scan. Set to `0` to use
    /// the macOS default (~6s). Positive values are capped at 60 seconds.
    #[arg(long, global = true, default_value_t = 1.0, value_name = "SECS")]
    ax_timeout: f32,

    /// Defensive cap on the number of apps walked in parallel. `0` uses
    /// the built-in hard cap of 128. Larger values are also capped at 128.
    /// Default 128 keeps typical 50–100 app sessions in one batch while
    /// larger sessions are processed in bounded batches.
    #[arg(long, global = true, default_value_t = 128, value_name = "N")]
    ax_concurrency: usize,

    /// Increase log verbosity. `-v` INFO (high-level progress), `-vv` DEBUG
    /// (per-app timing, filter decisions), `-vvv` TRACE (per-AX-call detail
    /// — only useful when diagnosing a specific slow IPC). Overridden by
    /// `RUST_LOG` if set.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan all hotkey sources and print discovered bindings.
    Scan {
        /// Emit JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,

        /// Keep only bindings whose combo contains this token. A token is
        /// either a modifier (`cmd`/`command`/`opt`/`alt`/`ctrl`/`shift`/
        /// `fn`/`globe`) or a key (`space`, `f1`, `a`, …); matching is
        /// case-insensitive. Repeat for OR semantics. Combined with
        /// `--app` via AND.
        #[arg(long, value_name = "TOKEN", action = ArgAction::Append)]
        key: Vec<String>,

        /// Keep only bindings owned by an app whose bundle id or display
        /// name contains this substring (case-insensitive). Repeat for OR
        /// semantics. Pushed down into the Accessibility scan so unmatched
        /// apps are skipped entirely — `--app rustrover` is much faster
        /// than scanning everything and filtering afterward.
        #[arg(long, value_name = "NAME", action = ArgAction::Append)]
        app: Vec<String>,

        /// Sort/group the output. Default `combo` stacks every owner of
        /// the same combo together (good for spotting conflicts).
        #[arg(long, value_enum, value_name = "KEY", default_value_t = GroupBy::Combo)]
        group_by: GroupBy,
    },
    /// Look up which sources own a key combination.
    Who {
        /// Combo to look up, e.g. `cmd+opt+b`. Omit to enter interactive
        /// capture mode.
        combo: Option<String>,

        /// Emit JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
}

/// Parse argv and dispatch to the chosen subcommand.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match cli.command {
        Command::Scan {
            json,
            key,
            app,
            group_by,
        } => commands::scan::scan(json, cli.ax_timeout, cli.ax_concurrency, key, app, group_by),
        Command::Who { combo, json } => {
            commands::who::who(combo, json, cli.ax_timeout, cli.ax_concurrency)
        }
    }
}

/// Initialize a tracing subscriber that writes to stderr. `RUST_LOG` always
/// wins; without it, `verbosity` selects the default level for our own
/// crates (`tussle_core` and `tussle_cli`):
///   - `0` → WARN (only real problems)
///   - `1` → INFO (high-level progress)
///   - `2` → DEBUG (per-app timings, filter decisions)
///   - `≥3` → TRACE (per-AX-call detail)
fn init_tracing(verbosity: u8) {
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // The binary's crate name is `tussle` (set via `[[bin]] name = "tussle"`
    // in tussle-cli's Cargo.toml), not `tussle_cli` — so events emitted from
    // cli/* report a `tussle::*` target. Filter both that and the core lib.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "tussle={lvl},tussle_core={lvl}",
            lvl = default_level
        ))
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .compact()
        .try_init();
}
