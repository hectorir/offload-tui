<div align="center">

# offload

**Verified camera-card offloading for the terminal.**<br>
Plug in the card, press <kbd>enter</kbd>, walk away. Every file is copied, re-read from disk and checksum-matched before it counts.

[![CI](https://github.com/hectorir/offload-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/hectorir/offload-tui/actions/workflows/ci.yml)
![license](https://img.shields.io/badge/license-MIT-blue)
![rust](https://img.shields.io/badge/rust-2024-orange)
![platform](https://img.shields.io/badge/macOS%20%C2%B7%20Linux-lightgrey)

</div>

```
 ╔═╗╔═╗╔═╗╦  ╔═╗╔═╗╔╦╗                                                             theme catppuccin
 ║ ║╠╣ ╠╣ ║  ║ ║╠═╣ ║║                    using built-in 'hectorir' profiles · press e to customize
 ╚═╝╚  ╚  ╩═╝╚═╝╩ ╩═╩╝
 verified camera offload

 ┏ 1 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
 ┃   ▁▂▁                                                                                          ┃
 ┃ ┏━━━━━━━━┓   DJI Osmo Action   ● ready                                                         ┃
 ┃ ┃ ╭────╮▫┃   ● DJI_001   ● DCIM                                                                ┃
 ┃ ┃ │(◉) │ ┃   copies MP4 WAV   never: LRF                                                       ┃
 ┃ ┗━╰────╯▬┛   into   <date>/DJI/                                                                ┃
 ┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
 ╭ 2 ─────────────────────────────────────────────────────────────────────────────────────────────╮
 │    ▂▇▇▇▂    ●                                                                                  │
 │ ╭────────╮▗▖   Sony Mirrorless   ● ready                                                       │
 │ │ ╭────╮ │██   ● DCIM   ● CLIP                                                                 │
 │ │ │(◎) │ │██   copies MP4 ARW                                                                  │
 │ ╰─╰────╯─╯▀▀   into   <date>/Sony/                                                             │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯
 ╭ b · batch ─────────────────────────────────────────────────────────────────────────────────────╮
 │  ▁▂▁     ▂▇▇▂                                                                                  │
 │ ┏━━━━┓  ╭────╮▗    Batch · DJI + Sony   ● all ready                                            │
 │ ┃(◉) ┃ +│(◎) │█    ● DJI Osmo Action   ● Sony Mirrorless                                       │
 │ ┗━━━━┛  ╰────╯▀    one review · one copy · every file verified                                 │
 │                    into   <date>/ DJI/ + Sony/                                                 │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯



 ╭ Backup destination ────────────────────────────────────────────────────────────────────────────╮
 │● Backup  ███████████████████████████████████▎░░░░  59.0 GB free of 494.4 GB                    │
 │/tmp/offload-demo/Backup                                                                        │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ choose  enter start  b batch all  e settings  s stats  t theme  ? help  q quit
```

`offload` is a fast, safety-first TUI for getting footage off SD cards and onto your backup drive(s).
It ships with its author's setup (a DJI Osmo Action and a Sony ZV-E1) as the built-in **`hectorir` config**,
and everything is configurable, so you can point it at any camera, folder layout or drive without writing code.

---

## Contents

[Why](#why) · [Install](#install) · [Quick start](#quick-start) · [Features](#features) · [Options](#options-at-a-glance) ·
[Configuration](#configuration) · [Command line](#command-line) · [Keys](#keys) · [How it stays safe](#how-it-stays-safe) ·
[FAQ](#faq) · [Development](#development)

## Why

Offloading is the one step of a shoot where a silent failure costs you the shot. Most copy tools tell you the
copy *finished*. `offload` tells you it is *correct*.

| | |
| --- | --- |
| **Verified, not hoped-for** | Each file goes to a hidden `.part` file, is flushed, re-read from the physical drive with the OS cache bypassed, and compared by BLAKE3 hash with the bytes read from the card. Only then is it renamed. A truncated or corrupt file can never carry a real name. |
| **Fast** | Reading the card, writing the backup and verifying the previous file overlap as a pipeline. An 89 GB two-camera batch to an SMB NAS ran at the network's pace (about 50 MB/s) with every byte verified. |
| **Hard to hurt yourself with** | Sources are read-only. Nothing is ever overwritten. Deleting from the card is opt-in, needs a typed <kbd>y</kbd>, and only ever touches files whose copies were verified. |
| **Safe to re-run** | Interrupted? Run it again. Files already there with a matching checksum are skipped. |
| **Yours** | TOML config, profile packs you can share, five themes, mouse support, notifications, lifetime stats. |

## Install

You need a [Rust toolchain](https://rustup.rs) (`brew install rust` works too).

```sh
cargo install --git https://github.com/hectorir/offload-tui
```

or from a clone:

```sh
git clone https://github.com/hectorir/offload-tui && cd offload-tui
cargo install --path .
```

Run `offload`. Developed and used on macOS; CI builds and tests on macOS and Linux. Eject, notifications,
keep-awake and sleep use `diskutil`/`osascript`/`caffeinate`/`pmset` on macOS (`notify-send`, `systemd-inhibit`, `systemctl` on Linux).

## Quick start

1. Run `offload`. Cards that are plugged in light up green (it checks every second, so you can plug in after launching).
2. Pick a camera with <kbd>↑</kbd><kbd>↓</kbd> + <kbd>enter</kbd> (or <kbd>1</kbd>-<kbd>9</kbd>, or click), or press <kbd>b</kbd> to **batch every connected camera**.
3. **Review** what was found. Everything is selected; <kbd>space</kbd> toggles files, <kbd>tab</kbd> toggles whole shoot days. Free space is checked now, before a byte is written.
4. Press <kbd>enter</kbd>. Watch it copy and verify. <kbd>p</kbd> pauses, <kbd>esc</kbd> cancels (partial files are removed).
5. Read the **summary**. Open the folder (<kbd>o</kbd>), delete the verified originals (<kbd>d</kbd>), or eject the card (<kbd>x</kbd>).

```
 ╭ Batch · DJI + Sony · 15 of 15 selected ────────────────────────────╮╭ Ready to copy ─────────────────────────────────╮
 │[x] DJI   DJI_20261005_0001_D.WAV         1.5 MB  Oct 08 12:09      ││15 files  ·  623.5 MB                           │
 │[x] DJI   DJI_20261005_0001_D.MP4        30.0 MB  Oct 11 12:09      ││to write  623.5 MB   0 already on dest          │
 │[x] DJI   DJI_20261005_0002_D.MP4        39.0 MB  Oct 11 12:09      ││into      /tmp/offload-demo/Backup/2026-10-11   │
 │[x] DJI   DJI_20261005_0003_D.MP4        48.0 MB  Oct 11 12:09      ││● backup  █████▎  59.0 GB free of 494.4 GB      │
 │[x] DJI   DJI_20261005_0004_D.MP4        57.0 MB  Oct 11 12:09      ││folders   by today's date · one per offload (o) │
 │[x] DJI   DJI_20261005_0005_D.MP4        66.0 MB  Oct 11 12:09      ││mode      COPY · originals stay on the card (m) │
 │[x] DJI   DJI_20261006_0006_D.MP4        75.0 MB  Oct 11 12:09      ││press enter to copy + verify                    │
 │[x] DJI   DJI_20261006_0007_D.MP4        84.0 MB  Oct 11 12:09      ││                                                │
 │[x] DJI   DJI_20261006_0008_D.MP4        93.0 MB  Oct 11 12:09      ││                                                │
 │[x] DJI   DJI_20261006_0009_D.MP4       102.0 MB  Oct 11 12:09      ││                                                │
 │[x] Sony  DSC03710.ARW                    2.0 MB  Oct 11 12:09      │╰────────────────────────────────────────────────╯
 │[x] Sony  DSC03711.ARW                    2.0 MB  Oct 11 12:09      │╭ Cameras in this batch ─────────────────────────╮
 │[x] Sony  DSC03712.ARW                    2.0 MB  Oct 11 12:09      ││● DJI      10/10  files   595.5 MB              │
 │[x] Sony  DSC03713.ARW                    2.0 MB  Oct 11 12:09      ││● Sony      5/5   files    28.0 MB              │
 │[x] Sony  C2546.MP4                      20.0 MB  Oct 11 12:09      │╰────────────────────────────────────────────────╯
 │                                                                    │╭ Sources ───────────────────────────────────────╮
 │                                                                    ││● /tmp/offloa…emo/SD_Card/DCIM/DJI_001  10 files│
 │                                                                    ││● /tmp/offload-demo/OsmoAction/DCIM  0 files    │
 │                                                                    ││● /tmp/offload-demo/Untitled/DCIM  4 files      │
 │                                                                    ││● /tmp/offload…tled/private/M4ROOT/CLIP  1 files│
 │                                                                    │╰────────────────────────────────────────────────╯
 │                                                                    │╭ By type ───────────────────────────────────────╮
 │                                                                    ││ARW     4     8.0 MB  ▎                         │
 │                                                                    ││MP4    10   614.0 MB  ██████████████████████    │
 │                                                                    ││WAV     1     1.5 MB                            │
 │                                                                    │╰────────────────────────────────────────────────╯
 ╰────────────────────────────────────────────────────────────────────╯╭ Shoot dates  (tab) ────────────────────────────╮
 ╭ Details ───────────────────────────────────────────────────────────╮│[x] Thu 08 Oct 2026    1 files     1.5 MB       │
 │/tmp/offload-demo/SD_Card/DCIM/DJI_001/DJI_20261005_0001_D.WAV      ││[x] Sun 11 Oct 2026   14 files   622.0 MB       │
 │WAV · 1.5 MB · Thu Oct 08 2026, 12:09:33                            ││                                                │
 │from DJI_001                                                        ││                                                │
 ╰────────────────────────────────────────────────────────────────────╯╰────────────────────────────────────────────────╯
space toggle  a all/none  tab dates  m mode: copy  o by today  r mirror off  enter go  esc back  ? help
```

```
 ╭ ⠧ COPYING  Batch · DJI + Sony ───────────────────────────────────────────────────────────────────────────────────────╮
 │copy   ████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  18%  110.3 MB / 623.5 MB               │
 │verify ███████▉░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  11%  70.5 MB / 623.5 MB                │
 │files  3/15   speed 77.9 MB/s   eta 6s   elapsed 1s                                                                   │
 │DJI    ██████▏░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  15%  3/10 files · 595.5 MB                                           │
 │Sony   ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   0%  0/5 files · 28.0 MB                                             │
 │                                                                                                                      │
 ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 ╭ Throughput ────────────────────────────────────────────────────────────────────╮╭ Backup drive ──────────────────────╮
 │▄█▇▆▇                                                                           ││███████████████████████████████▊░░░░│
 │█████                                                                           ││58.9 GB free                        │
 │█████                                                                           ││of 494.4 GB                         │
 ╰────────────────────────────────────────────────────────────────────────────────╯╰────────────────────────────────────╯
 ╭ Files  → /tmp/offload-demo/Backup/2026-10-11 ────────────────────────────────────────────────────────────────────────╮
 │✓ DJI   DJI_20261005_0001_D.WAV           1.5 MB  verified                                                            │
 │✓ DJI   DJI_20261005_0001_D.MP4          30.0 MB  verified                                                            │
 │✓ DJI   DJI_20261005_0002_D.MP4          39.0 MB  verified                                                            │
 │⠧ DJI   DJI_20261005_0003_D.MP4          48.0 MB  ██████████░░ copying  83%                                           │
 │· DJI   DJI_20261005_0004_D.MP4          57.0 MB  queued                                                              │
 │· DJI   DJI_20261005_0005_D.MP4          66.0 MB  queued                                                              │
 │· DJI   DJI_20261006_0006_D.MP4          75.0 MB  queued                                                              │
 │· DJI   DJI_20261006_0007_D.MP4          84.0 MB  queued                                                              │
 │· DJI   DJI_20261006_0008_D.MP4          93.0 MB  queued                                                              │
 │· DJI   DJI_20261006_0009_D.MP4         102.0 MB  queued                                                              │
 │· Sony  DSC03710.ARW                      2.0 MB  queued                                                              │
 │· Sony  DSC03711.ARW                      2.0 MB  queued                                                              │
 │· Sony  DSC03712.ARW                      2.0 MB  queued                                                              │
 │· Sony  DSC03713.ARW                      2.0 MB  queued                                                              │
 │· Sony  C2546.MP4                        20.0 MB  queued                                                              │
 │                                                                                                                      │
 │                                                                                                                      │
 │                                                                                                                      │
 ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
p pause  ↑↓ scroll  f follow  esc cancel
```

```
     ╭ Batch · DJI + Sony ────────────────────────────────────────────────────────────────────╮
     │                            ✓  All files copied and verified                            │
     │                                                                                        │
     │copied     15 files · 623.5 MB                                                          │
     │skipped    0 already on destination                                                     │
     │renamed    0 (name clashed with a different file; both kept)                            │
     │failed     0                                                                            │
     │time       9s  (67.2 MB/s)                                                              │
     │saved to   /tmp/offload-demo/Backup/2026-10-11                                          │
     │                                                                                        │
     │✓ DJI     10 copied · 595.5 MB                                                          │
     │✓ Sony    5 copied · 28.0 MB                                                            │
     │                                                                                        │
     │Every copied file was re-read from disk and matched its source checksum.                │
     │originals  still on the card · press d to delete the 15 verified ones (623.5 MB)        │
     │                                                                                        │
     │                                                                                        │
     │                                                                                        │
     ╰────────────────────────────────────────────────────────────────────────────────────────╯


o open folder  d delete originals  x eject card  s stats  enter home  q quit
```

## Features

### Verified copying
BLAKE3 hashing while reading, a full flush, an uncached re-read, and a hash compare before the file gets its
real name. Duplicates are detected by **checksum**, not name or size. A *different* file with the same name is
never overwritten: both are kept (`C0001.MP4`, `C0001_1.MP4`). Modified times are preserved.

### Batch mode: every camera at once
With two or more profiles the home screen shows a **batch** card (<kbd>b</kbd>). `offload` scans every card that is plugged in and gives you
**one review** (files color-tagged by camera), **one dashboard** (an overall bar plus one line per camera) and **one summary**.
A camera that isn't plugged in is left out and you're told which. Each camera keeps its own subfolder, so identical names never collide,
and free space is checked per drive.

### Copy or move, and deleting the originals afterwards
Press <kbd>m</kbd> on the review screen:

| Mode | Originals on the card | Use it when |
| --- | --- | --- |
| **Copy** (default) | stay | you want to look over the backup first |
| **Move** | each one is deleted right after *its* copy is verified | you want the card emptied as you go |

In copy mode the summary offers *"press d to delete the N verified originals"*, so you can back up, check, then clear the card.
Both routes ask for an explicit <kbd>y</kbd> (<kbd>enter</kbd> alone never confirms), and share the same guardrails:

- only files whose copies are **checksum-verified** (or already there and checksum-identical) are eligible;
- the copy must **still be on the destination** at the right size, and the original must be a plain file **unchanged since the scan**;
- with a **second drive** enabled, *both* copies must be verified;
- on any doubt the original is **kept** and reported. Failed, cancelled and unverified files are never touched;
- deletion is permanent (no Trash), which is why it is always behind a confirmation.

### Folders by shoot date
By default each offload goes into one dated folder (today). Switch to **shoot date** and every clip is filed under the day it was
*recorded*, so a card holding Monday to Friday becomes five folders instead of one:

```
Needs to Back up/
├── 2026-10-07/DJI/…
├── 2026-10-09/DJI/…
└── 2026-10-11/DJI/…
```

Toggle it per run with <kbd>o</kbd> on the review screen, or set `organize = "shoot"` as your default.

### Mirror to a second drive
Set a mirror folder and every file is written and verified on **both** drives in the same pass. The card is read **once**, hashed once,
and streamed to both destinations, so a second copy costs write time, not a second trip through your slow card reader.
It's off unless you configure it; once configured, <kbd>r</kbd> turns it on or off per run.

- Each drive's copy is verified independently against the source hash.
- A mirror problem never fails or discards the main copy. It is reported loudly, the original is kept, and no sleep or shutdown is triggered.
- If the mirror drive isn't mounted, you're told before anything starts.
- Re-running with a mirror newly added fills in only the mirror (the main copy is skipped by checksum).

### Keep awake, then sleep or shut down
A long copy shouldn't be undone by the Mac dozing off. While a copy runs, `offload` holds the machine awake (`caffeinate`,
which also releases by itself if `offload` is killed). Optionally, when a run finishes **cleanly** it can put the Mac to sleep or shut it down, after a
60-second on-screen countdown that **any key cancels**. It never fires after a cancelled run, a failure, a mirror problem or a kept original.

```
     ╭ DJI Osmo Action ───────────────────────────────────────────────────────────────────────╮
     │                            ✓  All files copied and verified                            │
     │                                                                                        │
     │copied     5 files · 5.0 MB                                                             │
     │skipped    0 already on destination                                                     │
     │renamed    0 (name clashed with a different file; both kept)                            │
     │failed     0                                                                            │
     │time       0s                                                                           │
     │saved to   /tmp/offload-demo2/Backup                                                    │
     │                                                                                        │
     │mirror     5 files verified on the second drive                                         │
     │Every copied file was re-read from disk and matched its source checksum.                │
     │⏻ This Mac will go to sleep in 59s - press any key to cancel                            │
     │originals  still on the card · press d to delete the 5 verified ones (5.0 MB)           │
     │                                                                                        │
     ╰────────────────────────────────────────────────────────────────────────────────────────╯



o open folder  d delete originals  x eject card  s stats  enter home  q quit
```

### Everything else
- **Selectable review** with per-file and per-day filtering and a details panel.
- **Safe ejecting** of the cards you just copied (with confirmation) and a desktop notification when you've walked away.
- **Stats** (<kbd>s</kbd>): lifetime totals, per-month chart, per-camera totals, recent sessions.
- **Five themes** (catppuccin, nord, gruvbox, dracula, mono), remembered between runs.
- **Mouse** support everywhere, plus key hints in the footer and a `?` help overlay.
- **Profile packs**: share a camera's setup as one TOML file.
- **Scriptable**: `offload run` does everything without the UI.

## Options at a glance

Everything below can be set in the settings screen (<kbd>e</kbd>), in `config.toml`, and (where it makes sense) per run on the review screen or the command line.

| Option | Default | Config key | Review key | CLI flag |
| --- | --- | --- | --- | --- |
| Copy or move | copy | `mode` | <kbd>m</kbd> | `--copy` / `--move` |
| Folders by today's date or shoot date | today | `organize` = `run` \| `shoot` | <kbd>o</kbd> | `--by-run-date` / `--by-shoot-date` |
| Mirror to a second drive | off | `mirror` = path | <kbd>r</kbd> | `--mirror PATH` / `--no-mirror` |
| Keep the Mac awake while copying | on | `keep_awake` | n/a | `--no-awake` |
| After a clean run | nothing | `when_done` = `nothing` \| `sleep` \| `shutdown` | n/a | `--when-done ACTION` |
| Notification when done | on | `notify` | n/a | n/a |
| Dated folder name | `%Y-%m-%d` | `date_format` | n/a | n/a |

```
 using built-in profiles · your first edit creates /tmp/offload-demo2/cfg/config.toml

 ╭ Settings · changes save instantly ─────────────────────────────────────────────────────────────╮
 │  Backup folder       /tmp/offload-demo2/Backup                                                 │
 │  Date folder format  %Y-%m-%d                                                                  │
 │  Default mode        copy (keep originals)                                                     │
 │  Folders by          today's date (one folder per offload)                                     │
 │▸ Mirror drive        /tmp/offload-demo2/Second SSD                                             │
 │  Keep Mac awake      on (while copying)                                                        │
 │  When finished       sleep the Mac (after a clean run)                                         │
 │  Notify when done    on                                                                        │
 │  Theme               catppuccin                                                                │
 │  DJI Osmo Action                                                                               │
 │    Name                DJI Osmo Action                                                         │
 │    Subfolder           DJI                                                                     │
 ╰────────────────────────────────────────────────────────────────────────────────────────────────╯
↑↓ move  enter edit  d delete profile  esc back
```

## Configuration

The config lives at `~/.config/offload/config.toml` (or `$XDG_CONFIG_HOME/offload/`; override with `--config`).
With no file, the built-in `hectorir` profiles are used. Write them out as a starting point:

```sh
offload init          # writes the hectorir config so you can edit it
offload config-path   # shows where it lives
```

Or edit inside the app with <kbd>e</kbd>. Changes are validated and saved instantly (a `.bak` of the previous file is kept).

```toml
# Where backups go. A dated folder is created inside.
destination = "/Volumes/Old External Drives/0 Photos & My Footage/Needs to Back up"
date_format = "%Y-%m-%d"        # strftime; must produce a single folder name
organize    = "run"             # run = one folder per offload | shoot = one per day filmed
mode        = "copy"            # copy = keep originals | move = delete each once verified
mirror      = ""                # a second drive/folder; every file is also written + verified there
keep_awake  = true              # hold the computer awake during a copy
when_done   = "nothing"         # nothing | sleep | shutdown   (only after a clean run)
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

- Sources are scanned **recursively**. A source that isn't mounted is skipped with a warning; the rest still copy.
- Symlinks and macOS `._*` / `.DS_Store` files are ignored.
- `subfolder`, `date_format` and the mirror are validated: anything that could escape the dated folder (`..`, `/`) is rejected, and the mirror can't be the same folder as the destination.
- `organize = "shoot"` needs a `date_format` that names a day (a time-only format is refused).
- Add as many `[[profile]]` blocks as you have cameras: GoPro, Insta360, a drone, an audio recorder.
- Older configs without the newer keys keep working; missing keys take the defaults above.

### Sharing camera setups: profile packs

A pack is a single TOML file describing one camera (never your drive paths):

```sh
offload pack export "DJI Osmo Action" -o osmo.toml
offload pack import osmo.toml         # validated, then added to your config
```

If you set up a camera others use, open a PR adding it to [`examples/`](examples).

## Command line

```
offload                          open the TUI
offload run <profile>... [--all] [--yes] [--dry-run]
                         [--move | --copy]
                         [--by-shoot-date | --by-run-date]
                         [--mirror FOLDER | --no-mirror]
                         [--no-awake] [--when-done nothing|sleep|shutdown]
                                 copy without the UI. Partial names work (`offload run dji`);
                                 several names, or --all, copy them together as one batch
offload profiles                 list profiles and whether their cards are connected
offload init [--force]           write the built-in config to disk
offload pack export|import       share profiles
offload --config FILE ...        use a different config file
```

```sh
offload run --all --dry-run                          # see exactly what would happen; writes nothing
offload run dji sony --yes                           # both cameras, no prompt
offload run --all --by-shoot-date --mirror /Volumes/SSD/Backup --yes
offload run sony --move --when-done sleep --yes      # empty the card, then sleep the Mac
```

Exit codes: `0` success · `1` some files failed, a mirror copy failed, or the run was cancelled · `2` error (bad config, drive not mounted, no space).

## Keys

| Where | Key | Action |
| --- | --- | --- |
| Home | `↑↓` `j k` / `1-9` / click | choose a profile |
| | `enter` | scan the card |
| | `b` / `0` | batch: every connected camera |
| | `e` `s` `t` | settings · stats · cycle theme |
| Review | `space` | toggle a file / a whole day |
| | `tab` | switch files ⇄ shoot dates |
| | `a` | select all / none |
| | `m` | copy ⇄ move |
| | `o` | folders by today's date ⇄ shoot date |
| | `r` | mirror to the second drive on ⇄ off |
| | `enter` | start (blocked if there's no space or a drive is missing) |
| Copying | `p` | pause / resume |
| | `esc` | cancel (asks first) |
| Summary | `o` `x` | open folder · eject cards (asks first) |
| | `d` | delete the verified originals (copy mode; asks first) |
| | any key | cancels a pending sleep / shutdown |
| Anywhere | `?` | help |
| | `ctrl+c` | quit (cleans up partial files) |

The mouse works too: click profiles, files, dates and settings; scroll lists with the wheel.

## How it stays safe

```
card ──read once──▶ [hash while reading] ──┬──▶ .name.part  (main drive)
                                           └──▶ .name.part  (mirror, if enabled)
                                                   │ flush to disk
                                                   ▼
                       re-read from each drive, cache bypassed ──▶ BLAKE3 ══ source hash?
                                                   │ yes, on every drive
                                                   ▼
                                    rename to name  (+ restore modified time)
                                                   │
                                                   ▼
                          (move only) delete the original, after re-checking
                          every copy exists and the original is unchanged
```

- The source hash is computed on the exact bytes read from the card, so what's verified is what was copied.
- On macOS the verify read uses `F_NOCACHE` and a full flush where the filesystem supports it (network shares fall back to `fsync`), so the check reads the drive, not RAM.
- Existing files are never overwritten. Files are created with `O_EXCL | O_NOFOLLOW`, so a planted symlink can't redirect a write.
- If your backup drive (`/Volumes/…`) isn't mounted, `offload` stops instead of silently filling your internal disk.
- Sources are opened read-only. Only files in the plan are ever deleted, and only after the checks above.
- File names are sanitized before display so a hostile name can't inject terminal escape codes.
- External commands (`diskutil`, `osascript`, `caffeinate`, `pmset`, `open`) are spawned with argument vectors, never through a shell.
- No network access of any kind.

### Honest limits

- Verification proves the copy matches what was read from the card. It can't detect a card that returns wrong data consistently.
- The details panel shows file metadata, not thumbnails or video duration.
- Skipping an existing file means hashing it, which reads it once. That's the price of trusting checksums over names.
- After a hard kill or power loss a hidden `.name.part` may be left behind. It is never mistaken for a good file and is replaced on the next run.
- A mirror is a second *copy*, not a sync: later changes on one drive are not propagated to the other.
- Tested on macOS. The Linux build is exercised in CI but less battle-tested.

## FAQ

**Does it ever touch my card?** Not in copy mode. In move mode, or after you press <kbd>d</kbd>, it deletes only files whose verified copies it just re-confirmed.

**What if the power dies mid-copy?** Run it again. Finished files are recognised by checksum and skipped; half-written ones were never given a real name.

**Can I use it with a drive that isn't under `/Volumes`?** Yes. The "is the drive mounted" guard applies to `/Volumes/…` paths; any other path is used as given.

**Where are my stats?** `~/.local/share/offload/history.jsonl`, one line per camera per run. Press <kbd>s</kbd> in the app.

**How do I add a GoPro?** Press <kbd>e</kbd> and choose *Add a profile*, or copy [`examples/gopro.pack.toml`](examples/gopro.pack.toml) and run `offload pack import`.

## Development

```sh
cargo test                # unit + integration + TUI + CLI tests (all against temp dirs)
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo test --test app_flow dump_frames  -- --ignored --nocapture   # render the main screens as text
cargo test --test app_flow dump_options -- --ignored --nocapture   # …the folder / mirror / sleep screens
```

| Module | Role |
| --- | --- |
| `config` | profiles, validation, the `hectorir` preset, packs |
| `scan` | recursive, filtered, symlink-safe discovery; merging cameras into a batch |
| `engine` | read → hash → write (1 or 2 drives) → verify → finalize; dedupe, pause, cancel, guarded deletion |
| `app` / `ui` / `widgets` / `theme` | state machine, rendering, gradient bars, themes |
| `sys` | notifications, eject, keep-awake, sleep / shutdown |
| `history` | JSONL log and lifetime stats |
| `headless` | the scriptable `offload run` |

The test suite never touches real volumes. It builds throwaway card and backup trees and asserts on bytes, including cancel mid-copy,
stale partials, symlink attacks, name collisions, unmounted drives, corrupted and truncated copies, failing mirrors, and every
condition under which an original must be kept.

Contributions welcome: new camera packs, themes and bug reports especially.

## License

MIT. See [LICENSE](LICENSE).
