# 径向启动器（Radial Launcher）

一个**极轻量的 Windows 径向菜单启动器**。用 Rust 直接调 Win32 API 手写渲染，
没有 UI 框架、没有第三方运行时依赖 —— 单个 exe，绿色免安装。

```
radial-launcher/          ← 源码（Rust 项目）
├─ src/main.rs            ← 全部实现（单文件，约 357KB）
├─ app.rc / app.ico       ← exe 图标资源
├─ build.rs               ← 调 Windows SDK 的 rc.exe 编译图标资源
├─ make_icon.py           ← 按中心头像的真实几何生成 app.ico
├─ package.py             ← 一键打包（编译 → 校验 → dist/ → zip）
├─ ring_geom.py           ← 布局公式的 Python 镜像（改 Rust 布局要同步改它）
└─ test_*.py              ← 自动化回归测试
```

## 特性

- **中键 / 鼠标侧键 / 长按右键** 呼出轮盘（三个开关可独立启用）
- **内环** = 应用图标；**外环** = 该应用的内容，点一下直接打开
- 外环是**圆角环形扇区**（梯形段），颜色、描边、投影与内环底板完全一致
- **双击外环段就地改名**；**按住图标拖出盘外移除**
- 中心是**会呼吸、会眨眼、眼神跟着鼠标走**的表情球（拖动时还会有情绪）
- 三级交互状态：常驻模式 / 键盘全程可控

## 设计基准

- **全中性灰阶**，界面里唯一有颜色的东西是**真实的应用图标**
- 扁平纯色：零渐变、零模糊；投影是**不模糊的硬投影**
- 统一圆角方块是「整齐」的唯一来源 —— 无论图标自带圆形/白纸/满出血方块，
  最终都收敛成同一个圆角方块（纹理阶段就裁好）

## 构建

需要 **Rust 工具链**（stable）。图标资源需要 Windows SDK 的 `rc.exe`（找不到会跳过，
只是 exe 没有自定义图标，不影响功能）。

```sh
cd radial-launcher
cargo build --release
```

产物：`target/release/radial-launcher.exe`（Release 是 **GUI 子系统**，双击不弹黑窗口）。

### 一键打包

```sh
python package.py
```

会编译 → 校验 PE 子系统与体积 → 产出 `dist/`（`RadialLauncher.exe` + `README.txt` + zip）。

### 重新生成图标

```sh
python make_icon.py
```

按 `src/main.rs` 里中心头像球的几何参数（球 = ink、眼睛 = paper 药丸）生成多尺寸
`app.ico`。小尺寸会自动补偿眼睛比例；球外加了一圈**极淡的白色描边**，
这样在深色任务栏上也看得见（浅色底上它会融进背景）。

## 运行

双击 `RadialLauncher.exe` 即可。配置在同目录的 `settings.ini`（首次运行自动生成）。

`RadialLauncher.exe --quit` 退出正在运行的实例。

## 测试

```sh
python test_rename.py     # 双击改名
python test_drag.py       # 拖出移除
python test_rhold.py      # 右键状态机（不变式）
```

这些脚本通过程序内置的**诊断消息**驱动（`WM_APP_*`），不需要注入真实输入 ——
本进程装着全局鼠标钩子时，注入输入会互相卡死。

## 一些踩过的坑（写给未来的自己）

- 鼠标/键盘**全局钩子回调里只许做原子操作 + PostMessage**。在回调里注入鼠标事件会自锁。
- **键盘钩子要按需装卸**：菜单关着时它什么都不做，但挂在输入链上会被别人的钩子堵住，
  表现为「打字几秒才出来」。
- **`UpdateLayeredWindow` 必须用预乘 alpha**，而 `blend_over` 产出的是非预乘。
- 32bpp 内存 DIB 上 **GDI 全不可靠**（`Pie` 是实心扇形、`DrawTextW` 的抗锯齿会污染周围），
  所以全部走手写像素 + 内置点阵字库。
- 「取最近一级 mip」这种函数，**先确认数组方向** —— 顺着遍历会永远命中第一项。
