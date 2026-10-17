# Pigo

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0078D6?logo=windows&logoColor=white" alt="Platform">
  <img src="https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
</p>

**pigo** is a radial (pie-menu) app launcher for Windows. Summon a cursor-centered dial with a mouse gesture or hotkey, pin your apps to the inner ring, and peek live window previews by hovering the outer ring — all in a single native executable with no runtime dependencies.

**中文简介：** pigo 是一个 Windows 平台的光标居中双环启动器。按住鼠标中键（或侧键）在光标处弹出轮盘，内环固定常用应用、外环展开该应用当前打开的窗口；悬停即显示实时窗口预览（基于 DWM 缩略图，最小化窗口也有画面）。原生 Win32 实现，单文件 exe，零运行时依赖。

## Screenshots

| Inner ring (主盘) | Outer chips (外环胶囊) |
| --- | --- |
| ![ring](design/preview-ring.png) | ![chips](design/preview-chips.png) |

| Live preview (实时预览) | App management (应用管理) |
| --- | --- |
| ![live](design/preview-live.png) | ![apps](design/preview-apps.png) |

## Features

- **Cursor-centered dual ring** — The inner ring holds your pinned apps; the outer ring expands that app's open windows, laid out at the default spacing right next to the corresponding inner icon. Hit-testing and drawing share one layout routine, so what you see is exactly what you click.
- **Drag-to-pin** — Drop a `.lnk`, folder, or file onto an inner tile to pin it. Reorder, rename, and remove are all supported.
- **Live running state** — A running app shows a dot under its inner icon. Hover an app that has windows and the outer ring lists them; hovering a window pops a **live preview** (`DwmRegisterThumbnail`, including minimized windows).
- **Liquid-glass material** — A very subtle glass effect applied to the dial only; the settings page stays flat paper for readability.
- **Built-in actions** — e.g. `@act:start` opens the Windows Start menu directly.
- **Autostart / resident / custom invoke** — Run at login and stay in the tray. Invocation can be middle button, side button, long-press right button, or a custom key combo (e.g. `Ctrl+W`).
- **Single file, zero dependencies** — The build output is one `Pigo.exe`; no .NET or other runtime required.

## Installation

### Download a prebuilt package

Grab `pigo-vX.Y.Z-win64.zip` from the [Releases](https://github.com/lifeFxxker/pigo/releases) page, unzip, and run `Pigo.exe`.

> If no Release is published yet, build it locally — see below.

### Build from source

**Prerequisites**

- Windows 10 / 11, **x64**
- Rust (edition 2021) via [rustup](https://rustup.rs/)
- MSVC toolchain — Visual Studio Build Tools, or `winget install Microsoft.VisualStudio.BuildTools`
- Windows SDK (used to embed the exe icon; if absent, the icon step is skipped and the build still succeeds)

```bash
cargo build --release
# output: target/release/Pigo.exe
```

> ⚠️ Keep `lto = "thin"` in `Cargo.toml`. Do **not** switch it to `"true"` (fat LTO): the current rustc miscompiles this project's `unsafe` FFI code, causing an immediate crash on launch (`0xC0000409`).

Package into a single-file zip (with a usage note):

```bash
python package.py
# output: dist/Pigo.exe  and  dist/pigo-v1.0.0-win64.zip
```

## Usage

- **Invoke** — By default, hold the **middle mouse button** (or a side button) to pop the dial at the cursor; release to execute, `Esc` / any other key cancels. In settings you can switch to long-press right button or record a custom combo such as `Ctrl+W`.
- **Pin an app** — Drag a `.lnk`, folder, or file onto an inner tile. Its windows are grouped onto that app's outer ring automatically.
- **Running indicator** — A dot under an inner icon means the app is running; hovering an app that owns windows lists them on the outer ring with live previews on hover.
- **Built-in action** — `@act:start` opens the Windows Start menu.
- **Settings** — Open the settings page from the menu (flat paper style, not glass).

## Configuration

pigo stores its config at `%APPDATA%\pigo\settings.ini`, created automatically on first run. The app list, invoke method, and appearance are all edited from the in-app settings page — manual editing is rarely needed.

## Project layout

```
src/main.rs            全部逻辑，原生 Win32 实现（自有软件渲染器）
app.rc / app.ico       exe 版本信息与图标
build.rs               调用 Windows SDK 的 rc.exe 嵌入图标（缺失则跳过）
package.py             一键打包为 zip
test_*.py              自动化 UI 测试（需先启动 Pigo.exe）
design/preview-*.png   README 用预览图
```

## Technical notes

- **Pure Win32 + `windows-rs`** — No UI framework and a self-contained software renderer, which keeps the binary small (~700KB), launches fast, and carries no runtime.
- **Live previews via DWM thumbnails** — Hover previews use `DwmRegisterThumbnail`, showing the target window's live picture (including a minimized window's snapshot) rather than a static screenshot, so video and animation stay live.
- **Layered window for the glass** — The dial renders through `UpdateLayeredWindow` with premultiplied alpha. Because a layered (ULW) window cannot be a DWM thumbnail source, previews use a separate plain top-level window.

## License

[MIT](LICENSE) © 2026 pigo
