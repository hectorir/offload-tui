# offload

**Fast, verified camera-card offloading with a terminal UI.**

Plug in a card, pick your camera, press <kbd>enter</kbd>. `offload` copies the right files into a new
date-stamped folder on your backup drive, re-reads every file from disk, and checks its checksum before
calling it done. Your card is never touched.

It ships with the author's setup (DJI Osmo Action and a Sony mirrorless camera) as the built-in
**`hectorir` config**, and everything about it is configurable: add your own cameras, folders, file types
and themes without writing code.

```
 ╔═╗╔═╗╔═╗╦  ╔═╗╔═╗╔╦╗                                                             theme catppuccin
 ║ ║╠╣ ╠╣ ║  ║ ║╠═╣ ║║                    using built-in 'hectorir' profiles · press e to customize
 ╚═╝╚  ╚  ╩═╝╚═╝╩ ╩═╩╝
 verified camera offload

 ┏ 1 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
 ┃             DJI Osmo Action   ● ready                                                          ┃
 ┃ ┌─────┐▪    ● DJI_001   ● DCIM                                                                 ┃
 ┃ │  ◉  │     copies MP4 WAV   never: LRF                                                        ┃
 ┃ └─────┘     into   <date>/DJI/                                                                 ┃
 ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 ╭ 2 ─────────────────────────────────────────────────────────────────────────────────────────────╮
 │    ▄▄▄      Sony Mirrorless   ● ready                                                          │
 │ ╭───────╮   ● DCIM   ● CLIP                                                                    │
 │ │  (◉)  │   copies MP4 ARW                                                                     │
 │ ╰───────╯   into   <date>/Sony/                                                                │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯


 ╭ Backup destination ────────────────────────────────────────────────────────────────────────────╮
 │● Backup  ███████████████████████████████████▏░░░░  59.7 GB free of 494.4 GB                    │
 │/tmp/offload-demo/Backup                                                                        │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ choose  enter start  e settings  s stats  t theme  ? help  q quit
```

## Why

Offloading footage is the one step in a shoot where a silent failure costs you the shot. Most copy tools
tell you the copy *finished*; `offload` tells you the copy is *correct*.

- **Verified, not hoped-for.** Each file is copied to a hidden `.part` file, flushed to disk, re-read with
  the OS cache bypassed, and compared by BLAKE3 hash against the bytes read from the card. Only then is it
  renamed to its real name. A truncated or corrupt file can never carry a real name.
- **Never destructive.** Sources are opened read-only. Nothing is ever deleted from your card, and nothing
  at the destination is ever overwritten. If a *different* file already has that name, both are kept
  (`C0001.MP4`, `C0001_1.MP4`).
- **Safe to re-run.** Interrupted? Run it again. Files already on the destination with a matching checksum
  are skipped; half-written `.part` files are replaced.
- **Fast.** A reader thread, a writer thread and a verifier thread run as a pipeline, so reading the card,
  writing the backup and verifying the previous file all overlap.
- **Looks good.** Gradient progress bars, a live throughput sparkline, storage meters, five themes, mouse support.

## Install

You need a [Rust toolchain](https://rustup.rs) (or `brew install rust`).

```sh
cargo install --git https://github.com/hectorir/offload-tui
```

or from a clone:

```sh
git clone https://github.com/hectorir/offload-tui && cd offload-tui
cargo install --path .
```

Then run `offload`. Tested on macOS; the engine uses standard POSIX APIs and CI also builds on Linux.
Eject and notifications use `diskutil`/`osascript` on macOS (`notify-send` on Linux); everything else is portable.

## Using it

Run `offload` with no arguments. The home screen shows each camera profile with a live indicator of
whether its card is plugged in (it updates once a second, so you can plug in after launching).

1. **Choose a camera** with <kbd>↑</kbd><kbd>↓</kbd> and <kbd>enter</kbd> (or press <kbd>1</kbd>-<kbd>9</kbd>, or click).
2. **Review** what was found. Everything is selected by default.
   <kbd>space</kbd> toggles a file, <kbd>tab</kbd> jumps to the *shoot dates* list so you can include or
   exclude whole days, <kbd>a</kbd> selects all/none. The free-space check happens here, before any byte
   is written.
3. **Copy.** <kbd>enter</kbd> starts. <kbd>p</kbd> pauses, <kbd>esc</kbd> cancels (partial files are removed).
4. **Done.** A summary shows what was copied, skipped, renamed or failed. <kbd>o</kbd> opens the folder,
   <kbd>x</kbd> ejects the card, and a desktop notification fires if you walked away.

```
 ╭ DJI Osmo Action · 10 of 10 selected ────────────────────────────╮╭ Ready to copy ────────────────────────────────╮
 │[x] DJI_20261005_0001_D.WAV            1.5 MB  Oct 08 10:36      ││10 files  ·  595.5 MB                          │
 │[x] DJI_20261005_0001_D.MP4           30.0 MB  Oct 11 10:36      ││to write  595.5 MB   0 already on dest         │
 │[x] DJI_20261005_0002_D.MP4           39.0 MB  Oct 11 10:36      ││into      /tmp/offload…mo/Backup/2026-10-11/DJI│
 │[x] DJI_20261005_0003_D.MP4           48.0 MB  Oct 11 10:36      ││● backup  █████▎  59.7 GB free of 494.4 GB     │
 │[x] DJI_20261005_0004_D.MP4           57.0 MB  Oct 11 10:36      ││press enter to copy + verify                   │
 │[x] DJI_20261005_0005_D.MP4           66.0 MB  Oct 11 10:36      ││                                               │
 │[x] DJI_20261006_0006_D.MP4           75.0 MB  Oct 11 10:36      │╰───────────────────────────────────────────────╯
 │[x] DJI_20261006_0007_D.MP4           84.0 MB  Oct 11 10:36      │╭ Sources ──────────────────────────────────────╮
 │[x] DJI_20261006_0008_D.MP4           93.0 MB  Oct 11 10:36      ││● /tmp/offloa…mo/SD_Card/DCIM/DJI_001  10 files│
 │[x] DJI_20261006_0009_D.MP4          102.0 MB  Oct 11 10:36      ││● /tmp/offload-demo/OsmoAction/DCIM  0 files   │
 │                                                                 │╰───────────────────────────────────────────────╯
 │                                                                 │╭ By type ──────────────────────────────────────╮
 │                                                                 ││MP4     9   594.0 MB  █████████████████████    │
 │                                                                 ││WAV     1     1.5 MB                           │
 │                                                                 │╰───────────────────────────────────────────────╯
 │                                                                 │╭ Shoot dates  (tab) ───────────────────────────╮
 │                                                                 ││[x] Thu 08 Oct 2026    1 files     1.5 MB      │
 │                                                                 ││[x] Sun 11 Oct 2026    9 files   594.0 MB      │
 │                                                                 ││                                               │
 │                                                                 ││                                               │
 ╰─────────────────────────────────────────────────────────────────╯│                                               │
 ╭ Details ────────────────────────────────────────────────────────╮│                                               │
 │/tmp/offload-demo/SD_Card/DCIM/DJI_001/DJI_20261005_0001_D.WAV   ││                                               │
 │WAV · 1.5 MB · Thu Oct 08 2026, 10:36:02                         ││                                               │
 │from DJI_001                                                     ││                                               │
 ╰─────────────────────────────────────────────────────────────────╯╰───────────────────────────────────────────────╯
space toggle  a all/none  tab dates  enter copy  esc back  ? help
```

```
 ╭ ⠧ COPYING  DJI Osmo Action ──────────────────────────────────────────────────────────────────────────────────────╮
 │copy   █████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  19%  112.4 MB / 595.5 MB           │
 │verify ████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  12%  70.5 MB / 595.5 MB            │
 │files  3/10   speed 78.7 MB/s   eta 6s   elapsed 1s                                                               │
 ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 ╭ Throughput ─────────────────────────────────────────────────────────────────╮╭ Backup drive ─────────────────────╮
 │▇█▇▆▇                                                                        ││██████████████████████████████▊░░░░│
 │█████                                                                        ││59.6 GB free                       │
 │█████                                                                        ││of 494.4 GB                        │
 ╰─────────────────────────────────────────────────────────────────────────────╯╰───────────────────────────────────╯
 ╭ Files  → /tmp/offload-demo/Backup/2026-10-11/DJI ────────────────────────────────────────────────────────────────╮
 │✓ DJI_20261005_0001_D.WAV           1.5 MB  verified                                                              │
 │✓ DJI_20261005_0001_D.MP4          30.0 MB  verified                                                              │
 │✓ DJI_20261005_0002_D.MP4          39.0 MB  verified                                                              │
 │⠧ DJI_20261005_0003_D.MP4          48.0 MB  ██████████▌░ copying  87%                                             │
 │· DJI_20261005_0004_D.MP4          57.0 MB  queued                                                                │
 │· DJI_20261005_0005_D.MP4          66.0 MB  queued                                                                │
 │· DJI_20261006_0006_D.MP4          75.0 MB  queued                                                                │
 │· DJI_20261006_0007_D.MP4          84.0 MB  queued                                                                │
 │· DJI_20261006_0008_D.MP4          93.0 MB  queued                                                                │
 │· DJI_20261006_0009_D.MP4         102.0 MB  queued                                                                │
 ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
p pause  ↑↓ scroll  f follow  esc cancel
```

### Where files go

```
<destination>/<date>/<subfolder>/<files>

/Volumes/Old External Drives/0 Photos & My Footage/Needs to Back up/2026-10-11/DJI/DJI_20261005_0001_D.MP4
                                                                    └─ date ──┘ └ camera ┘
```

Both the date format and the per-camera subfolder are configurable. The folder is created only if it does
not already exist, so running twice on the same day adds to the same folder.

## Configuration

Config lives at `~/.config/offload/config.toml` (or `$XDG_CONFIG_HOME/offload/`; override with `--config`).
With no config file, the built-in `hectorir` profiles are used. Write them out as a starting point:

```sh
offload init          # writes the hectorir config so you can edit it
offload config-path   # shows where it lives
```

Or edit settings inside the app with <kbd>e</kbd>. Changes are validated and saved instantly (a `.bak` of
the previous file is kept).

```toml
# Where backups go. A dated folder is created inside.
destination = "/Volumes/Old External Drives/0 Photos & My Footage/Needs to Back up"
date_format = "%Y-%m-%d"        # strftime; must produce a single folder name
theme       = "catppuccin"      # catppuccin | nord | gruvbox | dracula | mono
notify      = true              # desktop notification when a copy finishes

[[profile]]
name      = "DJI Osmo Action"
subfolder = "DJI"               # "" = put files directly in the dated folder
sources   = ["/Volumes/SD_Card/DCIM/DJI_001", "/Volumes/OsmoAction/DCIM"]
include   = ["mp4", "wav"]      # case-insensitive, no dots
exclude   = ["lrf"]             # never copied, even if also in `include`
icon      = "action"            # action | mirrorless | camera
# destination = "/other/drive"  # optional per-profile override

[[profile]]
name      = "Sony Mirrorless"
subfolder = "Sony"
sources   = ["/Volumes/Untitled/DCIM", "/Volumes/Untitled/private/M4ROOT/CLIP"]
include   = ["mp4", "arw"]
icon      = "mirrorless"
```

Notes:

- Sources are scanned **recursively**. A source that isn't mounted is skipped with a warning; the rest still copy.
- Symlinks and macOS `._*` / `.DS_Store` files are ignored.
- `subfolder` and `date_format` are validated: anything that could escape the dated folder (`..`, `/`) is rejected.
- Add as many `[[profile]]` blocks as you have cameras: GoPro, Insta360, a drone, an audio recorder.
- Theme choice from <kbd>t</kbd> is remembered separately in `~/.local/share/offload/state.json`.

### Sharing camera setups: profile packs

A pack is a single TOML file describing one camera (never your drive paths):

```sh
offload pack export "DJI Osmo Action" -o osmo.toml
offload pack import osmo.toml         # validated, then added to your config
```

If you set up a camera that others use, open a PR adding it to [`examples/`](examples).

## Command line

```
offload                          open the TUI
offload run <profile> [--yes] [--dry-run]
                                 copy without the UI (for scripts); partial names work: `offload run dji`
offload profiles                 list profiles and whether their cards are connected
offload init [--force]           write the built-in config to disk
offload pack export|import       share profiles
offload --config FILE ...        use a different config file
```

`offload run dji --dry-run` lists exactly what would be copied and writes nothing.
Exit codes: `0` success, `1` some files failed/cancelled/aborted, `2` error (bad config, drive not mounted, no space), `3` no source connected.

## Keys

| Where | Key | Action |
| --- | --- | --- |
| Home | `↑↓` `j k` / `1-9` / click | choose profile |
| | `enter` | scan the card |
| | `e` `s` `t` | settings · stats · cycle theme |
| Review | `space` | toggle file / whole day |
| | `tab` | switch files ⇄ shoot dates |
| | `a` | select all / none |
| | `enter` | start (blocked if no space or drive missing) |
| Copying | `p` | pause / resume |
| | `esc` | cancel (asks first) |
| Summary | `o` `x` | open folder · eject card (asks first) |
| Anywhere | `?` | help |
| | `ctrl+c` | quit (cleans up partial files) |

The mouse works too: click profiles, files, dates and settings rows; scroll lists with the wheel.

## How verification works

```
card ──read──▶ [hash while reading] ──▶ write ──▶ .name.part
                                                     │ fsync (full flush)
                                                     ▼
                          re-read from disk, cache bypassed ──▶ BLAKE3 ══ source hash?
                                                     │ yes
                                                     ▼
                                          rename to name  (+ restore modified time)
```

- The source hash is computed on the exact bytes read from the card, so what's verified is what was copied.
- On macOS the verify read uses `F_NOCACHE` and the flush uses a full fsync, so the check reads the drive,
  not RAM. On other platforms the verify read may be served from the page cache.
- Duplicates are detected by **checksum**, not by name or size. A same-named file with different content is
  never overwritten.
- The destination folder is created through `create_dir_all` only on a mounted volume: if your backup
  drive (`/Volumes/...`) isn't plugged in, `offload` stops instead of silently writing to your internal disk.
- Files are created with `O_EXCL | O_NOFOLLOW`; a pre-planted symlink can't redirect a write.
- File names are sanitized before display so a hostile name can't inject terminal escape codes.
- External commands (`diskutil`, `osascript`, `open`) are spawned with argument vectors, never through a shell.
- No network access of any kind.

### Honest limits

- Verification proves the copy matches what was read from the card; it can't detect a card that returns
  wrong data consistently.
- The "details" panel shows file metadata (size, timestamps, source), not image thumbnails or video
  duration/resolution.
- Skipping an existing file means hashing it, which reads it once; that's the price of trusting checksums over names.
- If the process is killed hard (`kill -9`, power loss), a hidden `.name.part` may be left behind. It is
  never mistaken for a good file and is replaced on the next run.
- Tested on macOS. The Linux build is exercised in CI but less battle-tested.

## Stats

Every copy appends a line to `~/.local/share/offload/history.jsonl`. Press <kbd>s</kbd> for lifetime totals,
a per-month chart, per-camera totals and recent sessions.

## Development

```sh
cargo test                # unit + integration + TUI + CLI tests (all against temp dirs)
cargo clippy --all-targets
cargo fmt
cargo test --test app_flow dump_frames -- --ignored --nocapture   # render every screen as text
```

| Module | Role |
| --- | --- |
| `config` | profiles, validation, the `hectorir` preset, packs |
| `scan` | recursive, filtered, symlink-safe discovery |
| `engine` | copy → verify → finalize pipeline, pause/cancel, dedupe |
| `app` / `ui` / `widgets` / `theme` | state machine, rendering, gradient bars, themes |
| `history` | JSONL log and lifetime stats |
| `headless` | the scriptable `offload run` |

The test suite never touches real volumes; it builds throwaway card/backup trees and asserts on bytes,
including cancel mid-copy, stale partials, symlink attacks, name collisions, unmounted drives and corrupted copies.

## License

MIT
