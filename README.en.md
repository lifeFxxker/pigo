# pigo

<p align="left">
  <a href="https://github.com/lifeFxxker/pigo/releases"><img src="https://img.shields.io/github/v/release/lifeFxxker/pigo?color=black&label=release" alt="Release"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-1f1f1f" alt="Platform">
  <img src="https://img.shields.io/badge/language-Rust-1f1f1f" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-1f1f1f" alt="License">
</p>

**[English](README.en.md) · [简体中文](README.md)**

A radial app launcher for Windows. Summon a cursor-centered dial with the middle mouse button, pin your frequently used apps to the inner ring, and hover an app to fan out its open windows on the outer ring — with a live window preview. One native executable, no runtime, no installer.

---

## Demo

![Demo](design/preview-demo.gif)

Hover an app to fan out its open windows; hold and drag an inner tile to reorder it (insertion, not a simple swap).

## Screenshots

| Inner ring | Outer ring — an app's open windows |
| :---: | :---: |
| ![Inner ring](design/preview-ring.png) | ![Outer ring](design/preview-chips.png) |
| A running app gets a dot under its icon. | Hovering an app fans its windows out beside it. |

| Live window preview | Settings |
| :---: | :---: |
| ![Live preview](design/preview-live.png) | ![Settings](design/preview-apps.png) |
| Hover a window chip for a live DWM thumbnail. | Flat, paper-style preferences. |

## Features

- **Cursor-centered dial** — Appears exactly where your pointer is, so you never travel to a corner of the screen.
- **Two-level layout** — The inner ring holds pinned apps; the outer ring expands the hovered app's windows as capsules grouped next to that app, not spread across the whole circle.
- **Drag to organize** — Drop a shortcut, `.exe`, folder, or file onto the inner ring to pin it. Reorder, rename, and remove from the same view.
- **Live window previews** — Hover a window capsule to see that window in real time (`DwmRegisterThumbnail`), including minimized windows. Video and animation stay live — it is not a screenshot.
- **Running indicators** — Apps with an open window show a small dot beneath their icon.
- **Liquid-glass material** — A subtle glass finish on the dial only; the settings page stays flat paper for readability.
- **Built-in actions** — e.g. `@act:start` opens the Windows Start menu directly.
- **Flexible invocation** — Middle mouse button, a side button, long-press right button, or a recorded key combo such as `Ctrl+W`.
- **Resident & autostart** — Optionally start at login and live in the tray.
- **Single file, zero dependencies** — The build output is one `Pigo.exe` (≈700 KB); no .NET or other runtime is required.

## Installation

### Download

Grab the latest `pigo-vX.Y.Z-win64.zip` from the [Releases](https://github.com/lifeFxxker/pigo/releases) page, unzip it anywhere, and run `Pigo.exe`.

> If no release is published yet, build it locally (below).

### Build from source

**Prerequisites**

- Windows 10 / 11, **x64**
- [Rust](https://rustup.rs/) (edition 2021)
- MSVC toolchain — Visual Studio Build Tools, or `winget install Microsoft.VisualStudio.BuildTools`
- Windows SDK — used to embed the exe icon; if it is missing the icon step is skipped and the build still succeeds

```bash
cargo build --release
# -> target/release/Pigo.exe
```

> **Keep `lto = "thin"` in `Cargo.toml`.** Do not switch to fat LTO (`"true"`): the current toolchain miscompiles this project's `unsafe` FFI and the app crashes on launch (`0xC0000409`).

Package a single-file zip:

```bash
python package.py
# -> dist/Pigo.exe  and  dist/pigo-v1.0.0-win64.zip
```

## Usage

- **Invoke** — Hold the **middle mouse button** (or a side button) to pop the dial at the cursor. Release to launch the highlighted app; press `Esc` or any other key to cancel. Switch to long-press right button or a custom combo in Settings.
- **Pin an app** — Drag a shortcut, `.exe`, folder, or file onto an inner tile.
- **Add a window** — Drag a window onto the dial to group it under its app; it then appears on that app's outer ring.
- **Preview** — Hover an app to fan out its windows; hover a window capsule for a live preview.
- **Settings** — Open from the tray icon (right-click → 设置…). The settings page is flat paper, deliberately not glass.

## Configuration

Configuration lives in `%APPDATA%\pigo\settings.ini`, created on first run and normally edited through the in-app Settings page. Logs are written to `%LOCALAPPDATA%\pigo\logs\`.

## How it works

- **Pure Win32 via `windows-rs`** — No UI framework. A self-contained software renderer keeps the binary small, the startup fast, and the footprint runtime-free.
- **Layered window** — The dial is presented through `UpdateLayeredWindow` with premultiplied alpha. The glass material samples the desktop behind the dial, which is why it is refreshed on every invocation.
- **Previews are DWM thumbnails** — Because a layered window cannot be a DWM thumbnail source, previews are rendered in a separate plain top-level window using `DwmRegisterThumbnail`. The result is a live picture rather than a static capture, and minimized windows still show their last frame.
- **One layout for drawing and hit-testing** — The inner ring and outer capsules share a single layout routine, so what you see is exactly what you click.

## Project layout

```
src/main.rs            All logic — native Win32 + a software renderer
app.rc / app.ico       Executable version info and icon
build.rs               Embeds the icon via the Windows SDK rc.exe (skipped if absent)
package.py             One-step packaging into a zip
test_*.py              Automated UI tests (launch Pigo.exe first)
design/preview-*.png   Screenshots used by the README
```

## License

[MIT](LICENSE) © 2026 pigo
