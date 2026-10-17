# Pigo

> A radial (pie-menu) app launcher for Windows — fast, mouse/keyboard-driven, with live window previews.

pigo 是一个 Windows 上的**圆形双环菜单启动器**：快捷键/鼠标手势呼出，内环放常用应用、外环放它的窗口，悬停即见实时预览。原生 Win32 实现，零第三方运行时，单文件 exe（约 700KB）。

**English:** Pigo is a radial-menu launcher for Windows 10/11 (x64). No UI framework — pure Win32 + `windows-rs`. It shows a radial menu centered on the cursor: the inner ring holds your pinned apps, the outer ring holds that app's open windows, and hovering a window shows a live DWM thumbnail (works even for minimized windows).

## Screenshots

| 主盘 (inner ring) | 外环胶囊 (outer chips) |
| --- | --- |
| ![ring](design/preview-ring.png) | ![chips](design/preview-chips.png) |

| 实时预览 (live preview) | 应用管理 (app management) |
| --- | --- |
| ![live](design/preview-live.png) | ![apps](design/preview-apps.png) |

## Features

- **径向菜单**：光标居中，内环 = 固定应用，外环 = 该应用的窗口（按默认间距贴合在对应内环图标附近）。
- **拖入即固定**：把 `.lnk` / 文件夹 / 文件拖进内环磁贴；支持重排、改名、删除。
- **运行中标识**：正在运行的应用，内环图标下显示圆点；悬停有窗口的应用，外环列出其窗口，悬停即弹**实时预览**（`DwmRegisterThumbnail`，最小化窗口也有画面）。
- **液态玻璃材质**：极淡，仅作用于轮盘；设置页始终为纸面。
- **内置动作**：如 `@act:start` 打开 Windows 开始菜单。
- **自启 / 常驻 / 自定义呼出**：呼出方式可在设置里改为鼠标手势或键盘。
- **单文件、零依赖**：编译产物一个 `Pigo.exe`，无需 .NET / 运行时。

## Requirements

- Windows 10 / 11, **x64**
- Rust (edition 2021) — 通过 [rustup](https://rustup.rs/) 安装 `cargo`
- **Windows SDK**（用于嵌入 exe 图标；缺失时自动跳过图标，不影响编译）
- MSVC 工具链（Visual Studio Build Tools 或 `winget install Microsoft.VisualStudio.BuildTools`）

## Build

```bash
cargo build --release
# 产物: target/release/Pigo.exe
```

> ⚠️ `Cargo.toml` 中 `lto = "thin"` **不要改为 `"true"`（fat LTO）**：当前 rustc 会对这份 unsafe FFI 代码误编译，启动即崩 `0xC0000409`。

打包成单文件 zip（含用法说明）：

```bash
python package.py
# 输出: dist/Pigo.exe  +  dist/pigo-v1.0.0-win64.zip
```

## Usage

- **呼出**：默认快捷键 / 鼠标手势（在设置里可改）；菜单在光标处弹出。
- **固定应用**：把 `.lnk` / 文件夹 / 文件拖进内环磁贴；它的窗口会自动归到对应应用的外环。
- **设置**：在菜单里打开设置页（纸面，非玻璃）。
- **内置动作**：`@act:start` 打开 Windows 开始菜单。
- **运行中标识**：应用运行时内环图标下显示圆点；悬停有窗口的应用，外环列出其窗口，悬停即实时预览。

## Project layout

```
src/main.rs            主程序（全部逻辑，原生 Win32）
app.rc / app.ico       版本信息与 exe 图标
build.rs               用 Windows SDK 的 rc.exe 嵌入图标（缺失则跳过）
package.py             一键打包成 zip
test_*.py              自动化 UI 测试（需先启动 Pigo.exe）
design/preview-*.png   README 用预览图
```

## License

[MIT](LICENSE) © 2026 pigo
