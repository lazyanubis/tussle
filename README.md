# tussle

**Inspect every keyboard shortcut on your Mac.**

Find which app uses a shortcut, list every binding registered on your
machine, and spot conflicts between apps.

```text
$ tussle who ctrl+1

 Owner  | Action
--------+----------------------------------
 macOS  | Switch to Desktop 1
 PixPin | ScreenShot
 Warp   | Left Panel: Agent Conversations
```

Three different things claim ⌃1 here. macOS dispatches to whichever
registered first; the others stay invisible. Knowing the full set
explains why a shortcut sometimes "doesn't work" and shows what to
disable to free a combo.

## Install

No prebuilt binaries yet. Build from source:

```bash
git clone https://github.com/defi-failure/tussle.git
cd tussle
cargo install --path crates/tussle-cli
```

Requires macOS and Rust 1.97+ (edition 2024). Uninstall with
`cargo uninstall tussle-cli`.

## Usage

### List every shortcut

```bash
tussle scan
```

A typical Mac has 1000–3000 bindings. The default ordering groups by
combo, so any shortcut claimed by more than one owner stacks
contiguously.

### Look up a single combo

```bash
tussle who cmd+w
tussle who 'shift+cmd+5'
```

Modifiers: `cmd`, `opt`, `ctrl`, `shift`, `fn` (case-insensitive,
aliases like `command` / `option` / `alt` / `control` / `globe` work).

Or interactive — omit the argument and press the actual key:

```bash
tussle who
# Press the hotkey to look up...
```

### Filter & group

```bash
tussle scan --app rustrover            # only RustRover bindings
tussle scan --key cmd                  # any shortcut containing cmd
tussle scan --key space --key f1       # space OR f1 shortcuts
tussle scan --group-by owner           # group by app instead of combo
tussle scan --json                     # JSON for piping to jq
```

`--app` matches both display name and bundle id, so `--app finder`
works on Chinese-localized macOS where Finder shows as `访达`.
Multiple `--app` or `--key` values combine as OR; different flags
combine as AND.

## Permissions

macOS prompts the first time each is needed:

- **Accessibility** — to read each running app's menu shortcuts.
- **Input Monitoring** — only for `tussle who` interactive capture.

Grant from `System Settings → Privacy & Security`, then re-run.

## Status

Early. macOS only.

## TODO

- Pre-built binaries on GitHub Releases.
- Third-party launcher parsers: Karabiner, Raycast, BetterTouchTool,
  Hammerspoon, Keyboard Maestro.
- Persistent cache for instant repeat lookups.
- `tussle diff` — what changed since the last scan.
- Homebrew tap.
- GUI (likely a SwiftUI menu-bar app?).

## License

MIT — see [LICENSE](LICENSE).
