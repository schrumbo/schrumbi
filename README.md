<p align="center">
  <img src="assets/brand/app-icon.svg" width="96" alt="Schrumbi app icon">
</p>

<h1 align="center">Schrumbi</h1>

<p align="center">Study planner for deadlines, files and notes, working directly on your own folders.</p>

Schrumbi is a desktop app built with Rust and [GPUI](https://www.gpui.rs). It reads a normal folder of course files, so your data stays plain files on your disk. The interface is available in German and English.

## Features

- **Overview:** what is due now, the next seven days, and overdue work per module
- **Semester and grades:** weekly progress per module, grades and the marks you still need
- **Inbox:** suggests where a file belongs and names it by your convention; press Enter to file it
- **Search:** full-text search across file names, PDFs and notes, with a jump to each match
- **PDF viewer:** thumbnails, outline, text search, highlights, links and a remembered position per file
- **Notes:** file browser for your whole folder with tabs, tags, tasks and a graph; Markdown code editor, optional live mode, reader, split view and PDF export
- **Sync:** optional, through Git, with conflict handling; requires Git to be installed
- **Obsidian:** optional; open or connect an existing vault and keep using both side by side
- **Undo:** step back through filing, renames, status changes and deadlines with `⌘Z`
- **Keyboard:** command palette and configurable shortcuts
- **Menu bar:** next deadline and upcoming days at a glance

## Install

Download the latest version from [Releases](https://github.com/schrumbo/schrumbi/releases/latest).

| Platform | File |
|---|---|
| macOS (Apple Silicon and Intel) | `Schrumbi-<version>-macos-universal.dmg` |
| Windows (x64) | `Schrumbi-Setup-<version>-windows-x64.exe` |
| Linux (x86_64) | `Schrumbi-<version>-linux-x86_64.AppImage` or `schrumbi_<version>-1_amd64.deb` |

macOS is the primary platform. Windows and Linux builds are less tested.

The macOS build is not notarized, so macOS blocks the first launch. Allow it under **System Settings → Privacy & Security → Open Anyway**.

## Shortcuts

| Action | macOS |
|---|---|
| Command palette | `⌘P` (`⌘K` outside the editor) |
| Shortcut overview | `⌘/` |
| Search | `/` |
| Quick open | `⌘O` |
| New note | `⌘N` |
| Switch reading and editing | `⇧⌘V` |
| Undo | `⌘Z` |

Most shortcuts can be changed in the settings.

## Build from source

Requires a current stable [Rust](https://rustup.rs) toolchain.

```sh
git clone https://github.com/schrumbo/schrumbi.git
cd schrumbi
cargo run
```

On Debian or Ubuntu, install the system libraries first:

```sh
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libdbus-1-dev pkg-config
```

Run the tests with `cargo test`.

## License

Copyright (c) 2026 Schrumbo. Licensed under the [GNU Affero General Public License v3.0](LICENSE).
