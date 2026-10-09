#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
//! release 构建切到 **Windows GUI 子系统** —— 不切的话双击会弹一个黑色控制台
//! 窗口挂在任务栏上，托盘程序看着像坏了。
//!
//! `cfg_attr(not(debug_assertions))` = **只在 release 关控制台**，
//! `cargo run`（debug）仍保留 stdout —— 启动横幅 / `[dump]` / `[rename]`
//! 等诊断信息都靠 stdout 看。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 径向启动器 v3 —— TheRing 风格（原生 Win32 + 软件渲染）
//!
//! 视觉（照搬 TheRing，参考 design/demos/demo-thering.html 与 design/thering_cover.jpg）：
//!   一整块圆形玻璃盘（盘面跟随壁纸染色）+ 应用图标立在盘面一圈
//!   中心子圆显示悬停应用名 + 项目数；悬停图标带圆角方框选中环
//!   悬停应用的内容以胶囊 chip 从盘沿外侧切向扇出；图标带角标
//! 结构：内环 = 常用应用（每个槽位 = 应用图标）；外环 = 该应用的内容项
//! 生命周期：托盘菜单 + 全局热键 Ctrl+Alt+R 可开启/停止监听
//!   停止后卸载鼠标低级钩子（UnhookWindowsHookEx），零监听开销
//!
//! 操作：按住中键（或侧键）呼出 → 悬停应用展开内容 → 松开执行；Esc / 点其他键取消

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Instant;

use windows::core::PCWSTR;
use windows::core as windows_core;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL};
use windows::Win32::UI::Shell::{AssocQueryStringW, DragAcceptFiles, DragQueryFileW, HDROP, ShellExecuteW, Shell_NotifyIconW, NOTIFYICONDATAW, NIF_MESSAGE, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, ASSOCF_NONE, ASSOCSTR_EXECUTABLE};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Ole::{DROPEFFECT, IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

// ───────────────────────── 几何（对齐 demo-thering.html） ─────────────────────────

/// ══ 全局界面缩放 ══
/// 下面所有尺寸都写成「基准值 × K」。**要整体放大/缩小，只改这一个数。**
/// 基准值是 K = 1.0 时的手感尺寸（盘面半径 178、图标 56 …）。
/// 改完 `cargo build --release`，然后把 exe 拷成 RadialMenu.exe 即可。
const K: f32 = 0.66;
/// 图标单独缩放：比布局缩得少 → 盘子变小了但图标相对更大、更清楚。
/// ⚠️ 这个值算出来的是**底板边长**（不是图标本身，图标还要再除 `TILE_PAD_K`）。
/// 2026-10-07 用户反馈「有点太小了，边框可以大一些，图标也大一点点」
/// → 0.554（底板 31）→ **0.640（底板 35.8，图标 29.9）**。
/// 上限提醒：盘径 `DISC_R_MAX = 145.2`。再往上（KI 0.68 → 盘径 141）会因为顶到上限
/// 被布局**反过来压缩**，12 个应用时盘径反而变小 —— 别再无脑加。
const KI: f32 = 0.640;
const KC: f32 = 1.34;   // chip 内容缩放（胶囊高度 / 图标方块 / 字号）

const CANVAS: i32 = (720.0 * K) as i32; // 渲染画布边长：最远点 = r_out + 段内内容的半长
const CC: f32 = CANVAS as f32 / 2.0;    // 画布中心

// 盘面半径**上限**。⚠️ 220K 时 12 个应用正好顶到，底板被从 35.8 压到 30.8、图标只剩
// 25.6px——肉眼就是「图标又小又糊」（用户 2026-10-08 反馈）。抬到 240K 之后 12 个应用
// 还能保住整块底板：图标 25.6 → 29.1px（+13%），盘径 285 → 316px（窗口 475 装得下）。
// 相邻底板间距仍然 = `GAP_K × 图标`，并没有变挤。
const DISC_R_MAX: f32 = 240.0 * K;
const R_ICON_MIN: f32 = 132.0 * K; // 图标环半径下限（条目少时别缩成一个点）
const ICON_DRAW: f32 = 56.0 * KI;  // 图标绘制尺寸上限（真实图标直接立在盘面上）
/// 相邻图标之间要留的空隙 = 图标边长 × GAP_K。
/// 原实现的目标只是「刚好不相撞」→ 12 个应用时间隙/图标 ≈ 0.28，图标挤成一片看着乱。
/// 对标参考（TheRing）实测间隙/图标 = 0.91；12 个应用在同盘径下取 0.45 已是现实上限。
const GAP_K: f32 = 0.45;
/// 圆角方块在**切向**的最坏投影系数（方块在 45° 处 ≈1.414，圆角半径越大越小）。
/// 按 TILE_RAD_K 的圆角实测 ≈1.22。
const TANG: f32 = 1.22;
/// 环带在图标内外各留的宽度：DISC_R = 图标环 + 图标半宽 + BAND_PAD。
const BAND_PAD: f32 = 28.0; // 图标外沿 → 盘边留白（用户反馈贴边，18 → 24 → 28）
/// 环带底色浓度。**这是白底图标（记事本那页白纸）能在浅色盘面上显形的前提** ——
/// 纯白盘上它整片消失，这也是「很多图标糊/看不见」的一半原因。
const BAND_DARK: f32 = 0.28;
/// 圆角方块：圆角半径 / 边长。参考图那种「整齐」的核心就是这个统一轮廓。
const TILE_RAD_K: f32 = 0.24;
/// **底板边长 / 图标边长** —— 底板的内边距系数。
///
/// 2026-10-07 用户澄清：「不是让你缩小图标吗，我不是让你缩小图标框，
/// **只缩小里面的图标**，这样框与图标就能有点间距了，好看」。
/// 之前那轮改错了：底板和图标**同比缩小**，间距恒为 0，等于白改。
/// 现在底板边长**锁死**（几何 / 盘径 / 图标间隙全都不变），
/// 只把图标按这个系数缩进去 → 底板露出一圈均匀白边。
/// 那圈白边本身就是「统一圆角方块」的一部分 —— 图标再乱，收敛后的轮廓一样。
/// `1.0` = 旧行为（无留白）。当前 `1.20` → 31px 底板里放 25.8px 图标，每边约 2.6px。
const TILE_PAD_K: f32 = 1.20;
/// `normalize_to` 第③步的裁切圆角要跟着缩：纹理是按**图标**尺寸裁的，
/// 不改的话图标放大时圆角会与底板的圆角对不上（露白缝）。
const TILE_TEX_PAD_K: f32 = TILE_PAD_K;
const ICON_MIN: f32 = 30.0;     // 图标缩放下限
/// 图标纹理尺寸：**2 倍超采样**。256→82 用 box 下采样一次到位，
/// 绘制 82→33（k≈2.5）走面积平均 —— 边缘接近像素完美锐利。
const ICON_PX: usize = (ICON_DRAW * 2.05) as usize;
const TILE_R: f32 = 17.0 * K;   // 无图标时的字形方块圆角
const HUB_R: f32 = 66.0 * K;    // 中心头像球半径
/// 外环段（梯形环）的厚度 = 旧胶囊的高度 —— 「甲」方案保持原尺寸不变。
const CHIP_H: f32 = 34.0 * K * KC;
const CHIP_PAD_L: f32 = 6.0 * K * KC;   // 左内边距
// 左端图标边长。⚠️ 不是随手定的：外环带动画的图标就画这么大，19.5px 时肉眼
// 已经偏小发虚（用户 2026-10-08 反馈）。环带厚 `RING_T = 30`，22.1 两侧各留 ~4px。
const CHIP_ICON: f32 = 25.0 * K * KC;
const CHIP_GAP: f32 = 8.0 * K * KC;     // 图标与文字间距
const CHIP_PAD_R: f32 = 14.0 * K * KC;  // 右内边距
const CHIP_MIN_GAP: f32 = 12.0 * K * KC; // 相邻 chip 最小切向间隙(px)
/// 贴弧胶囊的倾角 = **圆环在该处的切线角**（`t` 归一到 (-90°,90°]，见 `chip_layout`）。
/// 胶囊的长轴必须严格平行于圆环 —— 3 点 / 9 点方向就该是竖着的。
/// ⚠️ 之前这里是 `t * 0.82` 再夹到 ±58°，结果 3/9 点只倾斜 58°，看着是「斜着的」，
/// 和相邻的胶囊对不齐。用户的原话：「应该是平行于圆环，为什么斜着？」
/// 外环布局（方案 C）的切换阈值：外环占用跨度 ≤ 此值 → 以图标为中心**贴身扇面**；
/// 超过 → 退回「整圈均布 + 起点锚定」。240° 是「贴身好点」与「围满一圈」的折中。
const CHIP_FAN_MAX: f32 = 240.0;

const MAX_APPS: usize = 12;
const MAX_ITEMS: usize = 12; // 外环可围满一整圈

const WM_APP_SHOW: u32 = WM_APP + 1;
const WM_APP_HOVER: u32 = WM_APP + 2;
const WM_APP_COMMIT: u32 = WM_APP + 3;
const WM_APP_CANCEL: u32 = WM_APP + 4;
const WM_APP_TRAY: u32 = WM_APP + 5;
const WM_APP_DUMP: u32 = WM_APP + 6;
const WM_APP_DEBUGPOS: u32 = WM_APP + 7; // 仅自动化测试：直接设置悬停坐标
const WM_APP_DRAGEVENT: u32 = WM_APP + 11;
const WM_APP_DROPTEST: u32 = WM_APP + 12; // 诊断：给定屏幕坐标，回报 drop_force_app 的解析结果（拖放用不了自动化，只能这样验） // 拖放事件（w=0 进入/1 移动/2 离开/3 落下）
const WM_APP_KEY: u32 = WM_APP + 9;     // 键盘动作（w = 动作码）
const WM_APP_CLICK: u32 = WM_APP + 10;  // 常驻模式下左键点击
const WM_APP_DRAGSTEP: u32 = WM_APP + 13; // 按住应用图标拖动中
const WM_APP_DRAGEND: u32 = WM_APP + 14;  // 拖动松手（盘外 = 请求移除）
const WM_APP_DRAGSIM: u32 = WM_APP + 15;  // 仅测试：驱动拖出移除状态机
const WM_APP_CHAR: u32 = WM_APP + 16;    // 改名模式：一个已转换好的字符（w = codepoint）
const WM_APP_EDITSIM: u32 = WM_APP + 17; // 仅测试：驱动改名状态机
const WM_APP_CHIPPOS: u32 = WM_APP + 22; // 仅测试：回报 0 号胶囊的**屏幕**坐标
                                        // 返回值 = (y << 16) | x
                                        //（单击路径要靠真光标才能触发 update_hover，
                                        //  测试必须知道胶囊在哪才能把光标挪过去）
                                        // w = 阶段（0 双击进入 / 1 追加字符 / 2 退格
                                        //        3 提交 / 4 取消 / 5 提交空名）
const WM_APP_DISMISS_FG: u32 = WM_APP + 18;
// 长按右键呼出菜单时，被「放行」的那个 UP 会让前台程序弹出它自己的右键菜单 ——
// 这里补一次 Esc 关掉它（**窗口消息**，不是输入注入，见处理器处注释）。
const WM_APP_RHOLDSIM: u32 = WM_APP + 19;
/// 只在内部用：叫醒钩子线程去同步键盘钩子的「线程消息」（内容无所谓）。
const WM_APP_HOOK_SYNC: u32 = WM_APP + 21;
// 仅测试：直接驱动右键状态机（`w`：0=喂 RBUTTONDOWN / 1=喂 RBUTTONUP / 2=查询）。
// 为什么不注入真实鼠标事件来测：测试进程注入输入 + 我们装着全局钩子，
// 两边会互相卡死（实测整个测试挂死）。直接喂事件码安全且确定。
// 返回值打包：bit0=本次被消费 / bit1=R_DOWN_AT 非 0 / bit2=按下时菜单开着
//            / bits8..=REINJECT_POSTED 计数 / bits16..=R_DOWN_SEEN 计数
                                        // l = 附加码点（阶段 1 用）

// ── 拖出移除：鼠标钩子只记录「按在哪 / 有没有动」这些原始事实，
//    至于「抓到的是哪个应用」「在不在盘外」全部由 UI 线程判定（它才有 State）。
//    动作用消息回 UI 线程，和渲染同线程，不碰跨线程状态。
static LB_DOWN: AtomicBool = AtomicBool::new(false);   // 左键当前按着
static LB_MOVED: AtomicBool = AtomicBool::new(false);  // 已越过拖动阈值
static DRAG_POSTED: AtomicBool = AtomicBool::new(false); // 队列里只留一条拖动通知（防消息风暴）
static LB_DOWN_X: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static LB_DOWN_Y: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 拖动阈值（px）：按下后移动超过它才算「拖」而不是「点」
const DRAG_THRESH: f32 = 8.0;
/// 确认移除的倒计时（秒）。到点才落盘；期间点「取消」或收起菜单都会放回
const REMOVE_GRACE: f64 = 5.0;
/// 发呆判定（秒）：菜单开着但这么久没有任何输入，头像进入发呆
const IDLE_MOOD_AFTER: f64 = 6.0;
/// 菜单开着、且**没有任何输入**超过这么久就自动收起（非常驻模式）。
/// 安全阀：菜单一旦悬着，所有鼠标事件都得先过一遍钩子判断。
const IDLE_CLOSE_AFTER: f64 = 12.0;

// 键盘动作码
const KA_CLOSE: usize = 1;  // Esc
const KA_CONFIRM: usize = 2; // Enter
const KA_PREV: usize = 3;   // ← ↑ Shift+Tab
const KA_NEXT: usize = 4;   // → ↓ Tab
const KA_PASTE: usize = 5;  // V：把剪贴板内容加入环
const KA_EDIT_COMMIT: usize = 6; // 改名时按 Enter
const KA_EDIT_CANCEL: usize = 7; // 改名时按 Esc
const KA_EDIT_BACK: usize = 8;   // 改名时按 Backspace
const KA_EDIT_DEL: usize = 9;    // 改名时按 Delete

// ───────────────────── 拖放入环（OLE DropTarget） ─────────────────────
// IDropTarget 回调都到达注册线程（我们的 UI 线程），但 State 不能在
// COM 实现里直接碰 —— 回调只写这些静态量并 post 消息，全部状态变更
// 在 host_proc 的 WM_APP_DRAGEVENT 里做（与渲染同线程，无竞争）。
static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false); // OLE 拖放进行中（环上）
static DRAG_SEEN: AtomicBool = AtomicBool::new(false);   // 本次呼出期间见过拖放
static DROP_PT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static DROP_PAYLOAD: std::sync::Mutex<Vec<DropItem>> = std::sync::Mutex::new(Vec::new());
static DROP_MODE_ARMED: AtomicBool = AtomicBool::new(false); // 菜单处于拖放等待态（钩子线程读）

/// 拖放链路诊断日志（exe 目录 draglog.txt）
fn drag_log(msg: &str) {
    use std::io::Write;
    let path = format!("{}\\draglog.txt", exe_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{:?}] {}", std::time::SystemTime::now(), msg);
    }
}

// ───────────────────── 输入卡顿诊断 ─────────────────────
// 背景：用户报告「闲置二十分钟回来鼠标卡 ~30 秒自愈」。要区分三种根因：
//   A. 我们的钩子回调本身慢（页入/逻辑）      → HOOK_SLOW
//   B. 我们整个进程被冻结（待机/换页）        → WATCHDOG_GAP
//   C. 输入在别处被卡（系统唤醒/别的钩子）    → 看门狗心跳连续 + A/B 都没有
// 铁律：钩子回调里**绝不碰文件和锁**（那会反过来制造卡顿）——
// 慢事件只打包进无锁环形缓冲，由看门狗线程每秒落盘 radial_log.txt。

const LOG_RING_SLOTS: usize = 128; // 2 的幂，取模用掩码
static LOG_HEAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LOG_RING: [std::sync::atomic::AtomicU64; LOG_RING_SLOTS] = {
    const Z: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    [Z; LOG_RING_SLOTS]
};
/// 上次钩子回调进入的时刻（GetTickCount 口径，算事件间隔用）
static LAST_HOOK_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

const DK_HOOK_SLOW: u8 = 1; // aux = 鼠标钩子回调耗时 ms（>30ms 才记）
const DK_HOOK_GAP: u8 = 2;  // aux = 距上次鼠标事件的间隔 ms（>30s 才记）
const DK_UI_SLOW: u8 = 3;   // aux = 一帧渲染耗时 ms（>100ms 才记）
const DK_MENU_OPEN: u8 = 4; // 菜单呼出
const DK_MENU_CLOSE: u8 = 5; // 菜单收起
const DK_HOOK_ON: u8 = 6;   // 钩子安装成功
const DK_KEY_SLOW: u8 = 7;  // aux = 键盘钩子回调耗时 ms（>30ms 才记）
const DK_HOOK_LOST: u8 = 8; // 钩子线程意外退场（内核里可能残留死钩子 → 已自愈重启）
const DK_SNAPSHOT: u8 = 9; // 周期性状态快照（aux = 位掩码，见 STATE_* ）
const DK_MOUSE_IDLE: u8 = 10; // 鼠标静默间隔（**5s 阈值**，比 HOOK_GAP 细 6 倍）
const DK_POST_DROP: u8 = 11; // post 失败（窗口已销毁 / 队列满）
const DK_HB: u8 = 12;        // 看门狗心跳（aux = 距上次心跳 ms；证明线程还活着）

// ── 状态快照的位掩码（aux 只有 24 位，够放下 20 个开关）──
const S_MENU_OPEN: u32 = 1 << 0;
const S_LOCKED: u32 = 1 << 1;
const S_DRAG_ACTIVE: u32 = 1 << 2;
const S_DROP_ARMED: u32 = 1 << 3;
const S_LB_DOWN: u32 = 1 << 4;
const S_LB_MOVED: u32 = 1 << 5;
const S_HOVER_POSTED: u32 = 1 << 6;
const S_DRAG_POSTED: u32 = 1 << 7;
const S_EDIT_ARMED: u32 = 1 << 8;
const S_HIDING: u32 = 1 << 9;
const S_ENTERING: u32 = 1 << 10;
const S_CTX_OPEN: u32 = 1 << 11;
const S_PENDING_RM: u32 = 1 << 12;
const S_HAS_FOCUS_APP: u32 = 1 << 13;
const S_HAS_HOVER_CHIP: u32 = 1 << 14;
const S_HAS_HIT_APP: u32 = 1 << 15;

/// 把全部跨线程关键状态打包成 24 位掩码。
/// **为什么必须有它**：出问题时「鼠标点不动」可能源自任何一个原子量卡住
/// （MENU_OPEN 忘了清 / LOCKED 残留 / LB_DOWN 没复位…），只看日志事件推不出来。
/// 有了这份快照，出问题那一刻直接读位就知道是哪一位卡住了。
fn state_bits(hiding: bool, entering: bool, pending_rm: bool,
              focus: bool, hover_chip: bool, hit_app: bool) -> u32 {
    let mut b = 0u32;
    if MENU_OPEN.load(Ordering::Relaxed) { b |= S_MENU_OPEN }
    if LOCKED.load(Ordering::Relaxed) { b |= S_LOCKED }
    if DRAG_ACTIVE.load(Ordering::Relaxed) { b |= S_DRAG_ACTIVE }
    if DROP_MODE_ARMED.load(Ordering::Relaxed) { b |= S_DROP_ARMED }
    if LB_DOWN.load(Ordering::Relaxed) { b |= S_LB_DOWN }
    if LB_MOVED.load(Ordering::Relaxed) { b |= S_LB_MOVED }
    if HOVER_POSTED.load(Ordering::Relaxed) { b |= S_HOVER_POSTED }
    if DRAG_POSTED.load(Ordering::Relaxed) { b |= S_DRAG_POSTED }
    if EDIT_ARMED.load(Ordering::Relaxed) { b |= S_EDIT_ARMED }
    if hiding { b |= S_HIDING }
    if entering { b |= S_ENTERING }
    if CTX_OPEN.load(Ordering::Relaxed) { b |= S_CTX_OPEN }
    if pending_rm { b |= S_PENDING_RM }
    if focus { b |= S_HAS_FOCUS_APP }
    if hover_chip { b |= S_HAS_HOVER_CHIP }
    if hit_app { b |= S_HAS_HIT_APP }
    b
}

/// 打包 [tick:32][kind:8][aux:24]，写进环形缓冲（等待看门狗落盘）
fn diag_push(kind: u8, aux: u32) {
    let tick = unsafe { GetTickCount() };
    let v = ((tick as u64) << 32)
        | ((kind as u64 & 0xff) << 24)
        | (aux as u64 & 0x00ff_ffff);
    let i = LOG_HEAD.fetch_add(1, Ordering::Relaxed) & (LOG_RING_SLOTS as u64 - 1);
    LOG_RING[i as usize].store(v, Ordering::Release);
}

fn diag_kind_name(kind: u8) -> &'static str {
    match kind {
        DK_HOOK_SLOW => "HOOK_SLOW",
        DK_HOOK_GAP => "HOOK_GAP",
        DK_UI_SLOW => "UI_SLOW",
        DK_MENU_OPEN => "MENU_OPEN",
        DK_MENU_CLOSE => "MENU_CLOSE",
        DK_HOOK_ON => "HOOK_ON",
        DK_KEY_SLOW => "KEY_SLOW",
        DK_HOOK_LOST => "HOOK_LOST",
        DK_SNAPSHOT => "SNAPSHOT",
        DK_MOUSE_IDLE => "MOUSE_IDLE",
        DK_POST_DROP => "POST_DROP",
        DK_HB => "HB",
        _ => "?",
    }
}

/// 本地时间戳（GetLocalTime，不引入 chrono）
fn diag_now() -> String {
    #[repr(C)]
    struct ST {
        year: u16, month: u16, dow: u16, day: u16,
        hour: u16, minute: u16, second: u16, ms: u16,
    }
    extern "system" {
        #[link_name = "GetLocalTime"]
        fn get_local_time(st: *mut ST);
    }
    let mut st = ST { year: 0, month: 0, dow: 0, day: 0, hour: 0, minute: 0, second: 0, ms: 0 };
    unsafe { get_local_time(&mut st) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.year, st.month, st.day, st.hour, st.minute, st.second, st.ms)
}

/// 看门狗线程用它落盘（带时间戳）
fn diag_line(path: &str, s: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {}", diag_now(), s);
    }
}

/// 把状态掩码展开成 `MENU_OPEN|LB_DOWN` 这样的可读文本。
/// 出事后翻日志，一眼能看出「哪个开关忘了复位」。
fn state_str(b: u32) -> String {
    const NAMES: [(u32, &str); 16] = [
        (S_MENU_OPEN, "MENU_OPEN"),
        (S_LOCKED, "LOCKED"),
        (S_DRAG_ACTIVE, "DRAG_ACTIVE"),
        (S_DROP_ARMED, "DROP_ARMED"),
        (S_LB_DOWN, "LB_DOWN"),
        (S_LB_MOVED, "LB_MOVED"),
        (S_HOVER_POSTED, "HOVER_POSTED"),
        (S_DRAG_POSTED, "DRAG_POSTED"),
        (S_EDIT_ARMED, "EDIT_ARMED"),
        (S_HIDING, "HIDING"),
        (S_ENTERING, "ENTERING"),
        (S_CTX_OPEN, "CTX_OPEN"),
        (S_PENDING_RM, "PENDING_RM"),
        (S_HAS_FOCUS_APP, "focus"),
        (S_HAS_HOVER_CHIP, "hover_chip"),
        (S_HAS_HIT_APP, "hit_app"),
    ];
    let mut v = Vec::new();
    for (bit, name) in NAMES {
        if b & bit != 0 {
            v.push(name);
        }
    }
    if v.is_empty() {
        "（全清）".to_string()
    } else {
        v.join("|")
    }
}

#[derive(Clone)]
struct DropItem {
    name: String,
    target: String,
    is_url: bool,
    is_dir: bool,
}

fn pack_pt(x: i32, y: i32) -> i64 {
    ((x as i64) << 32) | (y as i64 & 0xffff_ffff)
}
fn unpack_pt(v: i64) -> (i32, i32) {
    ((v >> 32) as i32, v as i32)
}
const HOTKEY_TOGGLE: i32 = 1;
const TIMER_FRAME: usize = 1;
/// 「单击胶囊 → 延迟 `DBL_WAIT_MS` 再执行」用的定时器。
/// 为什么必须延迟：单击与双击在事件层面**无法区分**，只有等第二击来了才知道。
/// 若不等，双击的第一击就会把应用启动（用户 2026-10-07 报的正是这个）。
const TIMER_CLICK: usize = 2;

static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
fn now_s() -> f64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

// ───────────────────── 全局 ─────────────────────

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static RING_HWND: AtomicIsize = AtomicIsize::new(0);
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
// 鼠标坐标由钩子写入原子量，主线程统一消费（避免移动消息风暴）
static HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
static HOOK_TID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static CURSOR_X: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static CURSOR_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static HOVER_POSTED: AtomicBool = AtomicBool::new(false);
static TXT_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// 键盘钩子句柄（与鼠标钩子同线程安装）
static KEY_HOOK_HANDLE: AtomicIsize = AtomicIsize::new(0);
/// 钩子线程自己持有的句柄副本。用于**线程退场时自我摘钩**——
/// 全局 `HOOK_HANDLE` 可能在 `stop_hook()` 里被 `swap(0)` 拿走，
/// 那样线程退出时就已经不知道自己的句柄是什么了。
static MOUSE_HOOK_LOCAL: AtomicIsize = AtomicIsize::new(0);
static KEY_HOOK_LOCAL: AtomicIsize = AtomicIsize::new(0);
/// 键盘钩子「是否该挂着」。**键盘钩子只在菜单开着时才需要**（Esc/Enter/方向键/Tab，
/// 以及改名时的字符输入）；菜单关着时它在回调里什么都不做，却一直挂在系统输入链上。
///
/// 实测（2026-10-08，用户报「程序启动后偶发打字卡顿，有时五六秒才打出来」）：
/// 日志里 `KEY_SLOW` 成簇出现，耗时 1000 / 3000 / 6813 / **10704ms**，
/// 且**全部发生在菜单关着的时候** —— 那时 `key_hook_impl` 只做两次原子读 +
/// `CallNextHookEx`，所以时间全花在**链上别人的钩子**上（Windows 会等下游钩子返回，
/// 超时才把按键放行）。一个「什么都不做」的钩子让每次按键都可能等几秒，纯属自伤。
/// → 改成**按需装卸**：菜单开时装、关时摘。打字路径上就没有我们的钩子了。
static KEY_HOOK_WANT: AtomicBool = AtomicBool::new(false);
/// `set_running` 把「是否在监听」同步到这里，看门狗线程才不用碰 `st`（跨线程）。
static ST_RUNNING_FLAG: AtomicBool = AtomicBool::new(false);
/// `post()` 失败的节流位（见 post 里的说明）
static POST_DROP_LAST: AtomicBool = AtomicBool::new(false);
/// 仅测试： 里强制使用哪个 chip（-1 = 不强制，走真实 hover）。
/// 见使用处注释。**默认 -1，正常路径完全不受影响。**
static FORCE_CHIP: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
/// UI 线程私有状态的**跨线程镜像**。看门狗要定期落状态快照，但不能碰
/// `&mut State`（跨线程可变引用 = UB），所以 UI 线程每帧把几个关键值推到这里。
/// `usize::MAX` 表示 None（与 Option<usize> 的判别一致）。
static SNAP_HIDING: AtomicBool = AtomicBool::new(false);
static SNAP_ENTERING: AtomicBool = AtomicBool::new(false);
static SNAP_PENDING_RM: AtomicBool = AtomicBool::new(false);
static SNAP_FOCUS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
static SNAP_HOVER_CHIP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
static SNAP_HIT_APP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
/// 常驻模式已锁定（跨线程可见，供钩子线程判断左键语义）
static LOCKED: AtomicBool = AtomicBool::new(false);

const MENU_TOGGLE: u16 = 100;
const MENU_OPEN_INI: u16 = 101;
const MENU_RELOAD: u16 = 102;
const MENU_QUIT: u16 = 103;
const MENU_LIQUID: u16 = 104;
const MENU_PERSIST: u16 = 106;
const MENU_SETTINGS: u16 = 107;

// ───────────────────────── 模型 ─────────────────────────

#[derive(Clone)]
struct ItemEntry {
    name: String,
    target: String,
    /// 外环段里的**真图标**（mip 链纹理；与内环同一套提取管线）
    icon: Option<IconTex>,
    /// 配色覆盖（GRADS 下标）；None = 随名字自动（grad_for）。
    /// ini 持久化编码在 target 尾部：`target|N`（'|' 在 URL/路径里不合法，够用）
    color: Option<usize>,
}

#[derive(Clone)]
struct AppEntry {
    name: String,
    target: String,
    icon: Option<IconTex>, // mip 链纹理（加载时按 1/1.4 逐级降采样）
    items: Vec<ItemEntry>, // 外环内容（悬停展开）
    grad: usize,           // 磁贴渐变配色索引
}

/// 待确认移除的内容。设计要点：**请求移除的那一刻就从 `st.apps` 摘下**，
/// 盘面上立刻看不见；之后三分歧——
///   倒计时到点 → 落盘（`commit_remove`）
///   点「取消」 / 收起菜单 → 原位放回（`cancel_remove`）
/// 所以这里必须把完整条目和原槽位都揣着，放回时一个字节都不丢。
enum PendingRm {
    /// 应用图标：pos = 原槽位
    App { pos: usize, entry: AppEntry, deadline: f64 },
    /// 外环胶囊：app = 所属应用槽位，pos = item 原序号
    Chip { app: usize, pos: usize, entry: ItemEntry, deadline: f64 },
}

impl PendingRm {
    fn deadline(&self) -> f64 {
        match self {
            PendingRm::App { deadline, .. } | PendingRm::Chip { deadline, .. } => *deadline,
        }
    }
}

/// 最早到期的时刻 —— 取消按钮的倒计时条用它。
/// 用**最早**的那个（而不是最后加入的那个）才对得上用户看到的事：
/// 进度条走完时，确实有一个真的消失了。
fn rm_soonest(st: &State) -> Option<f64> {
    st.pending_rms.iter().map(|p| p.deadline()).reduce(f64::min)
}

struct State {
    // 数据
    apps: Vec<AppEntry>,
    ini_path: String,
    glass_liquid: bool,
    running: bool,
    /// 常驻模式：松开触发键后菜单保持显示，点击/回车确认，Esc/点空白关闭
    /// （TheRing 1.0.3 的「常驻模式」）
    persistent: bool,
    /// 是否挂钩子（设置页的「监听」开关；从 ini 的 listening= 读）
    listening: bool,
    /// 当前是否已进入常驻锁定态
    locked: bool,
    /// 拖放入环模式：拖放进行中挂起 commit/cancel 语义
    drop_mode: bool,
    /// 拖放悬停时高亮的目标应用（落到谁身上就加进谁的外环）
    drop_target_app: Option<usize>,
    /// 光标当前正悬停的磁贴（高亮严格跟随：移出即灭，与粘性的 active_app 解耦）
    hover_now: Option<usize>,

    // 交互状态
    active_app: Option<usize>, // 锁定的当前应用（移向外环不丢失）
    hover_chip: Option<usize>,
    /// 指针当前是否**真的**落在某个应用图标热区里（区别于粘性的 active_app）。
    /// 点击/松开必须用它判定，否则点屏幕任意位置都会启动 active_app。
    hit_app: Option<usize>,

    // ── 双击胶囊改名 ──
    /// 正在编辑的条目：Some((应用下标, 条目下标))
    editing: Option<(usize, usize)>,
    /// 输入缓冲（UTF-8）。中文经 IME 合成后由 WM_CHAR / WM_IME_CHAR 追加进来
    edit_buf: String,
    /// 光标闪烁的相位（now_s 口径）
    edit_caret: f64,
    /// 首次按键的时刻 —— 用来判「空缓冲时全选 vs 追加」
    edit_started: f64,
    /// 上一次双击的时刻 + 位置（判双击的第二击）
    edit_last_click: f64,
    edit_last_pos: (i32, i32),
    /// 「单击胶囊，等一下看有没有第二击」：排了延迟执行的目标 (app, chip)。
    /// Some = 有一击待执行（`TIMER_CLICK` 到点后才真正 commit）；
    /// 第二击到来时会被清掉并改为改名。
    pending_click: Option<(usize, usize)>,

    // ── 拖出移除 ──
    /// 头像情绪：0 常态 / 1 紧张（正拖到盘外）/ 2 生气（等待确认移除）
    ///           / 3 发呆（长时间无输入）/ 4 疑惑（盘内拖动）/ 5 忙碌（拖放悬停）
    mood: u8,
    mood_t: f64,          // 进入当前情绪的时刻（驱动过渡与抖动相位）
    mood_k: f32,          // 情绪过渡进度 0→1（帧率无关）
    mood_prev: u8,
    /// 短暂情绪的保持期限（now_s 口径；到点回 0）。0 = 无
    mood_hold: f64,
    /// 正被拖出的应用（None = 没在拖）
    drag_app: Option<usize>,
    /// 拖拽中光标下方的**另一个**槽位（松手时与它交换位置）。
    /// 内环/外环分开：跨层换位（应用 ↔ 外环条目）没有意义。
    drag_over_app: Option<usize>,
    drag_over_chip: Option<usize>,
    /// 正被拖出的外环胶囊 (应用槽位, item 序号)
    drag_chip: Option<(usize, usize)>,
    /// 被拖图标跟随的画布坐标
    drag_x: f32,
    drag_y: f32,
    /// 拖出淡出进度 0→1（1 = 完全淡掉）
    drag_fade: f32,
    /// 等待确认移除的内容 —— **已经从盘面摘下**，所以立即不可见；
    /// 超时落盘、点取消或收起菜单都原位放回
    /// 待确认移除的队列。**可以同时有多个** —— 拖走一个之后要能立刻拖下一个，
    /// 不必等它 5 秒倒计时走完（用户 2026-10-09 明确要求）。
    /// 「取消」只作用于**最后加入**的那个；各自按自己的 `deadline` 落盘。
    pending_rms: Vec<PendingRm>,
    /// 取消按钮的出现进度 0→1
    cancel_a: f32,
    /// 最近一次输入（光标移动/点击）的时刻（now_s 口径，发呆判定用）
    last_input: f64,
    /// 上次见到的光标（判定「输入有没有变」）
    cur_x: i32,
    cur_y: i32,

    // 动画
    entering: bool,
    ent0: f64,
    hiding: bool,
    hide0: f64,
    switch0: f64,        // 悬停切换时刻（chip 错峰出现基准）
    // 头像（圆球表情）：呼吸 / 眨眼 / 眼神跟随鼠标
    gaze: [f32; 2],           // 平滑后的注视方向（-1..1）
    blink_q: Vec<(f64, f32)>, // 眨眼关键帧队列 (时刻, 开合度)
    blink_next: f64,          // 下次自动眨眼时刻
    face0: f64,               // 呼吸相位基准
    face_prev: f64,           // 上一帧时刻（求 dt）
    face_n: u32,              // 帧计数（节流：头像 30fps 足够）
    app_anim: [f32; MAX_APPS], // 非悬停压暗渐变
    app_hot: [f32; MAX_APPS], // hover/选中动效渐变（0→1）
    chip_anim: [f32; MAX_ITEMS], // chip 出现渐变
    chip_hot: [f32; MAX_ITEMS], // chip hover 动效渐变（0→1）
    /// dump 模式：把合成壁纸铺在盘面下方，便于离线核对观感
    dump_bg: bool,
    commit_pulse: f32,
    commit_app: usize,
    commit_chip: usize,

    // 窗口
    win_x: i32,
    win_y: i32,

    // 渲染资源
    bg_bits: *mut u8,        // 抓屏+模糊后的桌面背景（CANVAS² BGRA）
    bg_dc: HDC,
    bg_bmp: HBITMAP,
    dbi_bits: *mut u8, // 合成输出（CANVAS² BGRA）
    dbi_dc: HDC,
    dbi_bmp: HBITMAP,
    txt_dc: HDC, // 文字遮罩临时 DIB
    txt_bits: *mut u8,
    txt_bmp: HBITMAP,
    font_name: HFONT,  // 17px 600 中心名称
    font_label: HFONT, // 11px 500 小标签
    font_row: HFONT,   // 13px 500 chip 文字
    font_hint: HFONT,  // 10px 400
    font_head: HFONT,  // 14px 600
    font_cnt: HFONT,   // 12px 400 项目数
    font_glyph: HFONT, // 20px 700 无图标时的首字母
    font_name3: HFONT, // 以下为 3 倍超采样字体（draw_text_rot 旋转文字用）
    font_label3: HFONT,
    font_row3: HFONT,
    font_hint3: HFONT,
    font_head3: HFONT,
    font_cnt3: HFONT,
    font_glyph3: HFONT,


    // ── 设置窗口 ──
    /// 0 亮色 / 1 暗色 / 2 跟随系统（写进 ini 的 dark=）
    theme: i32,
    key_middle: bool,
    key_side: bool,
    key_rhold: bool,
    set_page: usize,
    set_hover: Ctl,
    /// 上一个 hover 的控件（配合 set_hov_a 做淡出）
    set_hover_prev: Ctl,
    /// hover 过渡进度 0→1（新 hover 淡入、旧的用 1-它 淡出）
    set_hov_a: f32,
    /// 切页淡入进度 0→1
    set_page_a: f32,
    /// 三个开关的滑动动画值（0 关 / 1 开）：常驻、监听、开机自启
    sw_a: [f32; 3],
    /// 上一帧时刻（求 dt，帧率无关）
    set_last: Instant,
    /// 是否已注册 WM_MOUSELEAVE 通知
    set_tracking: bool,
    /// 应用页当前展开的应用（usize::MAX = 都没展开）
    set_expanded: usize,
    set_scroll: i32,
    // 设置窗口字体：固定 px，**不跟 K 缩放** —— 设置页是独立窗口，不该跟着环一起缩。
    // 字号 ×SS：文字是在 SS 倍的 DIB 上画的，降采样后才回到标称字号（同时拿到 AA）。
    sf_title: HFONT,
    sf_nav: HFONT,
    sf_body: HFONT,
    sf_small: HFONT,
    sf_bold: HFONT,
    // 设置窗口的超采样缓冲：hi = SS 倍（GDI 画布），lo = 1 倍（降采样结果，直接上屏）
    s_hi_bits: *mut u8,
    s_hi_dc: HDC,
    s_hi_bmp: HBITMAP,
    s_lo_bits: *mut u8,
    s_lo_dc: HDC,
    s_lo_bmp: HBITMAP,
    /// 分层窗口的屏幕位置（UpdateLayeredWindow 要用）
    set_x: i32,
    set_y: i32,
}

impl State {
    fn new() -> Self {
        State {
            apps: Vec::new(),
            ini_path: String::new(),
            glass_liquid: false,
            running: false,
            persistent: true,
            listening: true,
            locked: false,
            drop_mode: false,
            drop_target_app: None,
            hover_now: None,
            active_app: None,
            hover_chip: None,
            hit_app: None,
            editing: None,
            edit_buf: String::new(),
            edit_caret: 0.0,
            edit_started: 0.0,
            edit_last_click: -99.0,
            edit_last_pos: (0, 0),
            pending_click: None,
            mood: 0,
            mood_t: 0.0,
            mood_k: 1.0,
            mood_prev: 0,
            mood_hold: 0.0,
            drag_app: None,
            drag_over_app: None,
            drag_over_chip: None,
            drag_chip: None,
            drag_x: 0.0,
            drag_y: 0.0,
            drag_fade: 0.0,
            pending_rms: Vec::new(),
            cancel_a: 0.0,
            last_input: 0.0,
            cur_x: 0,
            cur_y: 0,
            entering: false,
            ent0: 0.0,
            hiding: false,
            hide0: 0.0,
            switch0: 0.0,
            gaze: [0.0, 0.0],
            blink_q: Vec::new(),
            blink_next: 0.0,
            face0: 0.0,
            face_prev: 0.0,
            face_n: 0,
            app_anim: [0.0; MAX_APPS],
            app_hot: [0.0; MAX_APPS],
            chip_anim: [0.0; MAX_ITEMS],
            chip_hot: [0.0; MAX_ITEMS],
            dump_bg: false,
            commit_pulse: 0.0,
            commit_app: usize::MAX,
            commit_chip: usize::MAX,
            win_x: 0,
            win_y: 0,
            bg_bits: ptr::null_mut(),
            bg_dc: HDC(ptr::null_mut()),
            bg_bmp: HBITMAP(ptr::null_mut()),
            dbi_bits: ptr::null_mut(),
            dbi_dc: HDC(ptr::null_mut()),
            dbi_bmp: HBITMAP(ptr::null_mut()),
            txt_dc: HDC(ptr::null_mut()),
            txt_bits: ptr::null_mut(),
            txt_bmp: HBITMAP(ptr::null_mut()),
            font_name: HFONT(ptr::null_mut()),
            font_label: HFONT(ptr::null_mut()),
            font_row: HFONT(ptr::null_mut()),
            font_hint: HFONT(ptr::null_mut()),
            font_head: HFONT(ptr::null_mut()),
            font_cnt: HFONT(ptr::null_mut()),
            font_glyph: HFONT(ptr::null_mut()),
            font_name3: HFONT(ptr::null_mut()),
            font_label3: HFONT(ptr::null_mut()),
            font_row3: HFONT(ptr::null_mut()),
            font_hint3: HFONT(ptr::null_mut()),
            font_head3: HFONT(ptr::null_mut()),
            font_cnt3: HFONT(ptr::null_mut()),
            font_glyph3: HFONT(ptr::null_mut()),

            // 设置
            theme: 1,
            key_middle: true,
            key_side: true,
            key_rhold: false,
            set_page: 0,
            set_hover: Ctl::None,
            set_hover_prev: Ctl::None,
            set_hov_a: 1.0,
            set_page_a: 1.0,
            sw_a: [1.0, 1.0, 0.0],
            set_last: Instant::now(),
            set_tracking: false,
            set_expanded: usize::MAX,
            set_scroll: 0,
            sf_title: HFONT(ptr::null_mut()),
            sf_nav: HFONT(ptr::null_mut()),
            sf_body: HFONT(ptr::null_mut()),
            sf_small: HFONT(ptr::null_mut()),
            sf_bold: HFONT(ptr::null_mut()),
            s_hi_bits: ptr::null_mut(),
            s_hi_dc: HDC(ptr::null_mut()),
            s_hi_bmp: HBITMAP(ptr::null_mut()),
            s_lo_bits: ptr::null_mut(),
            s_lo_dc: HDC(ptr::null_mut()),
            s_lo_bmp: HBITMAP(ptr::null_mut()),
            set_x: 0,
            set_y: 0,
        }
    }
}

// 磁贴配色：全部为中性灰阶（无彩色）。
// 仅用于「没有真实图标」的字母磁贴，以及外环 chip 左端的字母方块。
// 真实 app 图标是彩色内容，不在这里染灰。
const GRADS: [((u8, u8, u8), (u8, u8, u8)); 14] = [
    ((62, 62, 66), (62, 62, 66)),     // 0
    ((84, 84, 89), (84, 84, 89)),     // 1
    ((44, 44, 48), (44, 44, 48)),     // 2
    ((110, 110, 115), (110, 110, 115)), // 3
    ((92, 92, 97), (92, 92, 97)),     // 4
    ((70, 70, 74), (70, 70, 74)),     // 5
    ((122, 122, 127), (122, 122, 127)), // 6
    ((54, 54, 58), (54, 54, 58)),     // 7
    ((86, 86, 91), (86, 86, 91)),     // 8  默认
    ((122, 122, 127), (122, 122, 127)), // 9
    ((98, 98, 103), (98, 98, 103)),   // 10
    ((84, 84, 89), (84, 84, 89)),     // 11
    ((98, 98, 103), (98, 98, 103)),   // 12
    ((68, 68, 72), (68, 68, 72)),     // 13
];

/// ── 配色：黑白灰（无彩色）──
/// 亮色：浅灰底 + 白纸 + 近黑墨；强调色直接用墨色（不再是蓝色）
const P_WALL: (u8, u8, u8) = (238, 238, 240); // #EEEEF0
const P_WALL2: (u8, u8, u8) = (227, 227, 230); // #E3E3E6
const P_PAPER: (u8, u8, u8) = (255, 255, 255);
const P_INK: (u8, u8, u8) = (28, 28, 30); // #1C1C1E
const P_ACTION: (u8, u8, u8) = (28, 28, 30); // 强调 = 墨色
/// 暗色：中性深灰（不带蓝调）
const D_WALL: (u8, u8, u8) = (22, 22, 24); // #161618
const D_WALL2: (u8, u8, u8) = (15, 15, 17); // #0F0F11
const D_PAPER: (u8, u8, u8) = (36, 36, 39); // #242427
const D_INK: (u8, u8, u8) = (236, 236, 238); // #ECECEE
const D_ACTION: (u8, u8, u8) = (236, 236, 238); // 强调 = 墨色

#[derive(Clone, Copy)]
struct Pal {
    wall: (u8, u8, u8),
    wall2: (u8, u8, u8),
    paper: (u8, u8, u8),
    ink: (u8, u8, u8),
    action: (u8, u8, u8),
    /// control-edge = color-mix(ink 62%, paper)
    edge: (u8, u8, u8),
}

fn mixc(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

fn pal_of(dark: bool) -> Pal {
    if dark {
        let edge = mixc(D_INK, D_PAPER, 0.38);
        Pal { wall: D_WALL, wall2: D_WALL2, paper: D_PAPER, ink: D_INK, action: D_ACTION, edge }
    } else {
        let edge = mixc(P_INK, P_PAPER, 0.38);
        Pal { wall: P_WALL, wall2: P_WALL2, paper: P_PAPER, ink: P_INK, action: P_ACTION, edge }
    }
}

/// 投影色：中性近黑（不带蓝调）
const SHADOW: (u8, u8, u8) = (16, 16, 18);

/** 覆盖率：隐式边缘 1px 抗锯齿（sdf<0 在内部） */
#[inline]
fn cov1(sdf: f32) -> f32 {
    (0.5 - sdf).clamp(0.0, 1.0)
}

// ───────────────────── 基础工具 ─────────────────────

/// 4x 超采样覆盖率的子样本偏移
#[allow(dead_code)]
const SUB: [(f32, f32); 4] = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

#[inline]
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// easeOutCubic
#[inline]
fn ease_out(v: f32) -> f32 {
    1.0 - (1.0 - v).powi(3)
}

/// easeOutBack（入场「弹出」手感）
#[inline]
fn ease_out_back(v: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    1.0 + C3 * (v - 1.0).powi(3) + C1 * (v - 1.0).powi(2)
}

/// source-over 合成（按 alpha 归一化，非预乘）
#[inline]
fn blend_over(buf: &mut [u8], o: usize, cr: f32, cg: f32, cb: f32, a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let da = buf[o + 3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0001 {
        return;
    }
    let mix = |src: f32, dst: f32| ((src * a + dst * da * (1.0 - a)) / oa).clamp(0.0, 255.0);
    buf[o] = mix(cb, buf[o] as f32) as u8;
    buf[o + 1] = mix(cg, buf[o + 1] as f32) as u8;
    buf[o + 2] = mix(cr, buf[o + 2] as f32) as u8;
    buf[o + 3] = (oa * 255.0) as u8;
}

/// 圆角矩形 SDF（**内部为正**）
fn sdf_rounded_rect(dx: f32, dy: f32, half_w: f32, half_h: f32, r: f32) -> f32 {
    let qx = dx.abs() - half_w + r;
    let qy = dy.abs() - half_h + r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    r - (ox * ox + oy * oy).sqrt() - qx.min(0.0).max(qy.min(0.0))
}

/// 圆 SDF（**内部为正**）
#[allow(dead_code)]
#[inline]
fn circle_sdf(dx: f32, dy: f32, r: f32) -> f32 {
    r - (dx * dx + dy * dy).sqrt()
}

/// 4x 超采样覆盖率
#[allow(dead_code)]
fn cov4(f: impl Fn(f32, f32) -> f32, dx: f32, dy: f32) -> f32 {
    let mut c = 0u32;
    for (ox, oy) in SUB {
        if f(dx + ox, dy + oy) > 0.0 {
            c += 1;
        }
    }
    c as f32 / 4.0
}

/// 扁平圆：**向下偏移的双层模糊投影** → 描边（可半透明）→ 填充。全部纯像素。
///
/// 投影必须「偏移 + 模糊」，不能做成以圆心为中心的对�称光晕 ——
/// 那样等于给盘面套了一圈半透明的黑边，看起来又粗又脏（上一版的「大黑边」）。
#[allow(clippy::too_many_arguments)]
unsafe fn flat_circle(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    r: f32,
    fill: (u8, u8, u8),
    bw: f32,
    border: (u8, u8, u8),
    border_a: f32,
    sh_dy: f32,
    sh_a: f32,
    sh_blur: f32,
    am: f32,
) {
    let ext = r + bw + sh_dy.abs() + sh_blur * 1.7 + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    let w = CANVAS as usize;
    let sh = SHADOW;
    let rb = r + bw;
    // 投影扩散层最多只能覆盖到 (r - bw - |sh_dy|*1.55 - sh_blur*1.7 - 2)，
    // 深于这条线的像素**必然**落在填充区内部，算了也会被不透明填充盖掉。
    // 提前跳出可让整块盘的投影开销从 ~10 万像素降到 ~2 万。
    let sh_in = bw + sh_dy.abs() * 1.55 + sh_blur * 1.7 + 2.0;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt() - r;
            let o = (py * w + px) * 4;
            if sh_a > 0.001 && sh_blur > 0.1 && d > -sh_in {
                // 贴身层：偏移 1.0×
                let s1 = (dx * dx + (dy - sh_dy).powi(2)).sqrt() - rb;
                let b1 = sh_blur * 0.55;
                if s1 > -2.0 && s1 < b1 {
                    let t = 1.0 - (s1.max(0.0) / b1);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32, t * t * sh_a * am);
                }
                // 扩散层：偏移 1.55×，更大更淡（模拟大半径高斯）
                let s2 = (dx * dx + (dy - sh_dy * 1.55).powi(2)).sqrt() - rb;
                let b2 = sh_blur * 1.7;
                if s2 > -2.0 && s2 < b2 {
                    let t = 1.0 - (s2.max(0.0) / b2);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32, t * t * sh_a * 0.62 * am);
                }
            }
            let cb = cov1(d - bw) * border_a;
            if cb > 0.0 {
                blend_over(buf, o, border.0 as f32, border.1 as f32, border.2 as f32, cb * am);
            }
            let cf = cov1(d);
            if cf > 0.0 {
                blend_over(buf, o, fill.0 as f32, fill.1 as f32, fill.2 as f32, cf * am);
            }
        }
    }
}

/// 扁平圆角矩形：同 flat_circle
#[allow(clippy::too_many_arguments)]
unsafe fn flat_round_rect(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    half_w: f32,
    half_h: f32,
    rad: f32,
    fill: (u8, u8, u8),
    bw: f32,
    border: (u8, u8, u8),
    border_a: f32,
    sh_dy: f32,
    sh_a: f32,
    sh_blur: f32,
    am: f32,
) {
    let ext = half_w.max(half_h) + bw + sh_dy.abs() + sh_blur * 1.7 + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    let w = CANVAS as usize;
    let sh = SHADOW;
    // 注意：sdf_rounded_rect 是「内部为正」，统一取负
    let sdf = |x: f32, y: f32| -sdf_rounded_rect(x, y, half_w, half_h, rad);
    // 投影早退：深于形状内部的像素算出来的投影必然被不透明填充盖掉
    let sh_in = bw + sh_dy.abs() * 1.55 + sh_blur * 1.7 + 2.0;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let d = sdf(dx, dy);
            let o = (py * w + px) * 4;
            if sh_a > 0.001 && sh_blur > 0.1 && d > -sh_in {
                let s1 = sdf(dx, dy - sh_dy) - bw;
                let b1 = sh_blur * 0.55;
                if s1 > -2.0 && s1 < b1 {
                    let t = 1.0 - (s1.max(0.0) / b1);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32, t * t * sh_a * am);
                }
                let s2 = sdf(dx, dy - sh_dy * 1.55) - bw;
                let b2 = sh_blur * 1.7;
                if s2 > -2.0 && s2 < b2 {
                    let t = 1.0 - (s2.max(0.0) / b2);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32, t * t * sh_a * 0.62 * am);
                }
            }
            let cb = cov1(d - bw) * border_a;
            if cb > 0.0 {
                blend_over(buf, o, border.0 as f32, border.1 as f32, border.2 as f32, cb * am);
            }
            let cf = cov1(d);
            if cf > 0.0 {
                blend_over(buf, o, fill.0 as f32, fill.1 as f32, fill.2 as f32, cf * am);
            }
        }
    }
}

#[allow(dead_code)]
/// 球体径向渐变：光源在左上，中心亮 → 边缘暗，做出「绒球」的体积感。
/// 参数比例照搬 my-pet 的 `radialGradient cx=38% cy=32% r=75%`。
#[allow(clippy::too_many_arguments)]
unsafe fn ball_gradient(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    hi: (u8, u8, u8),
    mid: (u8, u8, u8),
    lo: (u8, u8, u8),
) {
    if rx <= 0.5 || ry <= 0.5 {
        return;
    }
    let w = CANVAS as usize;
    let lx = cx - 0.24 * rx; // 光源 x（对应 38%）
    let ly = cy - 0.36 * ry; // 光源 y（对应 32%）
    let rr = 1.5 * rx.max(ry);
    let ext = rx.max(ry) + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    let px_scale = rx.min(ry).max(0.5);
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let q = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
            let cov = cov1((q - 1.0) * px_scale);
            if cov <= 0.0 {
                continue;
            }
            let gx = (px as f32 + 0.5 - lx) / rr;
            let gy = (py as f32 + 0.5 - ly) / rr;
            let t = (gx * gx + gy * gy).sqrt().clamp(0.0, 1.0);
            let col = if t < 0.62 {
                mixc(hi, mid, t / 0.62)
            } else {
                mixc(mid, lo, (t - 0.62) / 0.38)
            };
            blend_over(buf, (py * w + px) * 4, col.0 as f32, col.1 as f32, col.2 as f32, cov);
        }
    }
}

/// 旋转的圆角矩形（绒球的眼睛是「微微倾斜的细长条」）。
#[allow(clippy::too_many_arguments)]
unsafe fn rot_rrect(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
    rad: f32,
    rot: f32,
    col: (u8, u8, u8),
    a: f32,
) {
    if hw <= 0.05 || hh <= 0.05 || a <= 0.002 {
        return;
    }
    let w = CANVAS as usize;
    let (sn, cs) = rot.sin_cos();
    let ext = hw.max(hh) + 3.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let lx = dx * cs + dy * sn;
            let ly = -dx * sn + dy * cs;
            // sdf_rounded_rect 内部为正 → 取负得「内部为负」
            let cov = cov1(-sdf_rounded_rect(lx, ly, hw, hh, rad));
            if cov <= 0.0 {
                continue;
            }
            blend_over(buf, (py * w + px) * 4, col.0 as f32, col.1 as f32, col.2 as f32, cov * a);
        }
    }
}

/// 旋转圆角矩形，支持描边与「向下偏移」的柔和投影。
///
/// 为什么需要它：外环 chip 要沿弧线**切线**排布（贴弧胶囊），
/// 轴对齐的 flat_round_rect 画出来还是「水平浮在旁边的药丸」。
/// 投影方向固定在**屏幕**竖直方向（不是局部坐标），否则 chip 转到侧面时
/// 影子会朝切线方向偏，看起来像悬空。
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)] // 外环改梯形段后暂无调用者；它是有用的「旋转圆角矩形 + 投影」原语，留着
unsafe fn rot_rrect_ex(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
    rad: f32,
    rot: f32,
    fill: (u8, u8, u8),
    bw: f32,
    border: (u8, u8, u8),
    border_a: f32,
    sh_dy: f32,
    sh_a: f32,
    sh_blur: f32,
    am: f32,
) {
    let w = CANVAS as usize;
    let (sn, cs) = rot.sin_cos();
    // 屏幕竖直偏移换算到局部坐标
    let (shx, shy) = (sh_dy * sn, sh_dy * cs);
    let ext = hw.max(hh) + bw + sh_dy.abs() + sh_blur * 1.7 + 3.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    let sh = SHADOW;
    // 投影早退：深于形状内部的像素算出来的投影必然被不透明填充盖掉
    let sh_in = bw + sh_dy.abs() * 1.55 + sh_blur * 1.7 + 2.0;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            // 旋回局部坐标：+x 沿切线，−y 朝外
            let lx = dx * cs + dy * sn;
            let ly = -dx * sn + dy * cs;
            let d = -sdf_rounded_rect(lx, ly, hw, hh, rad);
            let o = (py * w + px) * 4;
            if sh_a > 0.001 && sh_blur > 0.1 && d > -sh_in {
                let s1 = -sdf_rounded_rect(lx - shx, ly - shy, hw, hh, rad) - bw;
                let b1 = sh_blur * 0.55;
                if s1 > -2.0 && s1 < b1 {
                    let t = 1.0 - (s1.max(0.0) / b1);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32, t * t * sh_a * am);
                }
                let s2 = -sdf_rounded_rect(lx - shx * 1.55, ly - shy * 1.55, hw, hh, rad) - bw;
                let b2 = sh_blur * 1.7;
                if s2 > -2.0 && s2 < b2 {
                    let t = 1.0 - (s2.max(0.0) / b2);
                    blend_over(buf, o, sh.0 as f32, sh.1 as f32, sh.2 as f32,
                        t * t * sh_a * 0.62 * am);
                }
            }
            let cb = cov1(d - bw) * border_a;
            if cb > 0.0 {
                blend_over(buf, o, border.0 as f32, border.1 as f32, border.2 as f32, cb * am);
            }
            let cf = cov1(d);
            if cf > 0.0 {
                blend_over(buf, o, fill.0 as f32, fill.1 as f32, fill.2 as f32, cf * am);
            }
        }
    }
}

#[allow(dead_code)]
/// 椭圆（可描边、可半透明填充）。用归一化距离近似 SDF，边缘 1px 抗锯齿。
/// 头像的眼睛需要「垂直压扁」做眨眼，所以不能只用圆。
#[allow(clippy::too_many_arguments)]
unsafe fn flat_ellipse(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    fill: (u8, u8, u8),
    fill_a: f32,
    bw: f32,
    border: (u8, u8, u8),
    border_a: f32,
    am: f32,
) {
    if rx <= 0.05 || ry <= 0.05 {
        return;
    }
    let w = CANVAS as usize;
    let ext = rx.max(ry) + bw + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    // 把归一化半径差换算回像素量级，保证抗锯齿带宽稳定
    let px_scale = rx.min(ry).max(0.5);
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let q = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt();
            let d = (q - 1.0) * px_scale; // < 0 在内部
            let o = (py * w + px) * 4;
            let cb = cov1(d - bw) * border_a;
            if cb > 0.0 {
                blend_over(buf, o, border.0 as f32, border.1 as f32, border.2 as f32, cb * am);
            }
            let cf = cov1(d) * fill_a;
            if cf > 0.0 {
                blend_over(buf, o, fill.0 as f32, fill.1 as f32, fill.2 as f32, cf * am);
            }
        }
    }
}

/// 盘面垂直渐变：上亮下暗，让白盘读作「一块实体纸」而不是一坨纯色。
/// 幅度必须很小（≤6%），大了就变成「渐变球」，反而不高级。
unsafe fn disc_gradient(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    r: f32,
    top: (u8, u8, u8),
    bot: (u8, u8, u8),
) {
    let w = CANVAS as usize;
    let y0 = (cy - r).max(0.0) as usize;
    let y1 = ((cy + r).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - r).max(0.0) as usize;
    let x1 = ((cx + r).min(CANVAS as f32 - 1.0)) as usize;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let c = cov1((dx * dx + dy * dy).sqrt() - r);
            if c <= 0.0 {
                continue;
            }
            let t = ((dy + r) / (2.0 * r).max(1.0)).clamp(0.0, 1.0);
            let col = (
                lerp(top.0 as f32, bot.0 as f32, t),
                lerp(top.1 as f32, bot.1 as f32, t),
                lerp(top.2 as f32, bot.2 as f32, t),
            );
            blend_over(buf, (py * w + px) * 4, col.0, col.1, col.2, c);
        }
    }
}

/// 圆环带（annulus）：给盘面一条「图标轨道」，让图标看起来是被安置的，
/// 而不是散落在一块白饼上。这是盘面从「一片空白」变成「有结构」的关键。
unsafe fn ring_band(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    r0: f32,
    r1: f32,
    fill: (u8, u8, u8),
    a: f32,
) {
    if a <= 0.002 || r1 <= r0 {
        return;
    }
    let w = CANVAS as usize;
    let ext = r1 + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt();
            // 到环带最近边的距离（内部为正）
            let sdf = (d - r0).min(r1 - d);
            let c = cov1(-sdf);
            if c <= 0.0 {
                continue;
            }
            blend_over(buf, (py * w + px) * 4, fill.0 as f32, fill.1 as f32, fill.2 as f32, c * a);
        }
    }
}

#[allow(dead_code)]
/// 柔和圆角矩形色块（边缘 blur px 渐隐，无描边）
#[allow(clippy::too_many_arguments)]
unsafe fn soft_rrect(
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
    rad: f32,
    col: (u8, u8, u8),
    a: f32,
    blur: f32,
) {
    if a <= 0.002 {
        return;
    }
    let w = CANVAS as usize;
    let ext = hw.max(hh) + blur + 2.0;
    let y0 = (cy - ext).max(0.0) as usize;
    let y1 = ((cy + ext).min(CANVAS as f32 - 1.0)) as usize;
    let x0 = (cx - ext).max(0.0) as usize;
    let x1 = ((cx + ext).min(CANVAS as f32 - 1.0)) as usize;
    let b = blur.max(0.35);
    for py in y0..=y1 {
        let dy = py as f32 + 0.5 - cy;
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            // sdf_rounded_rect 内部为正 → 取负得「内部为负」
            let d = -sdf_rounded_rect(dx, dy, hw, hh, rad);
            if d >= b {
                continue;
            }
            let t = 1.0 - (d.max(0.0) / b);
            blend_over(buf, (py * w + px) * 4, col.0 as f32, col.1 as f32, col.2 as f32, t * t * a);
        }
    }
}

unsafe fn draw_scene(st: &mut State, buf: &mut [u8]) {
    let w = CANVAS as usize;
    let now = now_s() as f64;
    buf.fill(0);
    let n = st.apps.len();
    if n == 0 {
        return;
    }
    let (r_icon, icon_draw) = ring_layout(n);
    let disc_r0 = disc_r(n);        // 盘面半径（随条目数自适应）
    let band_r0 = band_in(n);       // 环带内沿

    let pal = pal_of(st.glass_liquid);
    // 精致化的关键：描边一律用「hairline」，不用纯 ink 粗描边
    let hair = mixc(pal.paper, pal.ink, 0.16);

    if st.dump_bg && !st.bg_bits.is_null() {
        let src = std::slice::from_raw_parts(st.bg_bits, w * CANVAS as usize * 4);
        buf.copy_from_slice(src);
    }

    let pop_k = if st.entering || now - st.ent0 < 0.32 {
        clamp01(((now - st.ent0) / 0.32) as f32)
    } else {
        1.0
    };
    let pop = lerp(0.62, 1.0, ease_out(pop_k));
    let disc_r = disc_r0 * pop;

    // ── 1. 盘面：白纸 + 1.5px 细描边 + 向下偏移的贴身柔和投影 ──
    // 投影刻意收紧（dy 6 / blur 18 → 最远只到 r+30），避免在深色桌面上
    // 显出一圈「半透明色带」。真正的高级感来自干净的边界，不是大范围光晕。
    flat_circle(
        buf,
        CC,
        CC,
        disc_r,
        pal.paper,
        1.7 * K,
        mixc(pal.paper, pal.ink, 0.40),
        1.0,
        6.0 * K,
        0.11,
        18.0 * K,
        1.0,
    );
    // 拖放感知指示：drop 模式下盘沿亮一圈绿环（证明 OLE 已找到我们）
    if st.drop_mode {
        let rim_r = disc_r0 * pop;
        for py in 0..CANVAS as usize {
            let dy = py as f32 + 0.5 - CC;
            if dy.abs() > rim_r + 2.0 {
                continue;
            }
            let sx = (rim_r * rim_r - dy * dy).sqrt().max(0.0);
            for px in (CC - sx - 2.0).max(0.0) as usize..((CC + sx + 2.0).min(CANVAS as f32 - 0.5) as usize + 1) {
                let dx = px as f32 + 0.5 - CC;
                let d = circle_sdf(dx, dy, rim_r);
                if d.abs() <= 1.6 {
                    let a = (1.0 - d.abs() / 1.6) * 0.55;
                    let o = (py * w + px) * 4;
                    if buf[o + 3] > 0 {
                        blend_over(buf, o, 60.0, 200.0, 90.0, a);
                    }
                }
            }
        }
    }

    // 极淡垂直渐变：上纯白 → 下 4% 灰，给出「纸质实体」的厚度
    disc_gradient(
        buf,
        CC,
        CC,
        disc_r - 1.0,
        pal.paper,
        mixc(pal.paper, pal.ink, 0.055),
    );
    // 图标环带：一条中性灰带，图标坐在带上。
    // **不能省** —— 记事本那种「白纸图标」在纯白盘面上会整片消失，
    // 参考图（TheRing）用同一条带子解决对比度；带子同时给图标划出统一的轨道。
    ring_band(
        buf,
        CC,
        CC,
        band_r0 * pop,
        (r_icon + icon_draw * 0.5 + BAND_PAD) * pop,
        mixc(pal.paper, pal.ink, BAND_DARK),
        1.0,
    );

    // ── 2. 中心头像：GrokBot 式表情球 ──
    //    参考 github.com/zhulin025/LaoA-GrokBot（MIT）的官方几何：
    //    **一个纯色圆球 + 两只药丸状眼睛**，眼睛贴在球面上滑动、
    //    靠近轮廓时按 cos(经度) 透视收窄。没有耳朵、没有腮红、没有嘴 ——
    //    极简才是它「高级」的原因（我前两版加的耳朵/腮红/渐变球都是错的）。
    //    配色按项目既定：球 = ink（近黑）、眼睛 = paper（白），深色主题自动反相。
    //    动效：呼吸 3.2s、眨眼关键帧、注视帧率无关指数平滑。
    {
        let ft = (now - st.face0) as f32;
        let open = blink_open(&st.blink_q, now).clamp(0.0, 1.45);

        // ── 情绪（拖出移除）────────────────────────────────────────────
        // 参数逐条抄自 my-pet `common/ui/avatar/avatar_data.py` 的 EMOTION_SEED，
        // 单位从它的脸半径 114.27px 归一到本项目的球半径（px2r）。
        //
        //  紧张 1 = 慌张 id 17「圆睁眼环高频轮换，目光乱晃，整球细颤」
        //  生气 2 = 生气 id 21「怒目圆睁，身体细微发抖」（不涨红，见下）
        //  发呆 3 = 发呆 id 04「双眼各望各的，缓慢游移」
        //  疑惑 4 = 疑惑 id 11「头微倾，一眼放大一眼收小」
        //  忙碌 5 = 处理中忙碌 id 32「目光小幅循环往复」
        //
        // ⚠️ 抖动幅度**放大 2.5 倍**：my-pet 的脸直径 228px，本项目的球只有 87px，
        //    同样的相对位移在小球上只有 0.7px，肉眼读不出来。
        // ⚠️ my-pet 的眼形靠 23 组眼环形状切换，本项目只有药丸眼，
        //    用「整体放大 / 单眼缩放 / 倾角」近似。
        struct MoodPose {
            body_x: f32,             // 整球水平位移（R 单位，已含放大）
            gaze_x: f32,             // 双眼共享注视偏移（gaze 单位）
            gaze_y: f32,
            eye_l: (f32, f32, f32),  // 左眼 (dx R, dy R, 缩放)
            eye_r: (f32, f32, f32),  // 右眼
            tilt: f32,               // 双眼整体倾角（弧度，「头微倾」的近似）
            esc: f32,                // 双眼整体放大
            breathe: f32,            // 呼吸幅度（my-pet body.breathe 原值）
        }
        impl MoodPose {
            fn calm() -> Self {
                MoodPose {
                    body_x: 0.0, gaze_x: 0.0, gaze_y: 0.0,
                    eye_l: (0.0, 0.0, 1.0), eye_r: (0.0, 0.0, 1.0),
                    tilt: 0.0, esc: 1.0, breathe: 0.024,
                }
            }
        }
        const PET_FACE_R: f64 = 114.27;
        const JIT_AMP: f64 = 2.5;
        let px2r = |v: f64| (v / PET_FACE_R * JIT_AMP) as f32;
        let tau = std::f64::consts::TAU;
        let mk = st.mood_k.min(1.0);
        let mt = (now - st.mood_t) as f64 * 1000.0;
        let pose = match st.mood {
            1 => MoodPose {
                body_x: jitter(mt, 7.0, px2r(1.9464)),
                gaze_x: jitter(mt + 37.0, 10.0, px2r(6.0)),
                gaze_y: jitter(mt + 91.0, 11.0, px2r(3.0)),
                esc: 1.22,
                breathe: 0.0057,
                ..MoodPose::calm()
            },
            2 => MoodPose {
                body_x: jitter(mt + 53.0, 6.0, px2r(1.0467)),
                gaze_y: 0.30,   // 怒目：目光下压
                // 脸色涨红会打破项目的「全中性灰阶」铁律，所以**默认不涨红**，
                // 只用眼睛放大 + 目光下压表达怒意。要涨红改这里即可（#E4574A）。
                esc: 1.18,
                breathe: 0.0032,
                ..MoodPose::calm()
            },
            3 => MoodPose {
                // 发呆 04：左眼望左下、右眼望右上，各想各的，再叠一层极慢的游移
                gaze_x: px2r(3.0) * ((mt / 5614.0 * tau).sin()) as f32,
                eye_l: (px2r(-4.0), px2r(2.0), 1.0),
                eye_r: (px2r(5.0), px2r(-1.0), 1.0),
                tilt: (-3.0f32).to_radians(),
                breathe: 0.0103,
                ..MoodPose::calm()
            },
            4 => MoodPose {
                // 疑惑 11：左眼抬高放大、右眼放低收小往一边瞟，头微倾 + 轻晃
                eye_l: (0.0, px2r(-5.0), 1.087),
                eye_r: (px2r(3.0), px2r(3.0), 0.881),
                tilt: (-6.0f32).to_radians()
                    + (1.74f32 * ((mt / 3392.0 * tau).sin()) as f32).to_radians(),
                breathe: 0.0078,
                ..MoodPose::calm()
            },
            5 => MoodPose {
                // 忙碌 32：目光小幅循环往复（lookX amp 6 / lookY amp 5，两轴不同频）
                gaze_x: px2r(6.0) * ((mt / 1495.0 * tau).sin()) as f32,
                gaze_y: px2r(5.0) * ((mt / 1058.0 * tau + 4.578).sin()) as f32,
                breathe: 0.0078,
                ..MoodPose::calm()
            },
            _ => MoodPose::calm(),
        };
        let m_bx = pose.body_x * mk;
        let esc = 1.0 + (pose.esc - 1.0) * mk;
        let br_amp = pose.breathe;
        let br = (ft / 3.2 * std::f32::consts::TAU).sin() * br_amp;
        // 取消按钮出现时头像缩 12% 让位（与按钮同步淡入，见 3b / rm_btn_center）
        let fr = HUB_R * pop * (1.0 + br) * (1.0 - RM_BALL_SHRINK * st.cancel_a.clamp(0.0, 1.0));

        // 注视 + 常驻微漂移（永不完全静止）
        let gx = ((st.gaze[0] + 0.05 * ((0.42 * ft).sin() + 0.35 * (1.0 * ft).sin()))
            + pose.gaze_x * 0.55 * mk)
            .clamp(-1.25, 1.25);
        let gy = ((st.gaze[1] + 0.04 * (0.58 * ft).sin()) + pose.gaze_y * 0.55 * mk)
            .clamp(-1.25, 1.25);
        // 整球随注视轻微位移（参考实现里 body 也有 x/y 偏移）
        let bcx = CC + gx * HUB_R * 0.030;
        let bcy = CC + gy * HUB_R * 0.030;

        // ① 球体：扁平实心（无渐变、无描边）+ 向下偏移的柔和投影
        flat_circle(buf, bcx + m_bx * fr, bcy, fr, pal.ink, 0.0, pal.ink, 0.0,
            2.6 * K, 0.16, 9.0 * K, 1.0);

        // ② 眼睛：药丸，贴着球面滑动
        //    参考的定位模型：x = R_ORB·sin(经度)，透视 = cos(经度)/cos(基准经度)
        const R_ORB: f32 = 0.92;    // 眼环轨道半径（相对球半径）
        const EYE_DX: f32 = 0.266;  // 眼睛基准横向偏移（相对球半径）
        const EYE_W: f32 = 0.156;   // 眼睛半宽
        const EYE_H: f32 = 0.276;   // 眼睛半高（全睁）
        const EYE_DY: f32 = -0.055; // 基准纵向偏移（略高于球心）
        let lon0 = (EYE_DX / R_ORB).clamp(-1.0, 1.0).asin();
        let turn = gx * 0.30;       // 眼球绕球心转过的弧度
        for side in [-1.0f32, 1.0f32] {
            // 发呆/疑惑需要两只眼「各动各的」：左眼 side=-1，右眼 side=+1
            let (edx, edy, escl) = if side < 0.0 { pose.eye_l } else { pose.eye_r };
            let lon = side * lon0 + turn;
            let persp = (lon.cos() / lon0.cos()).clamp(0.55, 1.50);
            let ex = bcx + m_bx * fr + R_ORB * lon.sin() * fr + edx * fr * mk;
            let ey = bcy + (EYE_DY + gy * 0.20) * fr + edy * fr * mk;
            let escl = 1.0 + (escl - 1.0) * mk;
            let ehw = EYE_W * fr * persp * esc * escl;
            let ehh = EYE_H * fr * open * esc * escl;
            if ehh <= 0.7 {
                continue;
            }
            // 圆角＝半宽 → 胶囊；倾斜角跟随注视方向 + 情绪的「头微倾」
            rot_rrect(buf, ex, ey, ehw, ehh, ehw.min(ehh), gx * 0.30 + pose.tilt * mk,
                pal.paper, 1.0);
        }
    }

    // ── 3. 应用：真实图标直接立在盘面上；选中态 = 图标背后的柔和高亮方 ──
    let n_sel = n;
    for i in 0..n {
        let ap = {
            let t0 = st.ent0 + 0.05 + i as f64 * 0.022;
            let k = clamp01(((now - t0) / 0.28) as f32);
            ease_out_back(k)
        };
        if ap <= 0.01 {
            continue;
        }
        // 拖拽经过时也当 hover 画 —— 那就是「要落在这里」的反馈
        let hot_k = if st.drag_over_app == Some(i) {
            1.0
        } else {
            st.app_hot[i]
        };
        let dim = st.app_anim[i];
        // 正在被拖出的那个：跟着光标走，并且随拖出距离淡出
        let dragging = st.drag_app == Some(i);
        let fade = if dragging { st.drag_fade } else { 0.0 };
        let am = (1.0 - 0.38 * dim) * ap * (1.0 - fade);
        let (cx, cy) = if dragging && fade > 0.01 {
            (st.drag_x, st.drag_y)
        } else {
            polar(r_icon * pop, app_angle(i, n))
        };
        let scale = (0.30 + 0.70 * ap) * (1.0 + 0.10 * hot_k);

        // 每格一块**统一的圆角方形底板** —— 「整齐」的来源就在这里：
        // 无论图标自带什么轮廓（圆形标 / 白纸 / 全出血方块），最后都收敛成同一个圆角方块。
        // 白底图标（记事本那页白纸）靠它显形；hover/选中只改描边粗细与投影，
        // 不再另画一层「托底」—— 托底会被底板和图标整个盖住。
        //
        // **底板边长 = icon_draw（锁死），图标边长 = icon_draw / TILE_PAD_K**。
        // 于是底板露出一圈均匀白边 —— 那一圈就是「内边距」，
        // 让图标之间/图标与盘沿之间有呼吸感，同时轮廓仍然是统一的那个圆角方块。
        let sel = hot_k.max(if st.active_app == Some(i) { 0.30 } else { 0.0 });
        let icon_side = icon_draw * scale / TILE_PAD_K;
        {
            let side = icon_draw * scale;
            let half = side * 0.5;
            let rad = side * TILE_RAD_K;
            flat_round_rect(
                buf,
                cx,
                cy,
                half,
                half,
                rad,
                pal.paper,
                (1.4 + 1.6 * sel) * K,
                mixc(mixc(pal.paper, pal.ink, 0.34), pal.ink, sel),
                1.0,
                (3.0 + 2.0 * sel) * K,
                0.20 + 0.10 * sel,
                1.5 * K,
                am,
            );
        }

        if let Some(tex) = &st.apps[i].icon {
            draw_bgra_icon(buf, cx, cy, icon_side, tex, am);
        } else if let Some(ch) = st.apps[i].name.chars().next() {
            // 没有真实图标：淡色圆角方 + 同色字形（不用实色块）
            let half = icon_side * 0.5 * 0.86;
            let acc = GRADS[st.apps[i].grad].0;
            flat_round_rect(
                buf, cx, cy, half, half, half * 0.5, mixc(pal.paper, acc, 0.18), 0.0, hair, 0.0,
                0.0, 0.0, 0.0, am,
            );
            let cht = ch.to_uppercase().to_string();
            draw_text(st, buf, cx as i32, cy as i32, &cht, 20, 700, acc.0, acc.1, acc.2,
                (255.0 * am) as u8, 0, 0);
        }

        if st.commit_pulse > 0.0 && st.commit_app == i && st.commit_chip == usize::MAX {
            let half = icon_draw * 0.5;
            flat_round_rect(buf, cx, cy, half, half, TILE_R, (255, 255, 255), 0.0, (0, 0, 0),
                0.0, 0.0, 0.0, 0.0, 0.42 * st.commit_pulse);
        }
    }
    let _ = n_sel;

    // ── 3b. 取消按钮（拖出去松手后出现，边框走 5 秒倒计时）──
    // 画法抄自 my-pet `objects/dialogs/dynamic_island.py::_paint_event_countdown`：
    // 把底边「点亮」为高亮色，高亮 = 剩余，随倒计时从起点回退。
    // 那边是手绘抖动边框所以线宽要 2.6px 才盖得住；我们是硬边，1.8px 就够。
    // ⚠️ 画在应用图标**之后**：z 序压过图标和它的投影，物理上不该有重叠，
    //    但万一有（数值将来被改坏），也绝不允许图标把按钮盖住。
    if st.cancel_a > 0.01 {
        let (bx, by) = rm_btn_center(st.cancel_a);
        let a = st.cancel_a;
        let hw = RM_BTN_W * 0.5 * a;
        let hh = RM_BTN_H * 0.5 * a;
        let frac = match rm_soonest(st) {
            Some(next) => ((next - now) / REMOVE_GRACE).clamp(0.0, 1.0) as f32,
            None => 0.0,
        };
        flat_round_rect(buf, bx, by + 2.5 * a, hw, hh, RM_BTN_R * a,
            mixc(pal.paper, pal.ink, 0.16), 0.0, pal.ink, 0.0, 0.0, 0.0, 0.0, a);
        flat_round_rect(buf, bx, by, hw, hh, RM_BTN_R * a, pal.paper,
            1.6 * K, mixc(pal.paper, pal.ink, 0.34), 1.0, 0.0, 0.0, 0.0, a);
        // 倒计时：底边直线段（避开两角圆角）画一条墨色圆头线，长度 ∝ 剩余
        let x0 = bx - hw + RM_BTN_R * a;
        let x1 = bx + hw - RM_BTN_R * a;
        let span = (x1 - x0) * frac;
        if span >= 3.0 {
            let y = by + hh - 0.8;
            flat_round_rect(buf, x0 + span * 0.5, y, span * 0.5, 0.9, 0.9,
                pal.ink, 0.0, pal.ink, 0.0, 0.0, 0.0, 0.0, a);
        }
        if a > 0.55 {
            draw_text(st, buf, bx as i32, by as i32, "取消", 13, 700,
                pal.ink.0, pal.ink.1, pal.ink.2, (255.0 * a) as u8, 0, 0);
        }
    }

    // ── 4. 内容段（外环：梯形环）──
    if let Some(ai) = st.active_app {
        if ai < n {
            let (segs, _rots, _rads) = chip_layout(st, ai);
            let m = st.apps[ai].items.len();
            let (ring_in, ring_out) = ring_radii(n);
            for j in 0..m {
                let appear = {
                    let t0 = st.switch0 + j as f64 * 0.026;
                    clamp01(((now - t0) / 0.20) as f32)
                };
                let anim = (st.chip_anim[j] * ease_out_back(appear)).clamp(0.0, 1.0);
                if anim <= 0.01 {
                    continue;
                }
                let hot_k = if st.drag_over_chip == Some(j) {
                    1.0
                } else {
                    st.chip_hot[j]
                };
                // 段的角度范围由布局给出（贴身扇面：角跨 ∝ 名字宽度，段间留缝）
                let (a1, a2) = segs[j];
                let seg = ((a1 + a2) * 0.5, a1, a2);
                // 正被拖出的那一段：原地淡出（与应用图标同款）
                let fade = if st.drag_chip == Some((ai, j)) {
                    st.drag_fade
                } else {
                    0.0
                };
                let am = anim * (1.0 - fade);
                let editing = st.editing == Some((ai, j));

                // 段本体（白底 + `mix(ink 34%)` 描边 + `0 3K` 硬投影）
                // + 段内内容（图标在左、名称在右，沿切线横排）—— 颜色与内环完全一致
                draw_ring_sector(buf, &pal, ring_in, ring_out, seg,
                                 if editing { 0.0 } else { hot_k }, am,
                                 if editing { 1 } else { 0 });
                draw_ring_content(
                    st, buf, &pal, seg, ring_in, ring_out,
                    st.apps[ai].items[j].icon.as_ref(),
                    if editing {
                        st.edit_buf.as_str()
                    } else {
                        st.apps[ai].items[j].name.as_str()
                    },
                    if editing { (255, 255, 255) } else { pal.ink },
                    hot_k, editing, am,
                );

                if st.commit_pulse > 0.0 && st.commit_chip == j {
                    // 提交成功的一次白色闪光
                    draw_ring_sector(buf, &pal, ring_in, ring_out, seg, 0.0,
                                     0.38 * st.commit_pulse, 2);
                }
            }
        }
    }

}

// ───────────────────── 图标位图（面积采样缩放） ─────────────────────

/// 图标纹理 = **mip 链**（逐级 ×1/1.4 的精确面积平均副本）。
///
/// 为什么要 mip：内环图标只画 ~26px、外环 ~19px，而纹理是 73px —— 一次 **2.8:1**
/// 的缩小，无论采样多好都会发虚（实测边缘能量比理想值低 ~25%，肉眼看就是「不清晰」）。
/// 做法是加载时就用**精确面积平均**生成几级副本，绘制时取「≥ 目标边长的最小一级」，
/// 剩下的只有 1.0~1.4:1 的双线性 —— 等效一次高质量缩小（实测边缘能量 +25%）。
#[derive(Clone, Default)]
struct IconTex {
    /// levels[0] 最大，边长递减
    levels: Vec<(usize, Vec<u8>)>,
}

impl IconTex {
    fn from_base(px: usize, data: Vec<u8>) -> IconTex {
        let mut levels = vec![(px, data)];
        let mut cur = px;
        // ⚠️ 步长决定最终质量：`pick` 返回「≥ size 的最小一级」，所以纹理/绘制比
        // `k = tex_px / size` 恒落在 [1, 1.06]。
        //
        // 原来是 **1.4** → k ∈ [1, 1.4]，而**双线性在缩小 27% 时会漏掉高频**
        // （三角核比足迹窄），于是边缘出现台阶状混叠 ——
        // 用户 2026-10-09 报「内环 edge/墨刀、外环桌面/下载锯齿感很明显」就是这个。
        // 1.06 让 k 几乎等于 1，重采样退化成「取最近的、几乎 1:1 的那一级」。
        while cur > 12 {
            let next = ((cur as f32 / 1.06).round() as usize).max(6);
            if next >= cur {
                break;
            }
            let (n0, d0) = &levels[0];
            levels.push((next, downsample_box(d0, *n0, next)));
            cur = next;
        }
        IconTex { levels }
    }

    /// 取「≥ size 的最小一级」；都比 size 小就退回最大的一级。
    fn pick(&self, size: f32) -> (usize, &[u8]) {
        // ⚠️⚠️ `levels` 是**从大到小**排的，而这里要的是「≥ size 里**最小**的那一级」。
        // 顺着从头找会**永远命中第一项 = 基础 73px 纹理** → 整条 mip 链形同虚设，
        // 绘制退化成一次 2.5:1（外环 3.3:1）的大缩小，而采样窗口又被 clamp 在 1.2 纹素
        // （只覆盖所需足迹的一半）→ **严重锯齿**。
        // 用户 2026-10-08 报的原话：「超强的锯齿感，无论外环还是内环」——就是这个。
        // 必须**倒着**找第一个满足的。
        for (n, d) in self.levels.iter().rev() {
            if *n as f32 >= size {
                return (*n, d);
            }
        }
        match self.levels.first() {
            Some((n, d)) => (*n, d),
            None => (0, &[]),
        }
    }
}

/// 精确面积平均（**预乘空间**）把方形纹理缩到 dw×dw。
/// 预乘累加是必须的：透明区 RGB=0，直接平均 RGB 会把边缘拉黑出脏边。
fn downsample_box(src: &[u8], sw: usize, dw: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dw * 4];
    if sw == 0 || dw == 0 || src.len() < sw * sw * 4 {
        return out;
    }
    let scale = sw as f32 / dw as f32;
    for dy in 0..dw {
        let y0 = dy as f32 * scale;
        let y1 = y0 + scale;
        let iy0 = y0.floor() as usize;
        let iy1 = (y1.ceil() as usize).min(sw);
        for dx in 0..dw {
            let x0 = dx as f32 * scale;
            let x1 = x0 + scale;
            let ix0 = x0.floor() as usize;
            let ix1 = (x1.ceil() as usize).min(sw);
            let mut acc = [0.0f32; 4];
            let mut tot = 0.0f32;
            for sy in iy0..iy1 {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                if wy <= 0.0 {
                    continue;
                }
                for sx in ix0..ix1 {
                    let wx = (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    if wx <= 0.0 {
                        continue;
                    }
                    let so = (sy * sw + sx) * 4;
                    let a = src[so + 3] as f32 / 255.0;
                    let ww = wx * wy;
                    acc[0] += src[so + 2] as f32 * a * ww; // R（缓冲是 BGRA）
                    acc[1] += src[so + 1] as f32 * a * ww;
                    acc[2] += src[so] as f32 * a * ww;
                    acc[3] += a * ww;
                    tot += ww;
                }
            }
            let o = (dy * dw + dx) * 4;
            if tot > 1e-6 && acc[3] > 1e-6 {
                out[o] = (acc[2] / acc[3]).clamp(0.0, 255.0) as u8;
                out[o + 1] = (acc[1] / acc[3]).clamp(0.0, 255.0) as u8;
                out[o + 2] = (acc[0] / acc[3]).clamp(0.0, 255.0) as u8;
                out[o + 3] = ((acc[3] / tot) * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn draw_bgra_icon(buf: &mut [u8], cx: f32, cy: f32, size: f32, tex: &IconTex, alpha: f32) {
    let (tex_px, data) = tex.pick(size);
    if alpha <= 0.01 || size <= 0.5 || tex_px == 0 || data.len() < tex_px * tex_px * 4 {
        return;
    }
    let n = tex_px as f32;
    let half = size * 0.5;
    let ext = half.ceil() as i32 + 1;
    // 每个目标像素覆盖多少源纹素。mip 步长 1.06 → 恒在 [1, 1.06]，
    // 所以下面的足迹面积平均最多跨 2×2 个纹素。
    let k = n / size;
    // 亚像素：中心不再取整（原来 `cx as i32` 是**截断**，最多错 1px，
    // 边缘的 AA 会和底板对不齐、看起来发毛）。
    let bx = cx.floor() as i32;
    let by = cy.floor() as i32;
    let fx0 = cx - bx as f32;                       // ∈ [0,1)
    let fy0 = cy - by as f32;
    // 足迹半宽（源纹素单位）
    let hw = k * 0.5;
    for iy in -ext..=ext {
        let py = by + iy;
        if py < 0 || py >= CANVAS {
            continue;
        }
        for ix in -ext..=ext {
            let px = bx + ix;
            if px < 0 || px >= CANVAS {
                continue;
            }
            // 目标像素中心 → 源坐标（像素中心对像素中心的约定：
            // 目标中心 -cx 时恰好落在源纹理中心）
            let sx = (ix as f32 + 0.5 - fx0) * k + (n - 1.0) * 0.5;
            let sy = (iy as f32 + 0.5 - fy0) * k + (n - 1.0) * 0.5;
            // 足迹 = k×k，做**面积平均**。
            // 这就是与缩小比匹配的抗混叠滤波 —— 2×2 抽头 + 双线性做不到这点，
            // 双线性的三角核只覆盖 2 纹素宽，而足迹需要 k 纹素宽。
            let x0f = sx - hw;
            let x1f = sx + hw;
            let y0f = sy - hw;
            let y1f = sy + hw;
            let ix0 = x0f.floor().max(0.0) as usize;
            let ix1 = (x1f.ceil() as usize).min(tex_px);
            let iy0 = y0f.floor().max(0.0) as usize;
            let iy1 = (y1f.ceil() as usize).min(tex_px);
            let mut acc = [0.0f32; 4];
            for yy in iy0..iy1 {
                let wy = (yy as f32 + 1.0).min(y1f) - (yy as f32).max(y0f);
                if wy <= 0.0 {
                    continue;
                }
                for xx in ix0..ix1 {
                    let wx = (xx as f32 + 1.0).min(x1f) - (xx as f32).max(x0f);
                    if wx <= 0.0 {
                        continue;
                    }
                    // ⚠️ 步长必须是**实际纹理尺寸** `tex_px`，不能写 `ICON_PX` ——
                    // `pe_png_icon` / 32px 回退会返回比 ICON_PX 小的纹理，
                    // 用常量步长会读串行、图标直接花掉。
                    let so = (yy * tex_px + xx) * 4;
                    let a = data[so + 3] as f32 / 255.0;
                    let ww = wx * wy;
                    acc[0] += data[so + 2] as f32 * a * ww; // R
                    acc[1] += data[so + 1] as f32 * a * ww; // G
                    acc[2] += data[so] as f32 * a * ww;     // B
                    acc[3] += a * ww;
                }
            }
            // 分母用**理论足迹面积** k*k（而不是实际覆盖和）：
            // 足迹越出纹理边界的部分按透明处理，边缘 alpha 才会正确衰减，
            // 不会被「重归一化」抬高成硬边。
            let wsum = k * k;
            if wsum <= 0.0 || acc[3] <= 1e-5 {
                continue;
            }
            let a = (acc[3] / wsum) * alpha;
            if a <= 0.004 {
                continue;
            }
            let r = acc[0] / acc[3];
            let g = acc[1] / acc[3];
            let b = acc[2] / acc[3];
            let o = (py as usize * CANVAS as usize + px as usize) * 4;
            blend_over(buf, o, r, g, b, a);
        }
    }
}

/// 旋转版图标绘制 —— 梯形段的内容沿切线排布，图标要跟着弧一起转。
///
/// 采样方式与 `draw_bgra_icon` **完全一致**（预乘空间累加 + 双线性 + 关于中心对称的
/// 超采样），只是把目标像素先**反旋**到图标局部坐标再取样。
/// 落在图标正方形外的子样本直接丢弃（不参与面积平均）—— 否则会去 clamp 纹理边缘，
/// 把边上的像素拖出一条脏边。
#[allow(clippy::too_many_arguments)]
fn draw_bgra_icon_rot(buf: &mut [u8], cx: f32, cy: f32, size: f32,
                      tex: &IconTex, rot: f32, alpha: f32) {
    let (tex_px, data) = tex.pick(size);
    if alpha <= 0.01 || size <= 0.5 || tex_px == 0 || data.len() < tex_px * tex_px * 4 {
        return;
    }
    let n = tex_px as f32;
    let half = size * 0.5;
    // 旋转后的外接框要按 √2 放大
    let ext = (half * 1.4143).ceil() as i32 + 1;
    let (sn, cs) = rot.sin_cos();
    let k = n / size;                                  // 每个目标像素覆盖多少源纹素
    // 亚像素（与非旋转版一致）。`i`/`j` 是相对 `bx`/`by` 的整数偏移，
    // 目标像素中心相对 (bx,by) 的局部坐标 = i + 0.5 - f0。
    let bx = cx.floor() as i32;
    let by = cy.floor() as i32;
    let fx0 = cx - bx as f32;
    let fy0 = cy - by as f32;
    // ⚠️ 旋转版没法直接按足迹做面积平均（足迹在纹理空间是个**旋转的**方块）。
    // 近似做法：在**局部空间**做 N×N 亚样本的 box 积分，每个亚样本反旋到纹理空间
    // 再双线性取样。k ≤ 1.06 → N=3 足够（采样点间距 1/3 局部像素）。
    const N: i32 = 3;
    let w_ = CANVAS as usize;
    for j in -ext..=ext {
        let py = by + j;
        if py < 0 || py >= CANVAS {
            continue;
        }
        for i in -ext..=ext {
            let px = bx + i;
            if px < 0 || px >= CANVAS {
                continue;
            }
            let mut acc = [0.0f32; 4];
            let mut hit = 0.0f32;
            for ty in 0..N {
                for tx in 0..N {
                    let lx = i as f32 + (tx as f32 + 0.5) / N as f32 - fx0;
                    let ly = j as f32 + (ty as f32 + 0.5) / N as f32 - fy0;
                    // 反旋到「纹理」坐标
                    let ux = lx * cs + ly * sn;
                    let uy = -lx * sn + ly * cs;
                    // ⚠️⚠️ 越界判据**必须用旋转后的 (ux, uy)** —— 那才是纹理的两个轴。
                    //
                    // 之前判的是旋转前的 (lx, ly)：那个正方形绕中心转 45° 后，角点会伸到
                    // 纹理外 √2 倍，于是 `sxf/syf` 被 clamp 到纹理边缘。对「内容撑满纹理」
                    // 的图标（Edge / 设置 / chrome…）边缘像素是不透明的，后果两条：
                    //   ① 圆角被边缘像素填满 → 图标看起来是**直角**方
                    //   ② 方块旋转后没覆盖到的 4 个角也被涂上颜色 → **四个方向凸起**
                    // 修法：落在纹理外就**直接丢这一份子样本**，绝不 clamp（clamp = 把内容
                    // 往外抹）。丢完之后画出来的轮廓正好是「随弧转的圆角方块」。
                    if ux.abs() > half || uy.abs() > half {
                        continue;
                    }
                    let sxf = (ux * k + (n - 1.0) * 0.5).clamp(0.0, n - 1.001);
                    let syf = (uy * k + (n - 1.0) * 0.5).clamp(0.0, n - 1.001);
                    let x0 = sxf.floor();
                    let y0 = syf.floor();
                    let (xi, yi) = (x0 as usize, y0 as usize);
                    let (fxr, fyr) = (sxf - x0, syf - y0);
                    let xi1 = (xi + 1).min(tex_px - 1);
                    let yi1 = (yi + 1).min(tex_px - 1);
                    for (yy2, wy) in [(yi, 1.0 - fyr), (yi1, fyr)] {
                        for (xx2, wx) in [(xi, 1.0 - fxr), (xi1, fxr)] {
                            let so = (yy2 * tex_px + xx2) * 4;
                            let a = data[so + 3] as f32 / 255.0;
                            let ww = wx * wy;
                            acc[0] += data[so + 2] as f32 * a * ww;
                            acc[1] += data[so + 1] as f32 * a * ww;
                            acc[2] += data[so] as f32 * a * ww;
                            acc[3] += a * ww;
                            hit += ww;
                        }
                    }
                }
            }
            // 分母 = N*N（理论亚样本数）→ 落在圆角外的子样本按透明计，
            // 边缘 alpha 正确衰减（不能拿 `hit` 当分母，那会把边"补满"成硬直角）。
            let _ = hit;
            let denom = (N * N) as f32;
            if acc[3] <= 1e-5 {
                continue;
            }
            let a = (acc[3] / denom) * alpha;
            if a <= 0.004 {
                continue;
            }
            let r = acc[0] / acc[3];
            let g = acc[1] / acc[3];
            let b = acc[2] / acc[3];
            let o = (py as usize * w_ + px as usize) * 4;
            blend_over(buf, o, r, g, b, a);
        }
    }
}

// ───────────────────── 文字（遮罩法：白字黑底 → 亮度作覆盖率） ─────────────────────

unsafe fn draw_text(
    st: &State,
    buf: &mut [u8],
    cx: i32,
    cy: i32,
    text: &str,
    px: i32,
    _weight: i32,
    cr: u8,
    cg: u8,
    cb: u8,
    alpha: u8,
    max_w: i32, // 0=居中不限宽；>0=左对齐省略号
    _align: i32,
) {
    if st.txt_dc.0.is_null() || text.is_empty() {
        return;
    }
    let dc = HDC(st.txt_dc.0);
    let font = match px {
        20 => st.font_glyph,
        0..=10 => st.font_hint,
        11 => st.font_label,
        12 => st.font_cnt,
        13 => st.font_row,
        14 => st.font_head,
        _ => st.font_name,
    };
    let _ = SelectObject(dc, HGDIOBJ(font.0));
    let _ = SetTextColor(dc, COLORREF(0x00FF_FFFF)); // 白字
    let _ = SetBkColor(dc, COLORREF(0)); // 黑底
    let _ = SetBkMode(dc, TRANSPARENT); // 只画字，不画背景
    // 只清**上一次**用过的宽度（每次清满 448×42 ≈ 1.9 万像素纯属浪费：
    // 一个 chip 要画两次，5 个 chip 每帧就是 19 万次无效写）
    let _ = PatBlt(dc, 0, 0, CANVAS, 140, BLACKNESS); // 全幅清除（与 draw_text_rot 共用 DC）
    let mut chars: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: if max_w > 0 { max_w } else { CANVAS - 8 },
        bottom: 40,
    };
    let fmt_base = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
    if max_w > 0 {
        let _ = DrawTextW(dc, &mut chars, &mut rect, fmt_base | DT_LEFT | DT_END_ELLIPSIS);
    } else {
        let _ = DrawTextW(dc, &mut chars, &mut rect, fmt_base | DT_CALCRECT);
        let tw = rect.right - rect.left;
        // ⚠️ DT_CALCRECT 把 rect.bottom 缩成了文字行高 —— 拿这个矩形直接重画，
        // DT_VCENTER 就在**行高内**居中，字整体上浮 ~11px（曾把「取消」顶出
        // 胶囊上沿）。重画前必须恢复 40px 的满高居中矩形（draw_text_rot 同款）。
        rect.top = 0;
        rect.bottom = 40;
        rect.right = rect.left + tw;
        let _ = DrawTextW(dc, &mut chars, &mut rect, fmt_base | DT_CENTER);
    }
    TXT_W.store((rect.right - rect.left).max(0), Ordering::Relaxed);
    let x0 = if max_w > 0 { cx } else { cx - (rect.right - rect.left) / 2 };
    let y0 = cy - 20;

    let af = alpha as f32 / 255.0;
    for yy in 0..40i32 {
        let py = y0 + yy;
        if py < 0 || py >= CANVAS {
            continue;
        }
        for xx in 0..CANVAS {
            let px_x = x0 + xx;
            if px_x < 0 || px_x >= CANVAS {
                continue;
            }
            let o = (yy as usize * CANVAS as usize + xx as usize) * 4;
            let t = st.txt_bits.add(o);
            let lum = ((*t.add(2)) as u32 + (*t.add(1)) as u32 + (*t.add(0)) as u32) / 3;
            if lum == 0 {
                continue;
            }
            let a = (lum as f32 / 255.0) * af;
            let bo = (py as usize * CANVAS as usize + px_x as usize) * 4;
            if buf[bo + 3] == 0 {
                continue;
            }
            blend_over(buf, bo, cr as f32, cg as f32, cb as f32, a);
        }
    }
}

/// 旋转文字：先把字形画进遮罩（GDI），再按 rot **反向旋转采样**写进画布。
///
/// 贴弧胶囊里的标签必须跟胶囊一起转 —— 否则文字横向躺在倾斜的胶囊里会溢出来。
/// 采样用双线性，保证旋转后边缘不出现硬锯齿。
#[allow(clippy::too_many_arguments)]
unsafe fn draw_text_rot(
    st: &State,
    buf: &mut [u8],
    cx: f32,
    cy: f32,
    text: &str,
    px: i32,
    rot: f32,
    cr: u8,
    cg: u8,
    cb: u8,
    alpha: u8,
) {
    if st.txt_dc.0.is_null() || text.is_empty() || alpha == 0 {
        return;
    }
    let dc = HDC(st.txt_dc.0);
    let font = match px {
        20 => st.font_glyph,
        0..=10 => st.font_hint,
        11 => st.font_label,
        12 => st.font_cnt,
        13 => st.font_row,
        14 => st.font_head,
        _ => st.font_name,
    };
    let _ = SelectObject(dc, HGDIOBJ(font.0));
    let _ = SetTextColor(dc, COLORREF(0x00FF_FFFF));
    let _ = SetBkColor(dc, COLORREF(0));
    let _ = SetBkMode(dc, TRANSPARENT);
    // 3 倍超采样：GDI 按 3 倍字号出遮罩，旋转采样时 1 个目标像素覆盖
    // 3x3 个高分辨率纹素 —— 双线性落在超采样遮罩上等效面积平均，
    // 旋转后的文字边缘恢复 GDI 原生锐度（1x 遮罩旋转 = 重采样发虚的根因）。
    const Z: f32 = 3.0;
    let font3 = match px {
        20 => st.font_glyph3,
        0..=10 => st.font_hint3,
        11 => st.font_label3,
        12 => st.font_cnt3,
        13 => st.font_row3,
        14 => st.font_head3,
        _ => st.font_name3,
    };
    let _ = SelectObject(dc, HGDIOBJ(font3.0));
    let _ = PatBlt(dc, 0, 0, CANVAS, 140, BLACKNESS); // 全幅清除（与 draw_text 共用 DC）
    let mut chars: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT { left: 0, top: 0, right: CANVAS - 8, bottom: 132 };
    let fmt = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
    let _ = DrawTextW(dc, &mut chars, &mut rect, fmt | DT_CALCRECT);
    let tw3 = (rect.right - rect.left).max(1) as f32;
    let tw = tw3 / Z; // 1x 逻辑宽度（用于布局/外接圆）
    let mut rect2 = RECT { left: 0, top: 0, right: tw3 as i32, bottom: 132 };
    let _ = DrawTextW(dc, &mut chars, &mut rect2, fmt | DT_CENTER);
    TXT_W.store((tw3 / 3.0) as i32, Ordering::Relaxed);

    // 一次性诊断：每次 draw_text_rot 的遮罩单独落盘
    if std::env::var_os("RL_MASKDBG").is_some() {
        static MC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let k = MC.fetch_add(1, Ordering::Relaxed);
        let n = (CANVAS * 140 * 4) as usize;
        let mb = std::slice::from_raw_parts(st.txt_bits, n);
        let _ = save_bmp(&format!("{}\u{5c}mask_{}.bmp", std::env::var("RL_MASKDBG").unwrap(), k), mb, CANVAS, 140);
    }

    let (sn, cs) = rot.sin_cos();
    let af = alpha as f32 / 255.0;
    // 目标像素的包围盒 = 旋转后的矩形外接圆
    let ext = (((tw * tw + 1600.0).sqrt()) * 0.5 + 4.0).ceil() as i32;
    let cxi = cx.round() as i32;
    let cyi = cy.round() as i32;
    let w = CANVAS as usize;
    for py in (cyi - ext).max(0)..=(cyi + ext).min(CANVAS - 1) {
        for pxx in (cxi - ext).max(0)..=(cxi + ext).min(CANVAS - 1) {
            let dx = pxx as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            let lx = dx * cs + dy * sn;
            let ly = -dx * sn + dy * cs;
            let mx = lx * Z + tw3 * 0.5;
            let my = ly * Z + 66.0;
            if mx < 0.0 || my < 0.0 || mx >= tw3 || my >= 132.0 {
                continue;
            }
            // 双线性采样遮罩亮度
            let x0 = mx.floor() as i32;
            let y0 = my.floor() as i32;
            let fx = mx - x0 as f32;
            let fy = my - y0 as f32;
            let mut acc = 0.0f32;
            for (ox, oy, wgt) in [
                (0i32, 0i32, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                if wgt <= 0.0 {
                    continue;
                }
                let sx = x0 + ox;
                let sy = y0 + oy;
                if sx < 0 || sy < 0 || sx >= CANVAS || sy >= 140 {
                    continue;
                }
                let o = (sy as usize * CANVAS as usize + sx as usize) * 4;
                let t = st.txt_bits.add(o);
                let lum = ((*t.add(2)) as u32 + (*t.add(1)) as u32 + (*t.add(0)) as u32) as f32
                    / 3.0;
                acc += (lum / 255.0) * wgt;
            }
            if acc <= 0.004 {
                continue;
            }
            let bo = (py as usize * w + pxx as usize) * 4;
            if buf[bo + 3] == 0 {
                continue;
            }
            blend_over(buf, bo, cr as f32, cg as f32, cb as f32, acc * af);
        }
    }
}

// ───────────────────── 双击胶囊改名 ─────────────────────
//
// 交互链：双击外环胶囊 → 就地编辑（不弹任何浮层）→ Enter 落盘 / Esc 取消 /
// 空提交时**删掉这个条目**（和设置页的 ✕ 一致，但有二次确认的机会：Esc 随时可撤）。
//
// 为什么不用右键菜单：菜单要多一套命中/焦点/键盘可达性，而改名是唯一高频操作，
// 双击两下直达更短。代价是**不能只靠键盘**（键盘用户走设置页的「编辑」）。
//
// ⚠️ 中文：不用自己管 IME。窗口是**正常有焦点的 HWND**，系统会发
// `WM_IME_SETCONTEXT` / `WM_IME_COMPOSITION`，合成结果最终以 `WM_CHAR` /
// `WM_IME_CHAR` 形式进来 —— 照着追加就行。但**要主动把 IME 的候选窗挪到光标下面**
// （`ImmSetCompositionWindow`），否则中文候选框会飘到屏幕中央。

/// 双击的时间窗（毫秒）。取系统值但夹一下：太短会误触发，太长会跟「两次单击
/// 分别执行」冲突（单击是执行应用/关闭菜单，这里必须区别对待）。
const DBL_MS: f64 = 380.0;
/// 双击允许的最大位移（px）。手指抖一下就不算了。
const DBL_SLOP: i32 = 6;
/// 第一次单击后**等多久**才真的启动应用（毫秒）。必须略大于 `DBL_MS`。
/// 没有这个延迟，第一击就会启动应用 —— 双击等于「先打开应用再改名」，
/// 这正是 2026-10-07 用户报的问题（「双击后依然是单击的效果，会打开胶囊对应的应用」）。
const DBL_WAIT_MS: u32 = (DBL_MS as u32) + 70;

/// 点击胶囊的结果（决定调用方下一步做什么）。
#[derive(PartialEq)]
enum ChipClick {
    /// 命中胶囊，且这是**双击的第二击** → 进改名，**不要执行应用**
    Double,
    /// 命中胶囊，但这是**第一击** → 排一个延迟执行；返回时**不要**执行应用
    Defer,
}

/// 在 (ai, j) 这个胶囊上处理一次点击。
///
/// ⚠️ **第一击绝不能直接执行应用** —— 单击和双击在事件上无法区分，
/// 只能「都延后执行」，等第二击来了就取消执行、改为改名。
/// 返回 `None` = 没命中胶囊（或已在改名态），调用方走原来的单击逻辑。
fn chip_click(st: &mut State, ai: usize, j: usize, sx: i32, sy: i32) -> Option<ChipClick> {
    let now = now_s();
    let is_dbl = (now - st.edit_last_click) * 1000.0 <= DBL_MS
        && (sx - st.edit_last_pos.0).abs() <= DBL_SLOP
        && (sy - st.edit_last_pos.1).abs() <= DBL_SLOP;
    if is_dbl {
        st.edit_last_click = -99.0;
        if st.editing == Some((ai, j)) {
            // 已经在编辑 → 这次是「选中原文本」（第一次按键会替换它）
            st.edit_started = -1.0;
        } else {
            st.editing = Some((ai, j));
            st.edit_buf = st.apps[ai].items[j].name.clone();
            st.edit_started = 0.0;
        }
        st.edit_caret = now;
        EDIT_ARMED.store(true, Ordering::SeqCst);
        return Some(ChipClick::Double);
    }
    st.edit_last_click = now;
    st.edit_last_pos = (sx, sy);
    // 正在编辑别处时点别处 = 放弃（不改内容）
    if st.editing.is_some() {
        st.editing = None;
        st.edit_buf.clear();
        EDIT_ARMED.store(false, Ordering::SeqCst);
        return None;
    }
    Some(ChipClick::Defer)
}

/// 排一个「延迟单击」：先不执行，等 `DBL_WAIT_MS` 看有没有第二击。
/// 用 `SetTimer` 而不是 `std::thread::sleep` —— 后者会卡住 UI 线程，
/// 那正是注释里写过的「钩子装在 UI 线程会被渲染帧阻塞」同款错误。
fn arm_delayed_click(st: &mut State, hwnd: HWND, ai: usize, j: usize) {
    clear_delayed_click(st, hwnd);
    st.pending_click = Some((ai, j));
    unsafe { let _ = SetTimer(hwnd, TIMER_CLICK, DBL_WAIT_MS, None); }
}

/// 取消待执行的单击（第二击来了 → 改为改名；或菜单收起 → 一律作废）。
fn clear_delayed_click(st: &mut State, hwnd: HWND) {
    if st.pending_click.take().is_some() {
        unsafe { let _ = KillTimer(hwnd, TIMER_CLICK); }
    }
}

/// 提交 / 取消编辑。落盘走 `save_settings`（它会先备份成 .bak）。
unsafe fn edit_finish(st: &mut State, commit: bool) {
    EDIT_ARMED.store(false, Ordering::SeqCst);
    let Some((ai, j)) = st.editing.take() else {
        return;
    };
    let buf = std::mem::take(&mut st.edit_buf);
    if !commit {
        return;
    }
    let newname = buf.trim();
    if ai >= st.apps.len() || j >= st.apps[ai].items.len() {
        return;
    }
    if newname.is_empty() {
        // 空提交 = 删除这个条目（与设置页 ✕ 同义）
        st.apps[ai].items.remove(j);
        println!("[rename] 空提交 → 删除条目 (app={} j={})", ai, j);
    } else if newname != st.apps[ai].items[j].name {
        let old = st.apps[ai].items[j].name.clone();
        st.apps[ai].items[j].name = newname.to_string();
        // 颜色是「按名字自动」的，改名后要失效重算（除非用户显式设过）
        if st.apps[ai].items[j].color.is_none() {
            st.apps[ai].items[j].color = None;
        }
        println!("[rename] {} -> {}", old, newname);
    } else {
        return;
    }
    save_settings(st);
}

/// 把一条字符追加进编辑缓冲（过滤控制字符）。
fn edit_push(st: &mut State, ch: char) {
    // 第一次输入时若处于「选中原文本」状态 → 丢弃旧文本
    if st.edit_started < 0.0 {
        st.edit_buf.clear();
        st.edit_started = 0.0;
    }
    if st.edit_buf.chars().count() < 24 && !ch.is_control() {
        st.edit_buf.push(ch);
    }
    st.edit_caret = now_s();
}

// ───────────────────── 拖放识别与入库 ─────────────────────

/// 画布坐标 → 命中 ai 应用的外环胶囊（返回 chip 序号）
fn hit_chip_at(st: &State, ai: usize, px: f32, py: f32) -> Option<usize> {
    // 与绘制共用同一套几何：**极坐标判定**（半径落在环带内、角度落在该段内）。
    // 梯形段是「两条直的径向边 + 内外弧」，极坐标判定与它天然吻合，
    // 比反旋到局部判矩形更准。⚠️ 只能有一份判定实现，否则形状一改就「画在这、点到那」。
    if ai >= st.apps.len() {
        return None;
    }
    let m = st.apps[ai].items.len();
    if m == 0 {
        return None;
    }
    let (segs, _rots, _rads) = chip_layout(st, ai);
    let (r_in, r_out) = ring_radii(st.apps.len());
    let dx = px - CC;
    let dy = py - CC;
    let r = (dx * dx + dy * dy).sqrt();
    let tol = 6.0;                       // 命中容差：比图形略松一点，好点
    if r < r_in - tol || r > r_out + tol {
        return None;
    }
    let th = dx.atan2(-dy).to_degrees(); // 与 `polar` 同一约定：0 = 正上、顺时针
    for (j, (a1, a2)) in segs.iter().enumerate() {
        // 角度落在该段内（首尾各给 4° 容差，好点）
        let mut d = th - (a1 + a2) * 0.5;
        d = ((d + 180.0).rem_euclid(360.0)) - 180.0;
        if d.abs() <= (a2 - a1) * 0.5 + 4.0 {
            return Some(j);
        }
    }
    None
}

/// 画布坐标 → 最近的应用磁贴（拖放落点识别）
fn nearest_app(st: &State, px: f32, py: f32, slack: f32) -> Option<usize> {
    let n = st.apps.len();
    if n == 0 {
        return None;
    }
    let dx = px - CC;
    let dy = py - CC;
    let (r_icon, icon_sz) = ring_layout(n);
    let hit_half = icon_sz * 0.5 + 7.0;
    if (dx * dx + dy * dy).sqrt() > r_icon + hit_half + slack + 4.0 {
        return None;
    }
    let per = 360.0 / n as f32;
    let deg = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
    let i = ((deg / per).round() as usize).rem_euclid(n);
    let (ix, iy) = polar(r_icon, app_angle(i, n));
    let d = ((px - ix) * (px - ix) + (py - iy) * (py - iy)).sqrt();
    if d <= hit_half + slack {
        Some(i)
    } else {
        None
    }
}

/// 拖放落点 → 归属应用：图标磁贴 > 外环胶囊 > 盘面以内（当前亮着的应用）
fn drop_force_app(st: &State, sx: i32, sy: i32) -> Option<usize> {
    let px = sx - st.win_x;
    let py = sy - st.win_y;
    if let Some(ai) = nearest_app(st, px as f32, py as f32, 8.0) {
        return Some(ai);
    }
    // 落在某个应用的外环胶囊上 → 归那个应用（按当前位置算，不是「当前亮着的」）
    for ai in 0..st.apps.len().min(MAX_APPS) {
        if hit_chip_at(st, ai, px as f32, py as f32).is_some() {
            return Some(ai);
        }
    }
    // ⚠️ 这里**不能**再回退到 `st.active_app`。
    // 它是粘性的：悬停过 A 之后光标挪到中心头像，active_app 仍是 A，
    // 于是「把东西拖到中间的球上松手」会被记进 A 的外环（用户实测踩到）。
    // 位置不在任何目标上就是不认 —— 交给 process_drop 的自动识别分支。
    None
}

/// 路径 → DropItem（文件夹/exe/lnk/普通文件）
fn drop_item_from_path(path: &str) -> DropItem {
    let is_dir = std::path::Path::new(path).is_dir();
    let stem = std::path::Path::new(path)
        .file_stem()
        .or_else(|| std::path::Path::new(path).file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    DropItem {
        name: clamp_name(&stem),
        target: path.to_string(),
        is_url: false,
        is_dir,
    }
}

/// 剪贴板兜底（环上按 V）：CF_HDROP → CF_UNICODETEXT（像链接才收）
unsafe fn clipboard_drop_items() -> Vec<DropItem> {
    let mut out: Vec<DropItem> = Vec::new();
    if OpenClipboard(HWND(ptr::null_mut())).is_err() {
        return out;
    }
    if let Ok(h) = GetClipboardData(15) {
        // CF_HDROP
        let hg = HGLOBAL(h.0);
        let n = DragQueryFileW(HDROP(hg.0), u32::MAX, None);
        for i in 0..n {
            let mut buf = [0u16; 520];
            let len = DragQueryFileW(HDROP(hg.0), i, Some(&mut buf)) as usize;
            let path = String::from_utf16_lossy(&buf[..len]);
            if !path.is_empty() {
                out.push(drop_item_from_path(&path));
            }
        }
    }
    if out.is_empty() {
        if let Ok(h) = GetClipboardData(13) {
            // CF_UNICODETEXT
            let hg = HGLOBAL(h.0);
            let p = GlobalLock(hg);
            if !p.is_null() {
                let ws = p as *const u16;
                let mut len = 0usize;
                while *ws.add(len) != 0 && len < 2048 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ws, len));
                let _ = GlobalUnlock(hg);
                let text = text.trim().to_string();
                if text.starts_with("http://") || text.starts_with("https://") {
                    out.push(DropItem {
                        name: url_host_name(&text),
                        target: text,
                        is_url: true,
                        is_dir: false,
                    });
                }
            }
        }
    }
    let _ = CloseClipboard();
    out
}

/// 当前**真正聚焦**的应用：光标在内环某个图标上、或在该应用的外环胶囊上、
/// 或拖放正悬在某个落点上。
///
/// ⚠️ 绝对不能直接拿 `active_app` 当焦点 —— 它是**粘性**的（为了「移向外环不丢选中」故意不清空）。
/// 光标挪到中心头像或盘面空白后它依然非空，于是其它应用的压暗遮罩会一直挂着不散：
/// 用户看到的就是「hover 取消了，但别的应用还蒙着一层灰」。
fn focus_app(st: &State) -> Option<usize> {
    if let Some(i) = st.drop_target_app {
        return Some(i);
    }
    if let Some(i) = st.hover_now {
        return Some(i);
    }
    if st.hover_chip.is_some() {
        return st.active_app;
    }
    None
}

/// 环上按 V：把剪贴板里的文件/链接加入环（拖放的兜底通道）
unsafe fn paste_add(st: &mut State) {
    let items = clipboard_drop_items();
    if items.is_empty() {
        return;
    }
    let force = focus_app(st); // 真的悬停在哪就加进谁；没悬停则自动识别
    let n = process_drop(st, &items, force);
    if n > 0 {
        st.commit_pulse = 1.0;
        // 与拖放入环同款：消化新成员时短暂「忙碌」
        set_mood(st, 5);
        st.mood_hold = now_s() + 1.5;
    }
    println!("[paste] 入环 {} 项", n);
}

/// 入环：URL → 浏览器外环（推荐）或落点应用；文件夹 → 资源管理器外环；
/// exe/lnk/文件 → 新增进常用。落到哪个图标上 = 强制加进谁。
/// 返回新增条数。入库即回写 ini 并全量热重载。
fn process_drop(st: &mut State, items: &[DropItem], force_app: Option<usize>) -> usize {
    let mut added = 0usize;
    for it in items {
        drag_log(&format!(
            "item [{}] target={} force={:?}",
            it.name, it.target, force_app
        ));
        let dest: Option<usize> = if let Some(ai) = force_app {
            if ai < st.apps.len() { Some(ai) } else { None }
        } else if it.is_url {
            // 推荐：目标本身是 URL 或已有 URL 条目的应用（= 浏览器）
            st.apps
                .iter()
                .position(|a| a.target.contains("://") || a.items.iter().any(|x| x.target.contains("://")))
        } else if it.is_dir {
            // 推荐：资源管理器（target 或已有条目是目录）
            st.apps.iter().position(|a| {
                a.target.eq_ignore_ascii_case("explorer.exe")
                    || a.items.iter().any(|x| std::path::Path::new(&x.target).is_dir())
            })
        } else {
            None
        };

        match dest {
            Some(ai) => {
                let app = &st.apps[ai];
                if app.items.iter().any(|x| x.target.eq_ignore_ascii_case(&it.target)) {
                    drag_log(&format!("  skip dup -> [{}]", app.name));
                    continue; // 已存在
                }
                if app.items.len() >= MAX_ITEMS {
                    drag_log(&format!(
                        "  skip FULL ({}/{} items) -> [{}]",
                        app.items.len(),
                        MAX_ITEMS,
                        app.name
                    ));
                    continue; // 外环满了
                }
                let app_name = app.name.clone();
                if ini_insert_item(&st.ini_path, &app_name, &it.name, &it.target) {
                    added += 1;
                }
            }
            None => {
                if st.apps.iter().any(|a| a.target.eq_ignore_ascii_case(&it.target)) {
                    drag_log("  skip dup (top-level)");
                    continue;
                }
                if st.apps.len() >= MAX_APPS {
                    drag_log("  skip FULL apps");
                    continue;
                }
                if ini_insert_app(&st.ini_path, &it.name, &it.target) {
                    added += 1;
                }
            }
        }
    }
    drag_log(&format!("  added={}", added));
    if added > 0 {
        // ini 已更新，全量热重载（重新解析 + 重新解码图标）
        unsafe { load_ini(st) };
    }
    added
}

/// 把 `name=target` 插入 [app.<app>] 段（无段则建段）。成功返回 true。
fn ini_insert_item(path: &str, app: &str, key: &str, value: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let header = format!("[app.{}]", app);
    let line = format!("{}={}", key, value);
    let mut out = String::new();
    if let Some(pos) = text.find(&header) {
        // 段尾 = 下一个段头或 EOF
        let after = &text[pos + header.len()..];
        let next_sec = after
            .find("\n[")
            .map(|i| pos + header.len() + i + 1)
            .unwrap_or(text.len());
        out.push_str(&text[..next_sec]);
        // 规范上一行结尾
        if !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        out.push_str(&line);
        out.push_str("\r\n");
        out.push_str(&text[next_sec..]);
    } else {
        out.push_str(text.trim_end());
        if !out.is_empty() {
            out.push_str("\r\n");
        }
        out.push_str(&header);
        out.push_str("\r\n");
        out.push_str(&line);
        out.push_str("\r\n");
    }
    std::fs::write(path, out).is_ok()
}

/// 把 `name=target` 追加进 [apps] 段。成功返回 true。
fn ini_insert_app(path: &str, key: &str, value: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let line = format!("{}={}", key, value);
    let mut out = String::new();
    if let Some(pos) = text.find("[apps]") {
        let after = &text[pos + 6..];
        let next_sec = after
            .find("\n[")
            .map(|i| pos + 6 + i + 1)
            .unwrap_or(text.len());
        out.push_str(&text[..next_sec]);
        if !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        out.push_str(&line);
        out.push_str("\r\n");
        out.push_str(&text[next_sec..]);
    } else {
        out.push_str("[apps]\r\n");
        out.push_str(&line);
        out.push_str("\r\n");
        out.push_str(text.trim_start());
    }
    std::fs::write(path, out).is_ok()
}

// ───────────────────── 执行 ─────────────────────

fn launch(target: &str) {
    unsafe {
        let p = wide(target);
        let _ = ShellExecuteW(
            HWND(ptr::null_mut()),
            PCWSTR::null(),
            PCWSTR(p.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn commit(st: &mut State) {
    // 正在改名时 commit 应该是「提交」，不是「启动」——
    // 正常路径 Enter 已在 WM_APP_KEY 里被 edit 分支截走，这里是兜底
    // （比如鼠标点别的路径绕过了钩子）。
    if st.editing.is_some() {
        unsafe {
            edit_finish(st, true);
        }
        return;
    }
    unlock(st);
    let action = match (st.active_app, st.hover_chip) {
        (Some(ai), Some(ci)) => st.apps.get(ai).and_then(|a| a.items.get(ci)).map(|it| (ai, Some(ci), it.name.clone(), it.target.clone())),
        (Some(ai), None) => st.apps.get(ai).map(|a| (ai, None, a.name.clone(), a.target.clone())),
        _ => None,
    };

    if let Some((ai, ci, _, _)) = action {
        st.commit_app = ai;
        st.commit_chip = ci.unwrap_or(usize::MAX);
        st.commit_pulse = 1.0;
    }
    unsafe {
        hide_menu(st);
    }

    if let Some((_, _, label, target)) = action {
        launch(&target);
        println!("[*] 启动 · {label}");
    }
}

// ───────────────────── 圆环窗口 ─────────────────────

unsafe extern "system" fn ring_proc(hwnd: HWND, msg: u32, _w: WPARAM, _l: LPARAM) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            // 窗口接收鼠标后必须自设光标：拖放期间系统可能留下忙碌指针，
            // 类光标为空时 Windows 不会自动改回，于是"一直转圈"
            unsafe {
                let _ = SetCursor(LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(ptr::null_mut())));
            }
            LRESULT(1)
        }
        WM_NCHITTEST => {
            // 菜单开着 = 接收鼠标（拖放定位/命中需要）；不透明像素之外
            // 的纯透明区域由分层窗口机制自动穿透
            if MENU_OPEN.load(Ordering::Relaxed) {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, _w, _l),
    }
}

// ───────────────────── 离屏调试导出 ─────────────────────

unsafe fn save_bmp(path: &str, bgra: &[u8], w: i32, h: i32) -> bool {
    use std::io::Write;
    let mut out: Vec<u8> = Vec::with_capacity(bgra.len() + 1078);
    // 每行按 4 字节对齐填充 —— 必须按**行宽**算，不能按总像素数算，
    // 否则宽度不是 4 的倍数时文件会短一截（BMP 损坏）。
    let row = w as u32 * 3;
    let pad = (4 - row % 4) % 4;
    let stride = row + pad;
    let data_len = stride * h as u32;
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54u32 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for y in (0..h as usize).rev() {
        for x in 0..w as usize {
            let o = (y * w as usize + x) * 4;
            out.push(bgra[o]); // B
            out.push(bgra[o + 1]); // G
            out.push(bgra[o + 2]); // R
        }
        for _ in 0..pad {
            out.push(0);
        }
    }
    std::fs::File::create(path).and_then(|mut f| f.write_all(&out)).is_ok()
}

/// 裸 BGRA（top-down，无文件头）落盘 —— 只给离屏分析用。
/// `save_bmp` 写的是 24bpp，**alpha 会被整条丢掉**，做图标对照稿时不够用
/// （透明底图标在对照稿里会变成白底图块，看不出真实轮廓）。
fn save_rgba(path: &str, bgra: &[u8]) -> bool {
    use std::io::Write;
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(bgra))
        .is_ok()
}

unsafe fn synth_bg(st: &mut State, gray: bool) {    // 彩色壁纸（macOS Sonoma 风格），便于离屏校验玻璃染色
    let n = (CANVAS * CANVAS) as usize;
    let buf = std::slice::from_raw_parts_mut(st.bg_bits, n * 4);
    for y in 0..CANVAS as usize {
        for x in 0..CANVAS as usize {
            let u = x as f32 / CANVAS as f32;
            let v = y as f32 / CANVAS as f32;
            // 蓝紫底 + 三个光斑
            let mut r = 0.42 + 0.20 * (1.0 - v) + 0.10 * u;
            let mut g = 0.46 + 0.16 * (1.0 - u) + 0.06 * v;
            let mut b = 0.92 - 0.10 * v + 0.06 * u;
            let blob = |cx: f32, cy: f32, rad: f32, amp: f32, ch: usize, c: &mut [f32; 3]| {
                let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt();
                let k = (1.0 - (d / rad).clamp(0.0, 1.0)).powi(2) * amp;
                c[ch] += k;
            };
            let mut c = [r, g, b];
            blob(0.18, 0.12, 0.55, 0.42, 0, &mut c);
            blob(0.18, 0.12, 0.55, 0.40, 1, &mut c);
            blob(0.18, 0.12, 0.55, 0.36, 2, &mut c);
            blob(0.86, 0.20, 0.50, 0.20, 0, &mut c);
            blob(0.86, 0.20, 0.50, 0.24, 1, &mut c);
            blob(0.86, 0.20, 0.50, 0.46, 2, &mut c);
            blob(0.70, 0.92, 0.52, 0.40, 0, &mut c);
            blob(0.70, 0.92, 0.52, 0.24, 1, &mut c);
            blob(0.70, 0.92, 0.52, 0.05, 2, &mut c);
            r = c[0].clamp(0.0, 1.0);
            g = c[1].clamp(0.0, 1.0);
            b = c[2].clamp(0.0, 1.0);
            let o = (y * CANVAS as usize + x) * 4;
            if gray {
                // 近白灰桌面：盘面必须仍然是一块亮玻璃，不能跟着塌成中灰
                let l = 0.299 * r + 0.587 * g + 0.114 * b;
                let v = 0.62 + 0.30 * l;
                buf[o] = (v * 255.0) as u8;
                buf[o + 1] = (v * 255.0) as u8;
                buf[o + 2] = (v * 255.0) as u8;
            } else {
                buf[o] = (b * 255.0) as u8;
                buf[o + 1] = (g * 255.0) as u8;
                buf[o + 2] = (r * 255.0) as u8;
            }
            buf[o + 3] = 255;
        }
    }
}

unsafe fn draw_to_buf(st: &mut State) -> Vec<u8> {
    let n = (CANVAS * CANVAS * 4) as usize;
    let buf = std::slice::from_raw_parts_mut(st.dbi_bits, n);
    draw_scene(st, buf);
    std::slice::from_raw_parts(st.dbi_bits, n).to_vec()
}

unsafe fn dump_all(dir: &str) {
    let mut st = State::new();
    st.dump_bg = true;
    st.ini_path = format!("{}\\settings.ini", dir);
    if !std::path::Path::new(&st.ini_path).exists() {
        let _ = std::fs::write(&st.ini_path, default_ini());
    }
    load_ini(&mut st);
    init_surfaces(&mut st);
    init_fonts(&mut st);
    synth_bg(&mut st, false);
    st.ent0 = -10.0; // 动画已收敛
    st.switch0 = -10.0;

    // 1) 静置态（无悬停）
    st.entering = false;
    st.active_app = None;
    st.hover_chip = None;
    eprintln!("[cp4] drawing disc");
    let buf = draw_to_buf(&mut st);
    eprintln!("[cp5] disc drawn");
    save_bmp(&format!("{}\\dump_disc.bmp", dir), &buf, CANVAS, CANVAS);

    // 2) 悬停态（第一个有内容的应用展开 chips + 选中环 + 压暗其他）
    if let Some(hi) = st
        .apps
        .iter()
        .enumerate()
        .max_by_key(|(_, a)| a.items.len())
        .filter(|(_, a)| !a.items.is_empty())
        .map(|(i, _)| i)
    {
        eprintln!("[cp6] hover app {} items {}", hi, st.apps[hi].items.len());
        st.active_app = Some(hi);
        st.hover_chip = Some(0);
        for (i, v) in st.app_anim.iter_mut().enumerate() {
            *v = if i == hi { 0.0 } else { 1.0 };
        }
        for v in st.chip_anim.iter_mut() {
            *v = 1.0;
        }
        for (i, v) in st.app_hot.iter_mut().enumerate() {
            *v = if i == hi { 1.0 } else { 0.0 };
        }
        for (j, v) in st.chip_hot.iter_mut().enumerate() {
            *v = if j == 0 { 1.0 } else { 0.0 };
        }
        let buf = draw_to_buf(&mut st);
        save_bmp(&format!("{}\\dump_hover.bmp", dir), &buf, CANVAS, CANVAS);

        // 2b) 改名编辑态：同一个 chip 换成墨底 + 输入缓冲（核对对比度与光标）
        let real_name = st.apps[hi].items[0].name.clone();
        st.editing = Some((hi, 0));
        st.edit_buf = format!("{}·改", real_name);
        st.edit_caret = now_s(); // 让光标落在「亮」的相位上
        let buf = draw_to_buf(&mut st);
        let edit_path = [dir, "\\dump_edit.bmp"].concat();
        save_bmp(&edit_path, &buf, CANVAS, CANVAS);
        st.editing = None;
        st.edit_buf.clear();
    }

    // 3) 灰底变体（模拟浅灰桌面，检查盘面是否仍然明亮通透）
    synth_bg(&mut st, true);
    st.active_app = Some(0);
    st.hover_chip = None;
    for v in st.chip_anim.iter_mut() {
        *v = 1.0;
    }
    let buf = draw_to_buf(&mut st);
    save_bmp(&format!("{}\\dump_disc_gray.bmp", dir), &buf, CANVAS, CANVAS);
    synth_bg(&mut st, false);

    // 3.5) 侧面方位：选「有子项且方位角最靠近 90°」的应用，
    //      验证「贴弧胶囊」在 3 点方向依然可读（旋转最容易在这一档崩）
    {
        let ns = st.apps.len();
        let mut best: Option<usize> = None;
        let mut bd = f32::MAX;
        for i in 0..ns {
            if st.apps[i].items.is_empty() {
                continue;
            }
            let d = (app_angle(i, ns) - 90.0).abs();
            if d < bd {
                bd = d;
                best = Some(i);
            }
        }
        if let Some(b) = best {
            st.active_app = Some(b);
            st.hover_chip = None;
            for v in st.chip_anim.iter_mut() {
                *v = 1.0;
            }
            for (i, v) in st.app_hot.iter_mut().enumerate() {
                *v = if i == b { 1.0 } else { 0.0 };
            }
            let buf = draw_to_buf(&mut st);
            save_bmp(&format!("{}\\dump_side.bmp", dir), &buf, CANVAS, CANVAS);
            st.app_hot = [0.0; MAX_APPS];
            st.chip_anim = [0.0; MAX_ITEMS];
        }
    }

    // 3.8) 深色盘面对照图（托盘可切换；深色下盘面/图标/胶囊的对比度要单独看）
    st.glass_liquid = true;
    st.active_app = None;
    st.hover_chip = None;
    st.app_hot = [0.0; MAX_APPS];
    let buf = draw_to_buf(&mut st);
    save_bmp(&format!("{}\\dump_dark.bmp", dir), &buf, CANVAS, CANVAS);
    st.glass_liquid = false;

    // 4) chip 命中自检：绘制用的中心点必须能命中自己（保证 hover/点击不失效）
    st.win_x = 0;
    st.win_y = 0;
    let mut pass = 0usize;
    let mut fail = 0usize;
    if let Some(hi) = st
        .apps
        .iter()
        .enumerate()
        .max_by_key(|(_, a)| a.items.len())
        .filter(|(_, a)| !a.items.is_empty())
        .map(|(i, _)| i)
    {
        st.active_app = Some(hi);
        let m = st.apps[hi].items.len();
        for j in 0..m {
            // 重新取布局（每次命中后 hover_chip 会被 reset，不影响 layout）
            let (segs, _rots, rads) = chip_layout(&st, hi);
            let (cx, cy) = polar(rads[j], (segs[j].0 + segs[j].1) * 0.5);
            update_hover(&mut st, cx as i32, cy as i32);
            if st.hover_chip == Some(j) {
                pass += 1;
            } else {
                fail += 1;
                eprintln!(
                    "[hit] FAIL chip {} ({}) center=({:.0},{:.0}) -> got {:?}",
                    j, st.apps[hi].items[j].name, cx, cy, st.hover_chip
                );
            }
        }
    }
    // 调试：导出文字遮罩 DC 内容（3x 超采样遮罩诊断）
    {
        let n = (CANVAS * 140 * 4) as usize;
        let buf = std::slice::from_raw_parts(st.txt_bits, n);
        save_bmp(&format!("{}\u{5c}dump_mask.bmp", dir), buf, CANVAS, 140);
    }
    println!("[*] 已导出 dump_disc/dump_hover/dump_disc_gray.bmp 到 {dir}");
    println!("[*] chip 命中自检：PASS={pass} FAIL={fail}");
}

// ───────────────────── 退出清理 ─────────────────────

/// 结束本项目的其它实例。
///
/// ⚠️ 两个必须注意的地方（都踩过）：
/// ① **不能只按进程名匹配**。上一轮为了绕开「文件被占用」把 exe 改名成
///    `RadialMenu.old.exe` / `RadialMenu.prev.exe`，测试实例又在 `_dragtest/` 下 ——
///    按名字匹配会把它们**全部漏掉**（`--quit` 什么都没杀就是这么来的）。
///    现在按**映像文件的完整路径**判断（文件名以 `RadialMenu` / `radial-launcher` 开头）。
/// ② **进程表里可能有「已退出但对象未回收」的僵尸**（异常终止 / 会话切换残留）。
///    它们 `GetExitCodeProcess` 返回的不是 `STILL_ACTIVE`，`taskkill` 会报
///    「没有此任务的实例在运行」。对这种 PID 调 `TerminateProcess` 纯属浪费，
///    直接跳过。
unsafe fn kill_other_instances() {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };

    let me = std::process::id();
    // ① 有宿主窗口的实例 = 活实例 → 发 WM_CLOSE 走 quit_app 优雅退出。
    //    TerminateProcess 会把钩子线程连人带枪活埋，钩子仍挂在输入链上
    //    （僵尸壳毒化全系统鼠标）。硬杀只留给不响应的壳。
    let host_cls: Vec<u16> = wide("RLHost");
    if let Ok(hw) = FindWindowW(PCWSTR(host_cls.as_ptr()), PCWSTR::null()) {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hw, Some(&mut pid));
        if pid != 0 && pid != me {
            let mut res = 0;
            let _ = SendMessageTimeoutW(
                hw, WM_CLOSE, WPARAM(0), LPARAM(0), SMTO_BLOCK, 3000, Some(&mut res),
            );
            // 等它彻底退场（quit_app 里摘钩子 + 等钩子线程，一般 <1s）
            let t0 = GetTickCount();
            loop {
                let alive = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    Ok(hq) => {
                        let mut code = 0u32;
                        let _ = GetExitCodeProcess(hq, &mut code);
                        let _ = CloseHandle(hq);
                        code as i32 == STILL_ACTIVE.0
                    }
                    Err(_) => break, // 进程没了
                };
                if !alive || GetTickCount().wrapping_sub(t0) > 4000 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }

    let Ok(snap_h) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
        return;
    };
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut killed = 0u32;

    if Process32FirstW(snap_h, &mut entry).is_ok() {
        loop {
            let pid = entry.th32ProcessID;
            let name = {
                let buf = &entry.szExeFile;
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                String::from_utf16_lossy(&buf[..n])
            };
            let base = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_ascii_lowercase();
            let ours = base.starts_with("radialmenu") || base.starts_with("radial-launcher");
            if pid != 0 && pid != me && ours {
                // 先开一个「只查询」的句柄判断是不是僵尸
                if let Ok(hq) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let mut code = 0u32;
                    let ok = GetExitCodeProcess(hq, &mut code).is_ok();
                    let _ = CloseHandle(hq);
                    if !ok || code as i32 == STILL_ACTIVE.0 {
                        if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                            if TerminateProcess(h, 0).is_ok() {
                                killed += 1;
                            }
                            let _ = CloseHandle(h);
                        }
                    }
                }
            }
            if Process32NextW(snap_h, &mut entry).is_err() {
                break;
            }
        }
    }
    let _ = CloseHandle(snap_h);
    if killed > 0 {
        println!("[*] 已结束 {killed} 个残留实例");
    }
}

unsafe impl Send for State {}

// ═══════════════════════════════════════════════════════════════════
//  以下为「重建段」：一次补丁锚点跨度过大，误删了这一整块业务代码，
//  按本会话记录逐段恢复。函数顺序在 Rust 里无关紧要，统一追加在文件末尾。
// ═══════════════════════════════════════════════════════════════════

// ───────────────────── 公共小工具 ─────────────────────

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 可执行文件所在目录（配置、导出文件都放这里）
fn exe_dir() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    let p = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    match p.rfind('\\') {
        Some(i) => p[..i].to_string(),
        None => ".".into(),
    }
}

/// 宽字符串 → String（在第一个 NUL 处截断）
fn path_str_of(wt: &[u16]) -> String {
    let end = wt.iter().position(|&c| c == 0).unwrap_or(wt.len());
    String::from_utf16_lossy(&wt[..end])
}

// ───────────────────── 配色语义映射 ─────────────────────

/// 按名字关键词挑配色 —— 避免「记事本=灰、终端=绿」这种按下标轮转的任意感
fn grad_for(name: &str) -> usize {
    let n = name.to_lowercase();
    let has = |k: &str| n.contains(k);
    // 站点 / 品牌专色（放在通用分组之前，保证外环 chip 有辨识度）
    if has("github") {
        return 2;
    }
    if has("哔哩") || has("bilibili") || has("b站") {
        return 9;
    }
    if has("豆包") || has("doubao") {
        return 4;
    }
    if has("知乎") || has("zhihu") {
        return 10;
    }
    if has("youtube") || has("视频") {
        return 6;
    }
    if has("gmail") {
        return 6;
    }
    if has("浏览器") || has("browser") || has("edge") || has("chrome") || has("firefox")
        || has("网页") || has("百度")
    {
        return 0;
    }
    if has("资源管理") || has("文件夹") || has("explorer") || has("folder") || has("目录") {
        return 1;
    }
    if has("终端") || has("cmd") || has("powershell") || has("wsl") || has("console")
        || has("控制台") || has("shell") || has("term")
    {
        return 2;
    }
    if has("设置") || has("setting") || has("配置") || has("preference") || has("系统") {
        return 3;
    }
    if has("计算器") || has("calc") || has("工具") || has("tool") {
        return 4;
    }
    if has("画图") || has("paint") || has("绘图") || has("截图") || has("photo") {
        return 5;
    }
    if has("邮件") || has("mail") || has("outlook") || has("账单") || has("订阅") {
        return 6;
    }
    if has("微信") || has("qq") || has("chat") || has("message") || has("消息")
        || has("聊天") || has("扫码") || has("收付款")
    {
        return 7;
    }
    if has("音乐") || has("music") || has("网易云") || has("spotify") || has("网易") {
        return 9;
    }
    if has("下载") || has("download") || has("云") || has("drive") {
        return 10;
    }
    if has("记事本") || has("notepad") || has("笔记") || has("note") || has("文档")
        || has("文本") || has("说明")
    {
        return 11;
    }
    if has("任务") || has("task") || has("监控") || has("性能") {
        return 12;
    }
    if has("桌面") || has("desktop") {
        return 5;
    }
    8
}

// ───────────────────── 配置（settings.ini） ─────────────────────

fn default_ini() -> &'static str {
    "[apps]\r\n\
    浏览器=https://www.doubao.com\r\n\
    资源管理器=explorer.exe\r\n\
    记事本=notepad.exe\r\n\
    终端=cmd.exe\r\n\
    计算器=calc.exe\r\n\
    画图=mspaint.exe\r\n\
    任务管理器=taskmgr.exe\r\n\
    设置=ms-settings:\r\n\
    \r\n\
    [app.浏览器]\r\n\
    豆包=https://www.doubao.com\r\n\
    GitHub=https://github.com\r\n\
    知乎=https://www.zhihu.com\r\n\
    哔哩哔哩=https://www.bilibili.com\r\n\
    \r\n\
    [app.资源管理器]\r\n\
    下载=shell:Downloads\r\n\
    文档=shell:Personal\r\n\
    桌面=shell:Desktop\r\n\
    项目=D:\\工作\\IDEA\r\n\
    \r\n\
    [app.终端]\r\n\
    PowerShell=powershell.exe\r\n\
    控制台=cmd.exe\r\n\
    \r\n\
    [app.设置]\r\n\
    显示=ms-settings:display\r\n\
    网络=ms-settings:network\r\n\
    关于=ms-settings:about\r\n"
}

/// 从 INI 读设置开关。dark= 现在是 0 亮 / 1 暗 / 2 跟随系统（旧值 0/1 完全兼容）。
unsafe fn read_persistent(st: &mut State) {
    if let Ok(txt) = std::fs::read_to_string(&st.ini_path) {
        for line in txt.lines() {
            let l = line.trim();
            let get = |p: &str| l.strip_prefix(p).map(|v| v.trim().to_string());
            if let Some(v) = get("dark=") {
                st.theme = v.parse::<i32>().unwrap_or(if v != "0" { 1 } else { 0 }).clamp(0, 2);
            } else if let Some(v) = get("persistent=") {
                st.persistent = v != "0";
            } else if let Some(v) = get("listening=") {
                st.listening = v != "0";
            } else if let Some(v) = get("key_middle=") {
                st.key_middle = v != "0";
            } else if let Some(v) = get("key_side=") {
                st.key_side = v != "0";
            } else if let Some(v) = get("key_rhold=") {
                st.key_rhold = v != "0";
            }
        }
    }
    apply_theme(st);
    apply_key_opts(st);
}

/// 重新解码全部应用图标（增删应用后调用）
unsafe fn load_icons(st: &mut State) {
    for a in st.apps.iter_mut() {
        a.grad = grad_for(&a.name);
        // `decode_icon` 返回**基础纹理**（ICON_PX），这里包成 mip 链：
        // 绘制时才能取到接近目标边长的那一级，避免一次 2.8:1 的大缩小。
        a.icon = decode_icon(&a.target).map(|(px, d)| IconTex::from_base(px, d));
        // 外环条目也要真图标（与内环同一套管线）。URL / ms-* / shell:* / 文件夹
        // `decode_icon` 内部都已经处理；失败时画首字母兜底。
        for it in a.items.iter_mut() {
            it.icon = decode_icon(&it.target).map(|(px, d)| IconTex::from_base(px, d));
        }
    }
}

/// 往 settings.ini 的 [设置] 段里写一个布尔开关（已存在则替换）。
/// ⚠️ 已被 save_settings 取代（设置页成为唯一入口），保留仅作历史参考。
#[allow(dead_code)]
unsafe fn write_ini_flag(path: &str, key: &str, val: bool) {
    let Ok(txt) = std::fs::read_to_string(path) else {
        return;
    };
    let line = format!("{}={}", key, if val { 1 } else { 0 });
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for l in txt.lines() {
        let t = l.trim();
        if t.starts_with(&format!("{}=", key)) {
            if !replaced {
                out.push(line.clone());
                replaced = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !replaced {
        // 插到 [设置] 段首；没有该段就追加到文件末尾
        match out.iter().position(|l| l.trim() == "[设置]") {
            Some(i) => out.insert(i + 1, line),
            None => {
                out.push(String::new());
                out.push("[设置]".into());
                out.push(line);
            }
        }
    }
    let _ = std::fs::write(path, out.join("\r\n"));
}

unsafe fn load_ini(st: &mut State) {
    st.apps.clear();
    if std::path::Path::new(&st.ini_path).exists() {
        if let Ok(text) = std::fs::read_to_string(&st.ini_path) {
            // 0=无 1=[apps] 2=[app.X]
            let mut mode = 0usize;
            let mut cur: Option<usize> = None;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    let sec = line.trim_start_matches('[').trim_end_matches(']').trim();
                    if sec.eq_ignore_ascii_case("apps") {
                        mode = 1;
                    } else if let Some(name) = sec.strip_prefix("app.") {
                        mode = 2;
                        let name = name.trim();
                        cur = st.apps.iter().position(|a| a.name == name);
                    } else {
                        mode = 0;
                    }
                    continue;
                }
                if mode == 0 || line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    if k.is_empty() || v.is_empty() {
                        continue;
                    }
                    match mode {
                        1 => {
                            if st.apps.len() < MAX_APPS {
                                st.apps.push(AppEntry {
                                    name: k.to_string(),
                                    target: v.to_string(),
                                    icon: None,
                                    items: Vec::new(),
                                    grad: grad_for(k),
                                });
                            }
                        }
                        2 => {
                            if let Some(ai) = cur {
                                if st.apps[ai].items.len() < MAX_ITEMS {
                                    // 配色覆盖编码在 target 尾部：`target|N`
                                    let (target, color) = match v.rsplit_once('|') {
                                        Some((t, s)) if !t.is_empty() => {
                                            match s.parse::<usize>() {
                                                Ok(n) if n < GRADS.len() => {
                                                    (t, Some(n))
                                                }
                                                _ => (v, None),
                                            }
                                        }
                                        _ => (v, None),
                                    };
                                    st.apps[ai].items.push(ItemEntry {
                                        icon: None,
                                        name: k.to_string(),
                                        target: target.to_string(),
                                        color,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    } else {
        let _ = std::fs::write(&st.ini_path, default_ini());
        load_ini(st); // 写完默认配置后走统一解析
        return;
    }
    if st.apps.len() > MAX_APPS {
        st.apps.truncate(MAX_APPS);
    }
    load_icons(st);
}

// ───────────────────── 图标提取 ─────────────────────
/// 图标内容的 alpha 外接框。图标位图四周常带透明留白（实测 `cmd` 的内容只有
/// 82×68、`mspaint` 82×62，其余 82×82）—— 不裁掉的话「光学尺寸」各不一样，
/// 在环上看起来就是有的满格、有的偏小，整体显得不整齐。
fn alpha_bbox(bits: &[u8], bw: usize, bh: usize) -> (usize, usize, usize, usize) {
    const THR: u8 = 8;
    let (mut x0, mut y0, mut x1, mut y1) = (bw, bh, 0usize, 0usize);
    for y in 0..bh {
        for x in 0..bw {
            if bits[(y * bw + x) * 4 + 3] > THR {
                if x < x0 { x0 = x; }
                if y < y0 { y0 = y; }
                if x + 1 > x1 { x1 = x + 1; }
                if y + 1 > y1 { y1 = y + 1; }
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        (0, 0, bw, bh)
    } else {
        (x0, y0, x1, y1)
    }
}

/// box-filter 把图标缩放进 tw×tw（BGRA）。tw 必须 **≤ 内容尺寸**（放大造不出细节）。
///
/// 两步：① 按 alpha 外接框裁掉透明留白；② 等比缩放并**居中**放进 tw×tw。
fn normalize_to(bits: &[u8], bw: usize, bh: usize, tw: usize) -> Vec<u8> {
    let mut norm = vec![0u8; tw * tw * 4];
    if bw == 0 || bh == 0 {
        return norm;
    }
    // 面积平均（box filter），不是双线性。
    // 双线性在下采样时只取 4 个纹素 = 点采样，会丢细节 + 产生锯齿，
    // 看上去就是「糊」。这里对每个目标像素覆盖的源矩形做加权平均。
    //
    // 关键：在**预乘空间**累加。图标透明区域 RGB=0，若直接平均 RGB，
    // 边缘会被拉黑出现脏边；先乘 alpha 平均后再除回去才对。
    let (cx0, cy0, cx1, cy1) = alpha_bbox(bits, bw, bh);
    let cw = (cx1 - cx0) as f32;
    let ch = (cy1 - cy0) as f32;
    // ⚠️⚠️ 用 **cover**（短边撑满 tw），**不是 contain**（长边撑满、短边留白）。
    //
    // 为什么必须 cover：③ 的圆角裁切作用在**整个 tw×tw 框**上。用 contain 时，内容不是
    // 正方的图标（cmd 254×210、文件夹 224×176、elasticsearch 256×192…）会在上下（或左右）
    // 留出透明带，圆角的那段弧正好落在透明带上 —— **图标自己的直角就原样露出来了**，
    // 于是环上混着直角图标和圆角图标（用户 2026-10-08 报「有的是直角有的是圆角」）。
    // 改成 cover 后每一格的轮廓都是同一个圆角方块，只有长边溢出的一点点被裁掉
    // （Windows 图标长宽比都在 1.4 以内 → 每边裁 ≤12%，且都是图标自身的留白）。
    //
    // 极端长宽比（>1.6，实际没遇到过）时退回 contain，免得硬裁把内容裁没。
    let aspect = cw.max(ch) / cw.min(ch).max(1.0);
    let cover = aspect <= 1.6;
    let k = if cover {
        tw as f32 / cw.min(ch).max(1.0)
    } else {
        tw as f32 / cw.max(ch).max(1.0)
    };
    let ox = (tw as f32 - cw * k) * 0.5;
    let oy = (tw as f32 - ch * k) * 0.5;
    // cover 时源范围会越出内容外框 → **夹到外框**（不是夹到纹理边，更不能放任取到透明区），
    // 内容才会一直铺到格子边缘；否则最外一两圈会取样到透明 → 边缘留一圈白、轮廓又变小。
    let (lo_x, hi_x, lo_y, hi_y) = if cover {
        (cx0 as f32, cx1 as f32, cy0 as f32, cy1 as f32)
    } else {
        (0.0, bw as f32, 0.0, bh as f32)
    };
    for ny in 0..tw {
        let y0f = cy0 as f32 + (ny as f32 - oy) / k;
        let y1f = cy0 as f32 + ((ny + 1) as f32 - oy) / k;
        let iy0 = y0f.floor().max(lo_y) as usize;
        let iy1 = (y1f.ceil()).max(iy0 as f32).min(hi_y) as usize;
        for nx in 0..tw {
            let x0f = cx0 as f32 + (nx as f32 - ox) / k;
            let x1f = cx0 as f32 + ((nx + 1) as f32 - ox) / k;
            let ix0 = x0f.floor().max(lo_x) as usize;
            let ix1 = (x1f.ceil()).max(ix0 as f32).min(hi_x) as usize;
            let mut acc = [0.0f32; 4]; // 预乘后的 R,G,B,A
            let mut wsum = 0.0f32;
            for yy in iy0..iy1 {
                let wy = (yy as f32 + 1.0).min(y1f) - (yy as f32).max(y0f);
                if wy <= 0.0 {
                    continue;
                }
                for xx in ix0..ix1 {
                    let wx = (xx as f32 + 1.0).min(x1f) - (xx as f32).max(x0f);
                    if wx <= 0.0 {
                        continue;
                    }
                    let wt = wx * wy;
                    let so = (yy * bw + xx) * 4;
                    let a = bits[so + 3] as f32 / 255.0;
                    acc[0] += bits[so + 2] as f32 * a * wt; // R
                    acc[1] += bits[so + 1] as f32 * a * wt; // G
                    acc[2] += bits[so] as f32 * a * wt;     // B
                    acc[3] += a * wt;
                    wsum += wt;
                }
            }
            let o = (ny * tw + nx) * 4;
            if wsum > 0.0 && acc[3] > 1e-5 {
                norm[o] = (acc[2] / acc[3]).clamp(0.0, 255.0) as u8; // B
                norm[o + 1] = (acc[1] / acc[3]).clamp(0.0, 255.0) as u8; // G
                norm[o + 2] = (acc[0] / acc[3]).clamp(0.0, 255.0) as u8; // R
                norm[o + 3] = ((acc[3] / wsum) * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
    // ③ 统一裁成**圆角方块**：图标自带轮廓千奇百怪 —— 圆形标、白纸、**直角方块**
    //    （设置 / 计算器就是 100% 不透明的直角方块，角落是尖的）。
    //    不裁的话环上会混着尖角方块和圆角，完全不是一个语言；
    //    裁完每格都是同一个圆角方形，这才是参考图那种「整齐」。
    // 圆角半径用 TILE_RAD_K × 边长 × TILE_PAD_K。
    // 为什么要再乘 PAD：纹理被画到「底板 ÷ PAD」的尺寸上，而底板圆角是
    // TILE_RAD_K × 底板边长。只按自身边长取半径的话，缩放后图标圆角会比
    // 底板小 PAD 倍 → 两者对不齐、边上露白缝。乘上后正好齐平。
    {
        // cover：裁切框 = 整个 tw×tw（内容已经铺满它，跟底板圆角天然对齐）。
        // 非 cover（极端长宽比）：裁切框贴着**内容外框**、半径按短边取 ——
        // 这样内容自己的直角一定会被倒掉，绝不会出现「方角图标」。
        let (fw, fh) = if cover { (tw as f32, tw as f32) } else { (cw * k, ch * k) };
        let half = tw as f32 * 0.5;
        let half_w = fw * 0.5;
        let half_h = fh * 0.5;
        let rad = fw.min(fh) * TILE_RAD_K * TILE_TEX_PAD_K;
        for ny in 0..tw {
            for nx in 0..tw {
                let o = (ny * tw + nx) * 4;
                if norm[o + 3] == 0 {
                    continue;
                }
                // cov1 要「内部为负」，而 sdf_rounded_rect 是「内部为正」→ 取负
                let cov = cov1(-sdf_rounded_rect(
                    nx as f32 + 0.5 - half,
                    ny as f32 + 0.5 - half,
                    half_w,
                    half_h,
                    rad,
                ));
                if cov < 0.999 {
                    norm[o + 3] = (norm[o + 3] as f32 * cov) as u8;
                }
            }
        }
    }
    norm
}

/// 高分辨率图标：走 shell 的 IShellItemImageFactory（Explorer 同款管线）。
/// `PrivateExtractIconsW` 对 explorer/notepad/cmd/calc/taskmgr 只给 32×32，
/// 放大画到 45px 必然发虚 —— 这是图标模糊的根因。
unsafe fn shell_icon_hires(exe_path: &str) -> Option<Vec<u8>> {
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };
    let dbg = std::env::var_os("RL_ICONDBG").is_some();
    let wt = wide(exe_path);
    let factory: IShellItemImageFactory =
        match SHCreateItemFromParsingName(PCWSTR(wt.as_ptr()), None) {
            Ok(f) => f,
            Err(e) => {
                if dbg {
                    eprintln!("[icon] hires parse FAIL {:?} <- {}", e.code(), exe_path);
                }
                return None;
            }
        };
    let hbmp = factory
        .GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)
        .ok()?;
    if hbmp.0.is_null() {
        return None;
    }
    let (iw, ih) = (256i32, 256i32);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = iw;
    bmi.bmiHeader.biHeight = -ih;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    let mut bits = vec![0u8; (iw * ih * 4) as usize];
    let hdc = GetDC(None);
    let got = GetDIBits(
        hdc,
        hbmp,
        0,
        ih as u32,
        Some(bits.as_mut_ptr() as *mut c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    let _ = ReleaseDC(None, hdc);
    let _ = DeleteObject(HGDIOBJ(hbmp.0));
    if got == 0 {
        return None;
    }
    let opaque = bits.chunks_exact(4).filter(|c| c[3] == 0).count();
    if dbg {
        eprintln!(
            "[icon] hires bmp get={} opaquePx={}/{} <- {}",
            got,
            opaque,
            256 * 256,
            exe_path
        );
    }
    if !bits.chunks_exact(4).any(|c| c[3] != 0) {
        return None;
    }
    if dbg {
        eprintln!("[icon] hires 256x256  <- {}", exe_path);
    }
    // 一次性诊断：把 shell 给的**原始位图**和**归一化后的纹理**分别落盘，
    // 用来判断「图标糊」到底来自源图还是我方缩放。
    if let Some(dir) = std::env::var_os("RL_ICON_DUMP") {
        let dir = dir.to_string_lossy().to_string();
        let tag: String = exe_path
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let _ = save_bmp(&format!("{}\\raw_{}.bmp", dir, tag), &bits, iw, ih);
        let _ = save_rgba(&format!("{}\\raw_{}.rgba", dir, tag), &bits);
        let n = normalize_to(&bits, iw as usize, ih as usize, ICON_PX);
        let _ = save_bmp(
            &format!("{}\\norm_{}.bmp", dir, tag),
            &n,
            ICON_PX as i32,
            ICON_PX as i32,
        );
        let _ = save_rgba(&format!("{}\\norm_{}.rgba", dir, tag), &n);
        return Some(n);
    }
    Some(normalize_to(&bits, iw as usize, ih as usize, ICON_PX))
}


/// 从 exe 资源里提取最大的内嵌 PNG 图标（Vista+ 的 256px 图标都是 PNG），
/// 用 WIC 解码 —— 完全绕开 shell 的图标缓存/放大充数，源即所见图。
/// 返回 (边长px, BGRA 直通 alpha)
unsafe fn pe_png_icon(path: &str) -> Option<(usize, Vec<u8>)> {
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, IWICImagingFactory, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};

    let d = std::fs::read(path).ok()?;
    if d.len() < 0x100 || &d[..2] != b"MZ" {
        return None;
    }
    let e = u32::from_le_bytes(d[0x3c..0x40].try_into().ok()?) as usize;
    if d.get(e..e + 4) != Some(b"PE\x00\x00") {
        return None;
    }
    let coff = e + 4;
    let nsec = u16::from_le_bytes(d[coff + 2..coff + 4].try_into().ok()?) as usize;
    let opt_size = u16::from_le_bytes(d[coff + 16..coff + 18].try_into().ok()?) as usize;
    let magic = u16::from_le_bytes(d[coff + 20..coff + 22].try_into().ok()?) as u16;
    let dd_off = coff + 20 + if magic == 0x20b { 112 } else { 96 };
    if dd_off + 24 > d.len() {
        return None;
    }
    let res_rva = u32::from_le_bytes(d[dd_off + 16..dd_off + 20].try_into().ok()?) as usize;
    if res_rva == 0 {
        return None;
    }
    let sec_off = coff + 20 + opt_size;
    let mut secs: Vec<(usize, usize, usize)> = Vec::new();
    for i in 0..nsec {
        let s = sec_off + i * 40;
        if s + 40 > d.len() {
            break;
        }
        let va = u32::from_le_bytes(d[s + 12..s + 16].try_into().ok()?) as usize;
        let sz = u32::from_le_bytes(d[s + 8..s + 12].try_into().ok()?) as usize;
        let raw = u32::from_le_bytes(d[s + 20..s + 24].try_into().ok()?) as usize;
        secs.push((va, sz, raw));
    }
    let rva2off = |rva: usize| -> Option<usize> {
        for (va, sz, raw) in &secs {
            if rva >= *va && rva < va + (*sz).max(1) {
                return Some(raw + (rva - *va));
            }
        }
        None
    };
    let r = rva2off(res_rva)?;
    // 目录：8 项 16 字节头 + 每项 8 字节（NameOrId, OffsetToData|0x80000000=子目录）
    let dir_entries = |off: usize| -> Option<Vec<(u32, u32)>> {
        if off + 16 > d.len() {
            return None;
        }
        let named = u16::from_le_bytes(d[off + 12..off + 14].try_into().ok()?) as usize;
        let ided = u16::from_le_bytes(d[off + 14..off + 16].try_into().ok()?) as usize;
        let mut v = Vec::new();
        for i in 0..named + ided {
            let e2 = off + 16 + i * 8;
            if e2 + 8 > d.len() {
                return None;
            }
            let name = u32::from_le_bytes(d[e2..e2 + 4].try_into().ok()?);
            let dvo = u32::from_le_bytes(d[e2 + 4..e2 + 8].try_into().ok()?);
            v.push((name, dvo));
        }
        Some(v)
    };
    // RT_ICON = 3：类型 → ID → 语言 → 数据
    let mut best: Option<(usize, usize, usize)> = None; // (png_len, off, 数据项里的估算尺寸)
    for (typ, dvo) in dir_entries(r)? {
        if typ != 3 || dvo & 0x8000_0000 == 0 {
            continue;
        }
        let l1 = rva2off(res_rva + (dvo & 0x7fff_ffff) as usize)?;
        for (_, dvo2) in dir_entries(l1)? {
            if dvo2 & 0x8000_0000 == 0 {
                continue;
            }
            let l2 = rva2off(res_rva + (dvo2 & 0x7fff_ffff) as usize)?;
            for (_, dvo3) in dir_entries(l2)? {
                if dvo3 & 0x8000_0000 != 0 {
                    continue;
                }
                let de = rva2off(res_rva + dvo3 as usize)?;
                if de + 8 > d.len() {
                    continue;
                }
                let drva = u32::from_le_bytes(d[de..de + 4].try_into().ok()?) as usize;
                let dsize = u32::from_le_bytes(d[de + 4..de + 8].try_into().ok()?) as usize;
                let doff = rva2off(drva)?;
                if doff + 8 > d.len() {
                    continue;
                }
                // PNG magic: 89 50 4E 47 0D 0A 1A 0A
                if d[doff] == 0x89 && d[doff + 1] == 0x50 && d[doff + 2] == 0x4E && d[doff + 3] == 0x47 {
                    if best.map(|(l, _, _)| dsize > l).unwrap_or(true) {
                        best = Some((dsize, doff, 0));
                    }
                }
            }
        }
    }
    let (_, off, _) = best?;
    let png = d.get(off..off + best.unwrap().0)?.to_vec();

    // WIC 解码 PNG → BGRA 直通 alpha
    let factory: IWICImagingFactory =
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let stream = factory.CreateStream().ok()?;
    if stream.Write(png.as_ptr() as *const c_void, png.len() as u32, None).is_err() {
        return None;
    }
    let decoder = factory
        .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
        .ok()?;
    let frame = decoder.GetFrame(0).ok()?;
    let mut w = 0u32;
    let mut h = 0u32;
    frame.GetSize(&mut w, &mut h).ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    let mut buf = vec![0u8; (w * h * 4) as usize];
    frame.CopyPixels(std::ptr::null(), w * 4, &mut buf).ok()?;
    Some((w as usize, buf))
}

/// SHGetFileInfoW / PrivateExtractIconsW 提取 → AND 掩码重建透明度 → 归一化
unsafe fn extract_icon_bits(wt: &[u16]) -> Option<Vec<u8>> {
    let path_str = path_str_of(wt);
    let hicon = if std::path::Path::new(&path_str).exists() {
        let mut h = [HICON(ptr::null_mut())];
        let mut id = 0u32;
        let mut fname = [0u16; 260];
        let cp = wt.len().min(259);
        fname[..cp].copy_from_slice(&wt[..cp]);
        let n = PrivateExtractIconsW(
            &fname,
            0,
            256,
            256,
            Some(&mut h),
            Some(&mut id as *mut u32),
            0,
        );
        if n == 0 || h[0].0.is_null() {
            return None;
        }
        h[0]
    } else {
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            PCWSTR(wt.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut sfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        );
        if r == 0 {
            return None;
        }
        sfi.hIcon
    };
    let mut ii = ICONINFO::default();
    if GetIconInfo(hicon, &mut ii).is_err() {
        let _ = DestroyIcon(hicon);
        return None;
    }
    let mut out: Option<Vec<u8>> = None;
    if !ii.hbmColor.0.is_null() {
        let (mut bw, mut bh) = (32i32, 32i32);
        let mut bm = BITMAP::default();
        if GetObjectW(
            ii.hbmColor,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut c_void),
        ) > 0
        {
            bw = bm.bmWidth;
            bh = bm.bmHeight.abs();
        }
        if std::env::var_os("RL_ICONDBG").is_some() {
            eprintln!("[icon] native {}x{}  <- {}", bw, bh, path_str);
        }
        if bw > 0 && bw <= 256 && bh > 0 && bh <= 256 {
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = bw;
            bmi.bmiHeader.biHeight = -bh;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let n_px = (bw * bh) as usize;
            let mut bits = vec![0u8; n_px * 4];
            let hdc = GetDC(None);
            let got = GetDIBits(
                hdc,
                ii.hbmColor,
                0,
                bh as u32,
                Some(bits.as_mut_ptr() as *mut c_void),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if got > 0 {
                let mut mbmi = BITMAPINFO::default();
                mbmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                mbmi.bmiHeader.biWidth = bw;
                mbmi.bmiHeader.biHeight = -(bh * 2);
                mbmi.bmiHeader.biPlanes = 1;
                mbmi.bmiHeader.biBitCount = 1;
                let mask_row = ((bw + 31) / 32 * 4) as usize;
                let mut mbits = vec![0u8; mask_row * bh as usize * 2];
                let mask_ok = GetDIBits(
                    hdc,
                    ii.hbmMask,
                    0,
                    (bh * 2) as u32,
                    Some(mbits.as_mut_ptr() as *mut c_void),
                    &mut mbmi,
                    DIB_RGB_COLORS,
                ) > 0;
                let coverage = bits.iter().skip(3).step_by(4).filter(|&&a| a > 0).count();
                if coverage < n_px * 3 / 10 && mask_ok {
                    for y in 0..bh as usize {
                        for x in 0..bw as usize {
                            let i = y * bw as usize + x;
                            if bits[i * 4 + 3] == 0 {
                                let transparent =
                                    mbits[y * mask_row + x / 8] & (0x80 >> (x % 8)) != 0;
                                bits[i * 4 + 3] = if transparent { 0 } else { 255 };
                            }
                        }
                    }
                }
                if bits.chunks_exact(4).any(|c| c[3] != 0) {
                    out = Some(normalize_to(&bits, bw as usize, bh as usize, 32));
                }
            }
            let _ = ReleaseDC(None, hdc);
        }
    }
    let _ = DeleteObject(ii.hbmColor);
    let _ = DeleteObject(ii.hbmMask);
    let _ = DestroyIcon(hicon);
    out
}

/// 提取通用文档图标（无图标 exe 的回退）作为基准，进程内只算一次
unsafe fn generic_exe_icon() -> &'static Option<Vec<u8>> {
    static G: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    G.get_or_init(|| extract_icon_bits(&wide("@@@generic@@@.zzz")))
}

unsafe fn default_browser_exe() -> Option<String> {
    let mut buf = [0u16; 520];
    let mut len = buf.len() as u32;
    let proto = wide("http");
    let hr = AssocQueryStringW(
        ASSOCF_NONE,
        ASSOCSTR_EXECUTABLE,
        PCWSTR(proto.as_ptr()),
        PCWSTR::null(),
        windows::core::PWSTR(buf.as_mut_ptr()),
        &mut len,
    );
    if hr.is_err() || len < 2 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(len - 1) as usize]))
}

unsafe fn default_browser_icon() -> Option<(usize, Vec<u8>)> {
    let exe = default_browser_exe()?;
    decode_icon(&exe)
}

/// `shell:Downloads` / `shell:Desktop` / `shell:Personal`… → 真实文件夹路径。
/// 认不出来（或路径不存在）就返回 `None`，调用方退回命名空间解析。
fn shell_ns_to_path(target: &str) -> Option<String> {
    let home = std::env::var("USERPROFILE").ok()?;
    let sub = match target.get(6..)?.trim().to_ascii_lowercase().as_str() {
        "desktop" => "Desktop",
        "personal" | "mydocuments" | "documents" => "Documents",
        "downloads" => "Downloads",
        "pictures" | "mypictures" => "Pictures",
        "music" | "mymusic" => "Music",
        "videos" | "myvideos" => "Videos",
        "favorites" => "Favorites",
        _ => return None,
    };
    let p = format!("{}{}{}", home.trim_end_matches('\\'), '\\', sub);
    if std::path::Path::new(&p).exists() {
        Some(p)
    } else {
        None
    }
}

unsafe fn decode_icon(target: &str) -> Option<(usize, Vec<u8>)> {
    // ── 1) URL / URI / shell 命名空间 ──
    // 关键：绝不能把 URL 本体交给 shell —— SIIGBF_ICONONLY 对网站返回的是
    // 16/32px 的 favicon，被放大到 256 就是用户看到的「糊」。网站书签一律
    // 用默认浏览器 exe 的内嵌 256px PNG（PE 直读），统一且锐利。
    if target.contains("://") || target.starts_with("ms-") || target.starts_with("shell:") {
        if target.starts_with("http") {
            if let Some(exe) = default_browser_exe() {
                if let Some(v) = pe_png_icon(&exe) {
                    return Some(v);
                }
                if let Some(v) = shell_icon_hires(&exe) {
                    return Some((ICON_PX, v));
                }
            }
            return default_browser_icon();
        }
        if target.starts_with("shell:") {
            // `shell:Downloads` / `shell:Desktop` / `shell:Personal`…（之前这里
            // `return None` → 只剩首字母兜底）。
            //
            // ⚠️ **优先换成真实文件夹路径再取图标**：直接用命名空间解析，
            // `shell:Desktop` 会返回一张**全出血的渐变方块**（不是文件夹形状，
            // 小尺寸下就是一块蓝方块），而真实路径给的是正常的文件夹图标；
            // `shell:Downloads` / `shell:Personal` 两者一致，所以统一走路径。
            if let Some(p) = shell_ns_to_path(target) {
                if let Some(v) = shell_icon_hires(&p) {
                    return Some((ICON_PX, v));
                }
            }
            if let Some(v) = shell_icon_hires(target) {
                return Some((ICON_PX, v));
            }
        }
        if target.starts_with("ms-settings") {
            for p in [
                "C:\\Windows\\ImmersiveControlPanel\\SystemSettings.exe",
                "C:\\Windows\\System32\\SystemSettings.exe",
            ] {
                if let Some(v) = pe_png_icon(p) {
                    return Some(v);
                }
                if let Some(v) = shell_icon_hires(p) {
                    return Some((ICON_PX, v));
                }
            }
        }
        return None;
    }

    // ── 2) 解析真实文件路径：SearchPathW → System32 → Windows 目录 ──
    let mut full: Vec<u16> = Vec::new();
    if !target.contains('\\') {
        let name = wide(target);
        let mut buf = [0u16; 520];
        let mut part = windows::core::PWSTR::null();
        let n = windows::Win32::Storage::FileSystem::SearchPathW(
            None,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut buf),
            Some(&mut part),
        );
        if n > 0 && (n as usize) <= buf.len() {
            full = buf[..n as usize].to_vec();
        }
        // 商店应用别名（WindowsApps\mspaint.exe）是 reparse 点，但 shell 能把它
        // 解析成真实 App 图标 —— 先交给 shell，不要直接放弃
        if !full.is_empty() {
            let resolved = path_str_of(&full);
            if resolved.contains("WindowsApps") {
                if let Some(v) = shell_icon_hires(&resolved) {
                    return Some((ICON_PX, v));
                }
                return None;
            }
        }
        if full.is_empty() {
            for dir in ["C:\\Windows\\System32\\", "C:\\Windows\\"] {
                let cand = format!("{dir}{target}");
                if std::path::Path::new(&cand).exists() {
                    full = wide(&cand);
                    break;
                }
            }
        }
        if full.is_empty() {
            return None;
        }
    } else if !std::path::Path::new(target).exists() {
        return None;
    }
    let wt = if full.is_empty() { wide(target) } else { full };
    let path = path_str_of(&wt);

    // ── 3a) 最优先：exe 内嵌的 PNG 大图标（现代应用都有 256px），WIC 直解码
    //    shell 管线对部分路径会走图标缓存甚至把 32px 放大充数，均不可依赖
    if let Some((w, buf)) = pe_png_icon(&path) {
        let tw = w.min(ICON_PX); // 只缩小不放大
        return Some((tw, normalize_to(&buf, w, w, tw)));
    }

    // ── 3b) shell 高分辨率（连 reparse 存根也能解析成真实图标）──
    if let Some(v) = shell_icon_hires(&path) {
        return Some((ICON_PX, v));
    }

    // ── 4) 到这一步还拿不到，且是 reparse 存根 → 只能提出假图标，放弃 ──
    {
        use windows::Win32::Storage::FileSystem::{
            GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT,
        };
        let attrs = GetFileAttributesW(PCWSTR(wt.as_ptr()));
        if attrs != u32::MAX && attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return None;
        }
    }

    // ── 5) 32px 回退 ──
    let bits = extract_icon_bits(&wt)?;
    let g = generic_exe_icon().as_ref();
    if g == Some(&bits) {
        return None;
    }
    // ⚠️ 这里以前直接返回 32px 原始位图 —— **绕过了 normalize_to**，于是这条路径的图标
    // 既没裁掉透明留白、也没倒圆角，画出来就是「直角方块 + 可能偏心」。
    // 现在也过一遍（tw=32 不放大，只做「按 alpha 外接框裁切 + 圆角方块」）。
    Some((32, normalize_to(&bits, 32, 32, 32)))
}

// ═══════════════════════════════════════════════════════════════════
//  重建段 B：布局 / 钩子 / 交互 / 生命周期 / 窗口过程 / main
// ═══════════════════════════════════════════════════════════════════

// ───────────────────── GDI 资源 ─────────────────────

unsafe fn make_dib(w: i32, h: i32) -> (*mut u8, HDC, HBITMAP) {
    let sdc = GetDC(None);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;
    let mut bits: *mut c_void = ptr::null_mut();
    let bmp = match CreateDIBSection(
        sdc,
        &bmi as *const _ as *const BITMAPINFO,
        DIB_RGB_COLORS,
        &mut bits,
        None,
        0,
    ) {
        Ok(b) => b,
        Err(_) => {
            let _ = ReleaseDC(None, sdc);
            return (ptr::null_mut(), HDC(ptr::null_mut()), HBITMAP(ptr::null_mut()));
        }
    };
    let memdc = CreateCompatibleDC(sdc);
    let _ = ReleaseDC(None, sdc);
    let _ = SelectObject(memdc, HGDIOBJ(bmp.0));
    std::ptr::write_bytes(bits as *mut u8, 0, (w * h * 4) as usize);
    (bits as *mut u8, memdc, bmp)
}

unsafe fn init_surfaces(st: &mut State) {
    // 扁平化后不再抓屏模糊 —— 背景 DIB 仅 --dump 的合成壁纸会用到，
    // 常规运行时省下一块画布尺寸的缓冲
    if st.dump_bg {
        let (b, d, m) = make_dib(CANVAS, CANVAS);
        st.bg_bits = b;
        st.bg_dc = d;
        st.bg_bmp = m;
    }
    let (b2, d2, m2) = make_dib(CANVAS, CANVAS);
    st.dbi_bits = b2;
    st.dbi_dc = d2;
    st.dbi_bmp = m2;
    let (b3, d3, m3) = make_dib(CANVAS, 140);
    st.txt_bits = b3;
    st.txt_dc = d3;
    st.txt_bmp = m3;
}

unsafe fn init_fonts(st: &mut State) {
    let face: Vec<u16> = wide("Microsoft YaHei UI");
    st.font_name = CreateFontW(-(15.0 * K).round() as i32, 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_label = CreateFontW(-(11.0 * K).round() as i32, 0, 0, 0, 500, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_row = CreateFontW(-(13.0 * K * KC).round() as i32, 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_hint = CreateFontW(-(10.0 * K).round() as i32, 0, 0, 0, 400, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_head = CreateFontW(-(16.0 * K * KC).round() as i32, 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_cnt = CreateFontW(-(12.0 * K).round() as i32, 0, 0, 0, 500, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_glyph = CreateFontW(-(22.0 * K).round() as i32, 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    // 3 倍超采样字体：与上面各字号一一对应 ×3（draw_text_rot 旋转文字防糊）
    st.font_name3 = CreateFontW(-(45.0 * K).round() as i32, 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_label3 = CreateFontW(-(33.0 * K).round() as i32, 0, 0, 0, 500, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_row3 = CreateFontW(-(39.0 * K * KC).round() as i32, 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_hint3 = CreateFontW(-(30.0 * K).round() as i32, 0, 0, 0, 400, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_head3 = CreateFontW(-(48.0 * K * KC).round() as i32, 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_cnt3 = CreateFontW(-(36.0 * K).round() as i32, 0, 0, 0, 500, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.font_glyph3 = CreateFontW(-(66.0 * K).round() as i32, 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    // 设置窗口字体（固定 px，×SS 在超采样画布上用）
    st.sf_title = CreateFontW(-(16 * SS), 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.sf_nav = CreateFontW(-(14 * SS), 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.sf_body = CreateFontW(-(13 * SS), 0, 0, 0, 600, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.sf_small = CreateFontW(-(12 * SS), 0, 0, 0, 400, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
    st.sf_bold = CreateFontW(-(14 * SS), 0, 0, 0, 700, 0, 0, 0, 134, 0, 0, 2, 0, PCWSTR(face.as_ptr()));
}

// ───────────────────── 径向布局 ─────────────────────

/// 应用方位角（0 = 正上方，顺时针）
fn app_angle(i: usize, n: usize) -> f32 {
    i as f32 * (360.0 / n as f32)
}

/// 方位角（0=上，顺时针）→ 画布坐标
fn polar(r: f32, deg: f32) -> (f32, f32) {
    let a = deg.to_radians();
    (CC + r * a.sin(), CC - r * a.cos())
}

/// 估算文字宽度（CJK 按全宽，其余按半宽）
fn est_text_w(text: &str, cjk_w: f32, lat_w: f32) -> f32 {
    text.chars().map(|c| if c as u32 > 0x2E7F { cjk_w } else { lat_w }).sum()
}

/// 内环自适应布局：返回 (图标环半径, 图标绘制边长)。
///
/// 规则（和 `chip_layout` 同一套思路 —— 布局由「内容尺寸」反推，不由常量钉死）：
/// 1. 先按**最大图标**求半径：相邻两格的弦长必须容得下「方块切向投影 + 目标间隙」
///    `2·r·sin(π/n) ≥ icon·TANG + icon·GAP_K`；
/// 2. 半径超过 `DISC_R_MAX` 就封顶（菜单不能无限大），此时**反过来缩图标**保间隙 ——
///    宁肯图标小一点，也不要挤成一片。
///
/// 旧实现的目标只是「刚好不相撞」（`icon·1.25 + 6`），所以 12 个应用时
/// 间隙只有 0.28×图标，视觉上就是「贴在一起」。
fn ring_layout(n: usize) -> (f32, f32) {
    let hs = (std::f32::consts::PI / n.max(1) as f32).sin();
    let mut icon = ICON_DRAW;
    let mut r = icon * (TANG + GAP_K) / (2.0 * hs);
    let r_cap = DISC_R_MAX - icon * 0.5 - BAND_PAD;
    if r > r_cap {
        r = r_cap;
        icon = 2.0 * r * hs / (TANG + GAP_K);
    }
    icon = icon.clamp(ICON_MIN, ICON_DRAW);
    // 条目很少时别把环缩到盘心（要给中心头像留位置）
    let r = r.max(R_ICON_MIN).max(HUB_R + icon * 0.5 + 10.0);
    (r, icon)
}

/// 盘面半径：图标环 + 图标半宽 + 环带留白。**绘制、命中、chip 环、压暗范围必须都走这里**，
/// 否则「画在哪」和「点到哪」又会分家。
fn disc_r(n: usize) -> f32 {
    let (r, icon) = ring_layout(n);
    r + icon * 0.5 + BAND_PAD
}

/// 环带内沿（中心留白区的半径）。
/// **压到头像球底下**（`HUB_R - 5`）—— 头像周围不留白圈：球是后画的，会盖住这一圈，
/// 于是视觉上「灰色环带 → 球」直接相接。留 5px 余量是为了呼吸动画
/// （球半径会缩 ~2.4%，不留余量会在球最小时露出一道白边）。
/// 环带内沿半径 —— 压到头像球底下（球是后画的会盖住这一圈）。
/// 留 5px 是因为呼吸动画会让球半径缩 ~2.4%。
fn band_in(n: usize) -> f32 {
    let (r, icon) = ring_layout(n);
    (HUB_R - 5.0).min(r - icon * 0.5 - 6.0).max(HUB_R * 0.5)
}

// ───────────────────── 外环：梯形环（圆角环形扇区）─────────────────────
//
// 形态模仿 StarPie 的「经典紧凑扇区」：环带切成 m 段，每段 = 两条**直的径向边**
// + 内弧 + 外弧，四角大圆角，段间留一条角度缝。
//
// 颜色**与内环完全一致**（§2「统一圆角方块」的那套）：白底 + `mix(ink 34%)` 描边
// + `0 3K` 硬投影。整个界面里唯一有颜色的东西是**真实图标**，内环也是这个规矩。
//
// **排布 = 「甲」方案（用户 2026-10-08 拍板）**：段仍然**贴身**铺在对应内环图标
// 的方位角两侧（`chip_layout` 的贴身扇面），内容**沿切线横排**（图标在左、名称在右）
// 并跟着弧转 —— 也就是「壳从胶囊换成梯形段，里面一律没动」，改动最小。

/// 外环厚度 = **与原来的胶囊同高**（`CHIP_H`）。
/// 「甲」方案要的是「保持现有排布 + 只换外壳形状」，所以厚度不变。
const RING_T: f32 = CHIP_H;
/// 四角圆角半径 = 厚度 × 这个比例（薄环用更大的比例，跟参考图一致）
const RING_CR_K: f32 = 0.30;
/// hover 时沿**径向向外**抬起的距离系数（段与段内内容必须用同一个，否则会脱开）
const RING_LIFT_K: f32 = 2.0;
/// 段里**名称的宽度上限**（px）。段宽正比于名字宽度 —— 不封顶的话一个 18 字符的
/// URL（`newtab.claisly.com`）会让单段吃掉 50°+ 的环，整圈被它一个人占满。
/// 超过就截断加省略号。`chip_half_w` 与段内绘制**必须用同一个上限**，
/// 否则「段宽 / 文字宽 / 命中框」三者会对不上。
const RING_NAME_MAX_W: f32 = 90.0;
/// 段内名称字号（传给 `draw_text_rot`：13 → `font_row`，13*K*KC ≈ 11.5px）
const RING_NAME_PX: i32 = 13;

/// 外环的内外半径（跟着自适应的盘面半径走）
fn ring_radii(n: usize) -> (f32, f32) {
    let r_in = disc_r(n) + 3.0 * K;
    (r_in, r_in + RING_T)
}

/// 圆角环形扇区的 SDF（**内部为负** —— 与项目里所有 `flat_*` 的约定一致）。
///
/// 做法：把像素换算到「(径向 u, 切向 v)」局部坐标，其中 **v 用中间半径的弧长刻度**
/// （`v = Δθ · r_mid`）—— 这样等 v 线正好是**真正的径向直线**（梯形要的就是直边）；
/// 若写成 `v = Δθ · r` 会得到微微弯曲的边。然后当圆角矩形求 SDF 即可。
fn sdf_sector(px: f32, py: f32, cx: f32, cy: f32,
              r_mid: f32, hu: f32, hv: f32, cr: f32, mid_rad: f32) -> f32 {
    let dx = px - cx;
    let dy = py - cy;
    let r = (dx * dx + dy * dy).sqrt();
    // 角度约定与 `polar` 一致：0 = 正上、顺时针
    let th = dx.atan2(-dy);
    let tau = std::f32::consts::TAU;
    let mut d = th - mid_rad;
    d -= tau * (d / tau).round();
    // ⚠️ `sdf_rounded_rect` 是「**内部为正**」的约定，而 `cov1()` 按「内部为负」用
    // （本项目所有 `flat_*` 都取内部为负）。这里必须取负号 ——
    // 忘了取负的后果不是「画小一点」，而是**填充整个包围盒、把梯形挖成洞**
    // （2026-10-08 真机调试时踩到：段变成一片白方框 + 中间透出壁纸）。
    -sdf_rounded_rect(r - r_mid, d * r_mid, hu, hv, cr)
}

/// 画一段梯形环（含硬投影、填充、描边）。
#[allow(clippy::too_many_arguments)]
fn draw_ring_sector(buf: &mut [u8], pal: &Pal, r_in: f32, r_out: f32,
                    seg: (f32, f32, f32), hot: f32, alpha: f32, mode: u8) {
    if alpha <= 0.01 {
        return;
    }
    let (mid_deg, a1, a2) = seg;
    let hu = (r_out - r_in) * 0.5;
    let r0 = (r_in + r_out) * 0.5;                 // 基准中位半径（不含 hover 抬升）
    let alpha_span = (a2 - a1).to_radians();
    let hv = alpha_span * 0.5 * r0;
    let cr = (RING_T * RING_CR_K).min(hu * 0.98).min(hv * 0.98);
    let mid_rad = mid_deg.to_radians();

    // mode：0 = 正常段（白底 + 描边 + 硬投影）
    //       1 = 改名中的段（墨底，无描边无投影）
    //       2 = 提交成功的一次白色闪光
    // hover：**加粗描边 + 沿径向外抬**（内环的选中态也是「加粗描边 + 抬升」，不另画托底）。
    // 抬升直接用「半径 +lift」表达 —— SDF 本来就在极坐标里，不用做屏幕位移。
    let plain = mode != 0;
    // ⚠️ 描边宽度/填充/投影**必须与内环底板用同一套公式**（用户 2026-10-08：
    // 「梯形颜色跟内环要一致」）。底板是 `(1.4 + 1.6*sel)*K` / `pal.paper` /
    // `(3.0+2.0*sel)*K` 偏移 + `0.20+0.10*sel` 强度 —— 这里以前是 2.0/1.1、
    // 填充还掺了 2% 墨、投影只有 0.13，所以两圈看着不是一种材料。
    let edge_w = if plain { 0.0 } else { (1.4 + 1.6 * hot) * K };
    let lift = 2.0 * K * hot;
    let r_mid = r0 + lift;
    // ⚠️ 极坐标原点**只能是画布中心**。之前这里收了一个 `cx, cy` 参数、调用方传的是
    // 「段中心」→ 半径算成了相对段中心的距离，整段被画成一坨巨形（真机一眼就看出）。
    let (cx, cy) = (CC, CC);

    // 包围盒：沿弧采样 + 取内外两个半径（稳、不依赖解析极值）
    let mut minx = f32::MAX;
    let mut maxx = f32::MIN;
    let mut miny = f32::MAX;
    let mut maxy = f32::MIN;
    let steps = ((a2 - a1).abs() / 3.0).ceil().max(2.0) as usize;
    for k in 0..=steps {
        let a = a1 + (a2 - a1) * k as f32 / steps as f32;
        for rr in [r_in + lift, r_out + lift] {
            let (x, y) = polar(rr, a);
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
        }
    }
    let pad = edge_w + 3.0 * K + 3.0;
    let x0 = (minx - pad).floor().max(0.0) as i32;
    let y0 = (miny - pad).floor().max(0.0) as i32;
    let x1 = (maxx + pad).ceil().min(CANVAS as f32 - 1.0) as i32;
    let y1 = (maxy + pad).ceil().min(CANVAS as f32 - 1.0) as i32;

    // 与内环底板同一套硬投影：偏移 `(3.0+2.0*sel)*K`、强度 `0.20+0.10*sel`
    let sh_dy = if plain { 0.0 } else { (3.0 + 2.0 * hot) * K };
    let sh_a = if plain { 0.0 } else { 0.20 + 0.10 * hot };
    // 填充一律 `pal.paper`（纯白）—— 内环底板也是纯白，不能掺墨变灰
    let fill = match mode {
        1 => pal.ink,
        _ => pal.paper,
    };
    let edge = mixc(mixc(pal.paper, pal.ink, 0.34), pal.ink, hot);
    let w = CANVAS as usize;
    for y in y0..=y1 {
        let fy = y as f32 + 0.5;
        for x in x0..=x1 {
            let fx = x as f32 + 0.5;
            let s = sdf_sector(fx, fy, cx, cy, r_mid, hu, hv, cr, mid_rad);
            let o = (y as usize * w + x as usize) * 4;
            let ss = sdf_sector(fx, fy - sh_dy, cx, cy, r_mid, hu, hv, cr, mid_rad);
            let a_sh = cov1(ss) * sh_a * alpha;
            if a_sh > 0.002 {
                blend_over(buf, o, pal.ink.0 as f32, pal.ink.1 as f32, pal.ink.2 as f32, a_sh);
            }
            let a_f = cov1(s) * alpha;
            if a_f > 0.002 {
                blend_over(buf, o, fill.0 as f32, fill.1 as f32, fill.2 as f32, a_f);
            }
            let a_e = cov1(s.abs() - edge_w * 0.5) * alpha;
            if a_e > 0.002 {
                blend_over(buf, o, edge.0 as f32, edge.1 as f32, edge.2 as f32, a_e);
            }
        }
    }
}

/// 段内内容：**沿弧线排**（图标在左 + 名称在右），每个元素**各自**贴在弧上、
/// 各自按该处的切线旋转 —— 这就是「甲」的「图标在左、名称在右、跟着弧转」。
///
/// ⚠️ 但**不能**把它们摆在一条**共用的切线**上（第一版就是这么写的）：切线只是弧
/// 在一点处的近似，离切点越远偏得越多（偏差 ≈ `d² / 2r`）。30px 的环带根本装不下，
/// 表现就是**图标被甩到段外面、文字只有中间一截露出来**（2026-10-08 真机踩到）。
/// 改成「每个元素用自己的弧长位置 + 自己那处的切线角」之后，偏差只剩
/// 「半个元素尺度 + 弦高」，10px 以内，稳稳待在环带里。
///
/// `fg` = 文字颜色（普通态 `ink` / 改名态白）；`caret` = 画不画编辑光标。
#[allow(clippy::too_many_arguments)]
unsafe fn draw_ring_content(st: &State, buf: &mut [u8], pal: &Pal,
                            seg: (f32, f32, f32), r_in: f32, r_out: f32,
                            icon: Option<&IconTex>, name: &str,
                            fg: (u8, u8, u8), hot: f32, caret: bool, alpha: f32) {
    let (mid_deg, a1, a2) = seg;
    let a8 = (255.0 * alpha).clamp(0.0, 255.0) as u8;
    if a8 == 0 {
        return;
    }
    // hover 抬升必须与 `draw_ring_sector` 用同一个公式，否则内容会和段脱开
    let r_mid = (r_in + r_out) * 0.5;
    let rc = r_mid + RING_LIFT_K * K * hot;
    // 局部坐标（lx = 沿弧的**弧长**、ly = 径向）→ 画布坐标 + 该处的切线角（度）。
    // ⚠️ 不要退回「切线近似」，理由见函数头注释。
    let place = |lx: f32, ly: f32| -> (f32, f32, f32) {
        let ang = mid_deg + (lx / rc).to_degrees();
        let (x, y) = polar(rc + ly, ang);
        (x, y, norm_rot(ang))
    };
    let half = ((a2 - a1).to_radians() * 0.5 * r_mid).max(6.0);
    let (cjk, lat) = (13.0 * K * KC, 7.0 * K * KC);
    // `+1.0` 是余量：段宽本来就是按同一个 `est_text_w` 算出来的，不给余量会出现
    // 「刚好相等 → 判成放不下 → 截成空串」的浮点毛刺（名称「66」就这样消失过）。
    let avail = (2.0 * half - CHIP_PAD_L - CHIP_ICON - CHIP_GAP - CHIP_PAD_R + 1.0).max(10.0);
    let mut label = name.to_string();
    if est_text_w(&label, cjk, lat) > avail {
        let mut acc = String::new();
        for ch in name.chars() {
            let mut t = acc.clone();
            t.push(ch);
            if est_text_w(&format!("{}…", t), cjk, lat) > avail {
                break;
            }
            acc.push(ch);
        }
        if acc.is_empty() {
            // 连「首字 + …」都放不下 → 至少画出首字，**绝不让整条名字凭空消失**
            acc = name.chars().take(1).collect();
        } else {
            acc.push('…');
        }
        label = acc;
    }

    // 左端图标（各自贴弧、按该处切线转；与内环同一套纹理管线）
    let (ix, iy, iang) = place(-half + CHIP_PAD_L + CHIP_ICON * 0.5, 0.0);
    if let Some(tex) = icon {
        draw_bgra_icon_rot(buf, ix, iy, CHIP_ICON * (1.0 + 0.06 * hot),
                           tex, iang.to_radians(), alpha);
    } else if let Some(ch) = name.chars().next() {
        // 拿不到图标（少数 shell: 命名空间）→ **中性**小底板 + 首字母。
        // 全界面唯一允许有颜色的东西是真实图标，所以这里不放彩色。
        let h2 = CHIP_ICON * 0.5;
        rot_rrect(buf, ix, iy, h2, h2, 4.0 * K, iang.to_radians(),
                  mixc(pal.paper, pal.ink, 0.14), alpha);
        let cht = ch.to_uppercase().to_string();
        draw_text_rot(st, buf, ix, iy, &cht, 13, iang.to_radians(),
                      pal.ink.0, pal.ink.1, pal.ink.2, a8);
    }

    if !label.is_empty() {
        let textw = est_text_w(&label, cjk, lat);
        let (tx, ty, tang) = place(-half + CHIP_PAD_L + CHIP_ICON + CHIP_GAP + textw * 0.5, 0.0);
        // ⚠️⚠️ `draw_text_rot` 收的是**弧度**（内部直接 `sin_cos()`）。传「度」=
        // 旋转矩阵整个是错的 → 文字被采样成一团乱码（2026-10-08 踩到，
        // 现场看起来就是「文字坏了」；遮罩本身是好的，落盘验证过）。
        draw_text_rot(st, buf, tx, ty, &label, RING_NAME_PX, tang.to_radians(),
                      fg.0, fg.1, fg.2, a8);
    }

    // 编辑光标：0.5s 周期方波闪烁，画在文本末尾。
    // ⚠️⚠️ 相位必须按**当前时间**算。原来用的是 `st.edit_caret` —— 那是「进入编辑的时刻」，
    // 是个常量，`(常量 * 2).fract()` 只会得到一个固定值 → 光标要么常亮要么常灭，
    // **永远不会变**。所以即使帧一直在重绘，看起来也完全不闪
    // （用户 2026-10-09 报「不知道这是编辑模式了」）。
    if caret {
        let blink = (now_s() * 2.0).fract() < 0.5;
        if blink {
            let textw = est_text_w(&label, cjk, lat);
            let cw = est_text_w("0", cjk, lat).max(4.0);
            let (bx, by, tang) = place(
                -half + CHIP_PAD_L + CHIP_ICON + CHIP_GAP + textw + 1.5 + cw * 0.5,
                0.0,
            );
            // 高度按**文字实际高度**取（原来是 7K ≈ 9.2px，比文字 11.5px 矮一截，
            // 看着像个小点）
            let chh = (RING_NAME_PX as f32 * K * KC) * 0.5;
            rot_rrect(buf, bx, by, cw * 0.5, chh, 1.0, tang.to_radians(), fg, alpha);
        }
    }
}



// ───────────────────── 拖出移除 ─────────────────────

/// 取消按钮：出现时**头像缩 12% 让位**（`cancel_a` 驱动，与按钮同步淡入），
/// 按钮贴着缩小后的球底悬挂 —— 球底 38.3 → 按钮占 41.3..59.3 → 图标砖内沿
/// ≥66.9，下方净空 ~7.6px（连图标投影都够不到），任何 n 都不遮挡。
/// 旧版固定位置两度压到图标（`HUB_R+24` 高30 → `HUB_R+11` 高18 都嫌挤），
/// 用户拍板：与其硬挤，不如让头像缩一下。
const RM_BTN_W: f32 = 62.0;
const RM_BTN_H: f32 = 18.0;
/// 按钮中心相对盘心的偏移 = 缩小的球底(HUB_R*0.88) + 12（cancel_a=1 时）
const RM_BTN_TUCK: f32 = 12.0;
/// 头像让位缩放幅度
const RM_BALL_SHRINK: f32 = 0.12;
const RM_BTN_R: f32 = RM_BTN_H * 0.5;

/// 取消按钮中心（画布坐标）。btn_a = cancel_a（0→1 淡入进度），
/// 按钮边淡入边跟着缩小的球上移
fn rm_btn_center(btn_a: f32) -> (f32, f32) {
    let a = btn_a.clamp(0.0, 1.0);
    (CC, CC + HUB_R * (1.0 - RM_BALL_SHRINK * a) + RM_BTN_TUCK * a)
}

/// 点是否落在取消按钮热区（比视觉尺寸大一圈，指针好点中）
fn hit_rm_btn(st: &State, x: f32, y: f32) -> bool {
    if st.cancel_a < 0.5 {
        return false;
    }
    let (cx, cy) = rm_btn_center(st.cancel_a);
    let hw = RM_BTN_W * 0.5 + 4.0;
    let hh = RM_BTN_H * 0.5 + 4.0;
    (x - cx).abs() <= hw && (y - cy).abs() <= hh
}

/// 屏幕坐标是否还在盘面内
fn in_disc(st: &State, sx: i32, sy: i32) -> bool {
    let px = (sx - st.win_x) as f32;
    let py = (sy - st.win_y) as f32;
    let dx = px - CC;
    let dy = py - CC;
    dx * dx + dy * dy <= disc_r(st.apps.len()) * disc_r(st.apps.len())
}

/// 屏幕坐标是否还在菜单窗口内（外环胶囊本来就悬在盘沿之外，
/// 它的「拖出」判定用窗口边界：拖出窗口才算动真格）
fn in_window(st: &State, sx: i32, sy: i32) -> bool {
    sx >= st.win_x && sx < st.win_x + CANVAS && sy >= st.win_y && sy < st.win_y + CANVAS
}

/// 输入心跳：光标动了/点了就刷新 last_input，并唤醒发呆中的头像
fn note_input(st: &mut State, x: i32, y: i32) {
    if x != st.cur_x || y != st.cur_y {
        st.cur_x = x;
        st.cur_y = y;
        st.last_input = now_s();
        if st.mood == 3 {
            set_mood(st, 0);
        }
    }
}

/// 切换头像情绪（带过渡）
fn set_mood(st: &mut State, m: u8) {
    if st.mood == m {
        return;
    }
    st.mood_prev = st.mood;
    st.mood = m;
    st.mood_k = 0.0;
    st.mood_t = now_s();
}

/// 倒计时到点：待移除的内容**早已从盘面摘下**，这里只负责落盘。
/// 与设置页的删除同一套持久化：应用删除备份→写→重载图标；胶囊删除只写。
unsafe fn commit_expired(st: &mut State, now: f64) {
    if st.pending_rms.is_empty() {
        return;
    }
    let mut removed_any = false;
    let mut removed_app = false;
    let mut i = 0;
    while i < st.pending_rms.len() {
        if st.pending_rms[i].deadline() > now {
            i += 1;
            continue;
        }
        let p = st.pending_rms.remove(i);
        removed_any = true;
        match p {
            PendingRm::App { entry, .. } => {
                println!("[remove] 已移除应用：{}", entry.name);
                removed_app = true;
            }
            PendingRm::Chip { app, entry, .. } => {
                let owner = st.apps.get(app).map(|a| a.name.as_str()).unwrap_or("?");
                println!("[remove] 已移除胶囊：{}（{}）", entry.name, owner);
            }
        }
    }
    // 落盘只做一次（`load_icons` 是重活，删多个应用时别重载多次）
    if removed_any {
        save_settings(st);
        if removed_app {
            load_icons(st);
        }
    }
}

/// 取消一次待移除：把摘下的内容原位放回（倒计时到点走 commit_remove，不走这里）
/// 取消一次待移除：只作用于**最后加入**的那个（用户 2026-10-09 明确要求），
/// 把它原位放回。其余待删的继续按各自的倒计时走。
fn cancel_remove(st: &mut State) {
    if let Some(p) = st.pending_rms.pop() {
        match p {
            PendingRm::App { pos, entry, .. } => {
                let pos = pos.min(st.apps.len());
                st.apps.insert(pos, entry);
                println!("[remove] 已取消，放回：{}", st.apps[pos].name);
            }
            PendingRm::Chip { app, pos, entry, .. } => {
                if let Some(a) = st.apps.get_mut(app) {
                    let pos = pos.min(a.items.len());
                    a.items.insert(pos, entry);
                    println!("[remove] 已取消，放回胶囊：{}", a.items[pos].name);
                }
            }
        }
    }
    // 还有别的待删 → 保持「生气」（等着继续确认）
    set_mood(st, if st.pending_rms.is_empty() { 0 } else { 2 });
}

/// 拖动中：抓的是「按下那一刻」命中的目标（应用图标优先，其次外环胶囊），
/// 不是当前光标下的。盘外 → 紧张；抓着东西在盘内晃 → 疑惑。
fn drag_step(st: &mut State, x: i32, y: i32) {
    note_input(st, x, y);
    if st.drag_app.is_none() && st.drag_chip.is_none() {
        let px = (LB_DOWN_X.load(Ordering::Relaxed) as i32 - st.win_x) as f32;
        let py = (LB_DOWN_Y.load(Ordering::Relaxed) as i32 - st.win_y) as f32;
        st.drag_app = nearest_app(st, px, py, 0.0);
        if st.drag_app.is_none() {
            // 胶囊只可能属于当前亮着的应用（别的应用的 chip 根本没画出来）
            if let Some(ai) = st.active_app {
                if ai < st.apps.len() {
                    st.drag_chip = hit_chip_at(st, ai, px, py).map(|j| (ai, j));
                }
            }
        }
    }
    st.drag_x = (x - st.win_x) as f32;
    st.drag_y = (y - st.win_y) as f32;
    // ── 换位目标：光标落在**另一个**图标/胶囊上就记下来（松手时交换）──
    // 每帧重算，所以离开就自动清掉。
    st.drag_over_app = None;
    st.drag_over_chip = None;
    if let Some(ai) = st.drag_app {
        if let Some(t) = nearest_app(st, st.drag_x, st.drag_y, 0.0) {
            if t != ai {
                st.drag_over_app = Some(t);
            }
        }
    } else if let Some((ai, j)) = st.drag_chip {
        if let Some(t) = hit_chip_at(st, ai, st.drag_x, st.drag_y) {
            if t != j {
                st.drag_over_chip = Some(t);
            }
        }
    }
    let grabbed = st.drag_app.is_some() || st.drag_chip.is_some();
    set_mood(st, if in_disc(st, x, y) {
        if grabbed { 4 } else { 0 }
    } else {
        1
    });
}

/// 拖动松手：
///   应用图标 —— 盘外 = 请求移除：**立即摘下**（盘面上马上消失）+ 生气 +
///              取消按钮 + 5 秒倒计时；到点落盘，取消/收起菜单都放回。
///   外环胶囊 —— 拖出**窗口** = 同上（胶囊本来就悬在盘沿外，所以边界取窗口）。
///   盘内/窗内松手 = 放弃，图标回槽位。
fn drag_end(st: &mut State, x: i32, y: i32) {
    note_input(st, x, y);
    if let Some(ai) = st.drag_app {
        // ⚠️ 这里**不再检查**有没有待确认的移除 —— 待删是可以排队的，
        // 拖走一个之后要能立刻拖下一个（用户 2026-10-09 要求）。
        if ai < st.apps.len() && !in_disc(st, x, y) {
            let entry = st.apps.remove(ai);
            // 槽位收缩：被摘槽位之后的索引整体左移，别让选中态指错应用
            match st.active_app {
                Some(a) if a > ai => st.active_app = Some(a - 1),
                Some(a) if a == ai => st.active_app = None,
                _ => {}
            }
            let name = entry.name.clone();
            st.pending_rms.push(PendingRm::App {
                pos: ai,
                entry,
                deadline: now_s() + REMOVE_GRACE,
            });
            set_mood(st, 2);
            println!("[remove] {} 秒后移除：{}（点「取消」放回）", REMOVE_GRACE as u32, name);
        } else if let Some(t) = st.drag_over_app.filter(|&t| t != ai && t < st.apps.len()) {
            // ── 盘内落在**另一个应用图标**上 = 换位 ──
            // 内环只和内环换；跨层（应用 ↔ 外环条目）没有意义。
            st.apps.swap(ai, t);
            // 选中态跟着被换的那个走，否则会指到错的 app
            if st.active_app == Some(ai) {
                st.active_app = Some(t);
            } else if st.active_app == Some(t) {
                st.active_app = Some(ai);
            }
            let (na, nb) = (st.apps[ai].name.clone(), st.apps[t].name.clone());
            unsafe { save_settings(st) };
            println!("[reorder] 应用换位：{} ↔ {}", na, nb);
            set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
        } else {
            // 盘内松手 = 放弃；但若已有待确认移除，表情得回到「生气」而不是常态
            set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
        }
    } else if let Some((ai, j)) = st.drag_chip {
        if ai < st.apps.len() && !in_window(st, x, y) {
            let entry = st.apps[ai].items.remove(j);
            match st.hover_chip {
                Some(h) if h > j => st.hover_chip = Some(h - 1),
                Some(h) if h == j => st.hover_chip = None,
                _ => {}
            }
            let name = entry.name.clone();
            st.pending_rms.push(PendingRm::Chip {
                app: ai,
                pos: j,
                entry,
                deadline: now_s() + REMOVE_GRACE,
            });
            set_mood(st, 2);
            println!("[remove] {} 秒后移除胶囊：{}（点「取消」放回）", REMOVE_GRACE as u32, name);
        } else if let Some(t) = st.drag_over_chip
            .filter(|&t| t != j && t < st.apps.get(ai).map(|a| a.items.len()).unwrap_or(0))
        {
            // ── 外环内换位（只换同一个应用的条目顺序）──
            let (na, nb) = {
                let items = &st.apps[ai].items;
                (items[j].name.clone(), items[t].name.clone())
            };
            st.apps[ai].items.swap(j, t);
            match st.hover_chip {
                Some(h) if h == j => st.hover_chip = Some(t),
                Some(h) if h == t => st.hover_chip = Some(j),
                _ => {}
            }
            unsafe { save_settings(st) };
            println!("[reorder] 条目换位：{} ↔ {}", na, nb);
            set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
        } else {
            set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
        }
    }
    // 空手拖动松手（按在空白处划了一圈）：回常态；有待确认移除则回「生气」
    if st.drag_app.is_none() && st.drag_chip.is_none() {
        set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
    }
    st.drag_app = None;
    st.drag_chip = None;
    st.drag_over_app = None;
    st.drag_over_chip = None;
}

/// 三个不同频率正弦叠加成一个伪随机抖动。amp 的单位是**球半径的倍数**。
fn jitter(t_ms: f64, speed: f64, amp: f32) -> f32 {
    let s = (t_ms / 1000.0 * speed) as f32;
    ((s * 3.1).sin() + (s * 5.7 + 0.85).sin() + (s * 9.3 + 1.52).sin()) / 3.0 * amp
}

/// 段半宽（命中判定与绘制共用同一公式）。
/// ⚠️ 名称宽度封顶 `RING_NAME_MAX_W`；`draw_ring_content` 里截断用的也是它。
fn chip_half_w(name: &str) -> f32 {
    (CHIP_PAD_L + CHIP_ICON + CHIP_GAP
        + est_text_w(name, 13.0 * K * KC, 7.0 * K * KC).min(RING_NAME_MAX_W)
        + CHIP_PAD_R)
        / 2.0
}

/// 把角度归一到 (-90°, 90°] —— 只为「文字永远从左向右读」。
/// 下半圈翻转 180°，否则段里的字会是倒的。
fn norm_rot(a: f32) -> f32 {
    let aa = ((a % 360.0) + 360.0) % 360.0;
    if aa <= 90.0 || aa >= 270.0 {
        if aa > 180.0 {
            aa - 360.0
        } else {
            aa
        }
    } else {
        aa - 180.0
    }
}

/// 外环段布局：返回 (**每段的角度范围 (a1, a2)**, 每段的旋转角(度), 环半径)。
///
/// **甲方案 = 贴身扇面**：以当前应用的方位角 `a0` 为中心，各段紧贴着对应的内环
/// 图标向两侧铺开（用户明确要求过「优先在内环对应图标的位置开始排列」「离图标太远
/// 不好点击」）。每段的角跨 ∝ 该条目的名字宽度，段间留 `CHIP_MIN_GAP` 的**弧长**缝。
/// 只有当整体跨度超过 `CHIP_FAN_MAX` 时才退回「整圈均布 + 首段锚定 a0」。
///
/// ⚠️ 半径**只能**用环带的中位半径 —— 段就画在那条带上；用别的半径算角宽会让段
/// 与内容对不上（宽了相邻重叠、窄了留大缝）。
/// ⚠️ 绘制与命中判定**只能**调这一个函数。
fn chip_layout(st: &State, ai: usize) -> (Vec<(f32, f32)>, Vec<f32>, Vec<f32>) {
    let n = st.apps.len();
    if n == 0 || ai >= n {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let items = &st.apps[ai].items;
    let m = items.len();
    if m == 0 {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    // 正在改名的那个条目用**输入缓冲**算宽度 —— 否则名字一变段宽跟着变，
    // 而命中框还是旧宽度（「画在这、点到那」）。
    let name_of = |j: usize| -> String {
        match st.editing {
            Some((eai, ej)) if eai == ai && ej == j => st.edit_buf.clone(),
            _ => items[j].name.clone(),
        }
    };
    let halfs: Vec<f32> = (0..m).map(|j| chip_half_w(&name_of(j))).collect();
    let sum: f32 = halfs.iter().map(|h| h * 2.0).sum();

    let (r_in, r_out) = ring_radii(n);
    let r = (r_in + r_out) * 0.5;
    let a0 = app_angle(ai, n);
    let deg_per_px = 180.0 / (std::f32::consts::PI * r);
    let gap_deg = CHIP_MIN_GAP * deg_per_px;
    let wdeg: Vec<f32> = halfs.iter().map(|h| 2.0 * h * deg_per_px).collect();
    let arc = sum + CHIP_MIN_GAP * (m.saturating_sub(1)) as f32;

    let mut segs: Vec<(f32, f32)> = Vec::with_capacity(m);
    if arc * deg_per_px <= CHIP_FAN_MAX {
        // 贴身扇面：整体以 a0 为中心左右对称
        let mut acc = -arc * deg_per_px * 0.5;
        for j in 0..m {
            segs.push((a0 + acc, a0 + acc + wdeg[j]));
            acc += wdeg[j] + gap_deg;
        }
    } else {
        // 整圈均布：间隙数 = m（闭合环首尾之间也要算一个），第 1 段锚定在 a0
        let gap = ((std::f32::consts::TAU * r - sum) / m as f32).max(CHIP_MIN_GAP);
        let gd = gap * deg_per_px;
        let mut acc = 0.0f32;
        for j in 0..m {
            segs.push((a0 + acc, a0 + acc + wdeg[j]));
            acc += wdeg[j] + gd;
        }
    }
    let rots: Vec<f32> = segs.iter().map(|(a1, a2)| norm_rot((a1 + a2) * 0.5)).collect();
    (segs, rots, vec![r; m])
}

// ───────────────────── 窗口状态 ─────────────────────

unsafe fn state_of(hwnd: HWND) -> &'static mut State {
    &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State)
}

unsafe fn post(m: u32, w: usize, l: usize) {
    let h = MAIN_HWND.load(Ordering::Relaxed);
    if h != 0 {
        // PostMessage 失败 = 窗口已销毁，或 UI 线程消息队列塞满。
        // **队列塞满正是「UI 线程被卡住」的直接证据** —— UI 线程卡住时
        // 所有 post 过来的消息都不处理，菜单就再也不响应任何操作。
        //
        // 节流：用 exchange 而不是「失败才置位」—— 每次失败都记会瞬间刷爆日志
        // （队列满时每毫秒都在失败）。只在「从成功切到失败」时记第一条。
        if PostMessageW(HWND(h as *mut c_void), m, WPARAM(w), LPARAM(l as isize)).is_err() {
            if !POST_DROP_LAST.swap(true, Ordering::Relaxed) {
                diag_push(DK_POST_DROP, m);
            }
        } else {
            POST_DROP_LAST.store(false, Ordering::Relaxed);
        }
    }
}

// ───────────────────── OLE 放置目标 ─────────────────────

#[windows_core::implement(IDropTarget)]
struct RingDropTarget;

impl IDropTarget_Impl for RingDropTarget_Impl {
    fn DragEnter(
        &self,
        _pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        DRAG_ACTIVE.store(true, Ordering::SeqCst);
        DRAG_SEEN.store(true, Ordering::SeqCst);
        drag_log("DragEnter");
        DROP_PT.store(pack_pt(pt.x, pt.y), Ordering::SeqCst);
        unsafe {
            *pdweffect = DROPEFFECT(1 | 4); // COPY | LINK
        }
        unsafe { post(WM_APP_DRAGEVENT, 0, 0); }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        let packed = pack_pt(pt.x, pt.y);
        let prev = DROP_PT.load(Ordering::SeqCst);
        DROP_PT.store(packed, Ordering::SeqCst);
        unsafe {
            *pdweffect = DROPEFFECT(1 | 4);
        }
        // 节流：位移超过 3px 才通知（否则拖动期间消息风暴）
        let (px, py) = unpack_pt(packed);
        let (qx, qy) = unpack_pt(prev);
        if (px - qx).abs() > 3 || (py - qy).abs() > 3 {
            unsafe { post(WM_APP_DRAGEVENT, 1, 0); }
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows_core::Result<()> {
        DRAG_ACTIVE.store(false, Ordering::SeqCst);
        drag_log("DragLeave");
        unsafe { post(WM_APP_DRAGEVENT, 2, 0); }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Option<&IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows_core::Result<()> {
        DROP_PT.store(pack_pt(pt.x, pt.y), Ordering::SeqCst);
        drag_log("Drop");
        unsafe {
            *pdweffect = DROPEFFECT(1);
        }
        if let Some(obj) = pdataobj {
            let items = unsafe { extract_drop_items(obj) };
            if let Ok(mut q) = DROP_PAYLOAD.lock() {
                *q = items;
            }
        }
        unsafe { post(WM_APP_DRAGEVENT, 3, 0); }
        Ok(())
    }
}

/// 从 IDataObject 提取可入环的条目：文件/文件夹/lnk（CF_HDROP）优先，
/// 其次 URL（UniformResourceLocatorW），最后纯文本（像 URL 才收）。
unsafe fn extract_drop_items(obj: &IDataObject) -> Vec<DropItem> {
    let mut out: Vec<DropItem> = Vec::new();
    let fmt_hdrop = FORMATETC {
        cfFormat: 15, // CF_HDROP
        dwAspect: DVASPECT_CONTENT.0 as u32,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ptd: ptr::null_mut(),
    };
    let cf_url = RegisterClipboardFormatW(PCWSTR(wide("UniformResourceLocatorW").as_ptr()));
    let fmt_url = FORMATETC {
        cfFormat: cf_url as u16,
        dwAspect: DVASPECT_CONTENT.0 as u32,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ptd: ptr::null_mut(),
    };
    let fmt_text = FORMATETC {
        cfFormat: 13, // CF_UNICODETEXT
        dwAspect: DVASPECT_CONTENT.0 as u32,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ptd: ptr::null_mut(),
    };

    if obj.GetData(&fmt_hdrop).is_ok() {
        let mut stg = obj.GetData(&fmt_hdrop).unwrap();
        let hg = stg.u.hGlobal;
        if !hg.0.is_null() {
            let n = DragQueryFileW(HDROP(hg.0), u32::MAX, None);
            for i in 0..n {
                let mut buf = [0u16; 520];
                let len = DragQueryFileW(HDROP(hg.0), i, Some(&mut buf)) as usize;
                let path = String::from_utf16_lossy(&buf[..len]);
                if path.is_empty() {
                    continue;
                }
                let is_dir = std::path::Path::new(&path).is_dir();
                let stem = std::path::Path::new(&path)
                    .file_stem()
                    .or_else(|| std::path::Path::new(&path).file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                out.push(DropItem {
                    name: clamp_name(&stem),
                    target: path,
                    is_url: false,
                    is_dir,
                });
            }
        }
        ReleaseStgMedium(&mut stg);
        if !out.is_empty() {
            return out;
        }
    }

    if obj.GetData(&fmt_url).is_ok() {
        let mut stg = obj.GetData(&fmt_url).unwrap();
        let hg = stg.u.hGlobal;
        if !hg.0.is_null() {
            let p = GlobalLock(HGLOBAL(hg.0));
            if !p.is_null() {
                let mut len = 0usize;
                let ws = p as *const u16;
                while *ws.add(len) != 0 && len < 2048 {
                    len += 1;
                }
                let url = String::from_utf16_lossy(std::slice::from_raw_parts(ws, len));
                let _ = GlobalUnlock(HGLOBAL(hg.0));
                let url = url.trim().to_string();
                if !url.is_empty() {
                    let name = url_host_name(&url);
                    out.push(DropItem { name, target: url, is_url: true, is_dir: false });
                }
            }
        }
        ReleaseStgMedium(&mut stg);
        if !out.is_empty() {
            return out;
        }
    }

    if obj.GetData(&fmt_text).is_ok() {
        let mut stg = obj.GetData(&fmt_text).unwrap();
        let hg = stg.u.hGlobal;
        if !hg.0.is_null() {
            let p = GlobalLock(HGLOBAL(hg.0));
            if !p.is_null() {
                let mut len = 0usize;
                let ws = p as *const u16;
                while *ws.add(len) != 0 && len < 2048 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ws, len));
                let _ = GlobalUnlock(HGLOBAL(hg.0));
                let text = text.trim().to_string();
                // 只有看起来像链接的纯文本才收（拖动网页文字不该误入环）
                if text.len() < 2048
                    && (text.starts_with("http://")
                        || text.starts_with("https://")
                        || text.starts_with("www."))
                {
                    let name = url_host_name(&text);
                    out.push(DropItem {
                        name,
                        target: if text.starts_with("www.") {
                            format!("https://{}", text)
                        } else {
                            text
                        },
                        is_url: true,
                        is_dir: false,
                    });
                }
            }
        }
        ReleaseStgMedium(&mut stg);
    }
    out
}

fn clamp_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '=' && *c != '\n' && *c != '\r' && *c != '[' && *c != ']')
        .collect();
    cleaned.trim().chars().take(20).collect()
}

fn url_host_name(url: &str) -> String {
    let rest = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url);
    let host = rest.trim_start_matches("www.");
    let name = if host.is_empty() { url } else { host };
    clamp_name(name)
}

// ───────────────────── 钩子 ─────────────────────

/// 低级鼠标钩子的事件结构（MSLLHOOKSTRUCT）
#[repr(C)]
struct MsLlHook {
    pt: POINT,
    mouse_data: u32,
    flags: u32,
    time: u32,
    dw_extra_info: usize,
}

unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    // 计时包装：本回调变慢 = 用户鼠标全系统卡顿的直接证据。
    // 只多两次 GetTickCount（纳秒级），事件本身照常处理。
    //
    // ⚠️ 静默阈值从 30s 降到 **5s**：用户报「鼠标失灵」时要能定位**精确起止时刻**。
    // 30s 太粗 —— 461 秒的空档只记一条，看不出「什么时候开始不对劲」。
    // 5s 一条，一个小时最多 720 条，看门狗有 20000 行预算，绰绰有余。
    let t0 = GetTickCount();
    let last = LAST_HOOK_TICK.swap(t0, Ordering::Relaxed);
    let gap = t0.wrapping_sub(last);
    if gap > 5_000 {
        diag_push(DK_MOUSE_IDLE, gap.min(0x00ff_ffff));
    }
    if gap > 30_000 {
        diag_push(DK_HOOK_GAP, gap.min(0x00ff_ffff));
    }
    let r = mouse_hook_impl(code, w, l);
    let dt = GetTickCount().wrapping_sub(t0);
    if dt > 30 {
        diag_push(DK_HOOK_SLOW, dt.min(0x00ff_ffff));
    }
    r
}

/// 右键长按（呼出键之一）状态机。
///
/// 返回 `Some(r)` = 这次事件由我们消费，调用方直接 `return r`；
/// 返回 `None`   = 不归我们管，调用方继续往下（等价于 `CallNextHookEx`）。
///
/// **为什么抽成独立函数**：这段逻辑的 bug 无法用「注入全局鼠标事件」来测 ——
/// 测试进程注入输入 + 我们装着全局钩子，两边会互相卡死（试过，整个测试挂死）。
/// 抽出来之后，自动化可以走 `WM_APP_RHOLDSIM` 直接喂事件码来验，安全且确定。
///
/// **必须守住的不变式**：吞掉的 DOWN 必须由配对的 UP 收尾；
/// **没吞过 DOWN 的 UP 一律放行**。违反它的后果（2026-10-08 用户报的现场）：
/// 应用收到 RIGHTDOWN、永远收不到 RIGHTUP → 它以为右键一直按着
/// → 「关掉菜单后右键点哪儿都不生效」。
unsafe fn rhold_step(m: u32) -> Option<LRESULT> {
    if !OPT_RHOLD.load(Ordering::Relaxed) {
        return None;
    }
    if m == WM_RBUTTONDOWN {
        let menu_open = MENU_OPEN.load(Ordering::Relaxed);
        R_DOWN_AT.store(GetTickCount(), Ordering::Relaxed);
        R_DOWN_INMENU.store(menu_open, Ordering::Relaxed);
        R_DOWN_SEEN.fetch_add(1, Ordering::Relaxed);
        if menu_open {
            // 菜单开着时右键 = 收起。这一对 DOWN/UP 都由我们吞掉（**成对**，不留孤儿）。
            if !DRAG_ACTIVE.load(Ordering::Relaxed) {
                post(WM_APP_CANCEL, 0, 0);
            }
            return Some(LRESULT(1));
        }
        // 菜单没开：**放行**。为什么不像以前那样吞掉——见 UP 分支的长注释。
        return None;
    }
    if m == WM_RBUTTONUP {
        let t0 = R_DOWN_AT.swap(0, Ordering::Relaxed);
        if t0 == 0 {
            // 没有配对的 DOWN → 一律放行（吞掉会让应用永久以为右键按着）
            return None;
        }
        let was_in_menu = R_DOWN_INMENU.load(Ordering::Relaxed);
        let held = GetTickCount().wrapping_sub(t0) >= RHOLD_MS;
        if held && !MENU_OPEN.load(Ordering::Relaxed) {
            // 长按 = 呼出菜单。
            // 先记下当前前台窗口：这次放行的 UP 会让它弹出自己的右键菜单，
            // 我们要在 UI 线程里补一次 Esc 把它关掉（见 WM_APP_DISMISS_FG）。
            let fg = GetForegroundWindow();
            FG_HWND.store(fg.0 as isize, Ordering::Relaxed);
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            CURSOR_X.store(p.x, Ordering::Relaxed);
            CURSOR_Y.store(p.y, Ordering::Relaxed);
            post(WM_APP_SHOW, 0, 0);
            post(WM_APP_DISMISS_FG, 0, 0);
        }
        // ── 为什么长按呼出**不再吞 DOWN / 也不再补发** ──
        //
        // 老设计：吞掉 DOWN，短按时在回调里 `mouse_event` 补发一次。
        // 实测三连炸（2026-10-08，全部可复现）：
        //   ① 在**钩子回调里**注入 → 钩子线程自锁，回调耗时 1000ms，鼠标卡 5~6 秒
        //   ② 改到 **UI 线程**注入 → UI 线程卡死在 `mouse_event` 里永不返回
        //      （证据：补发请求已发出但 inject_done 一直 0，之后所有消息超时 4 秒）
        //   ③ 再改到 **独立线程**注入 → 注入线程卡在第二个 mouse_event，
        //      等于给应用留了个「有 DOWN 没 UP」的孤儿事件
        // 结论：**本进程装着全局鼠标钩子时，任何鼠标事件注入都不可靠。**
        // 所以改成：右键**全程放行**（DOWN、UP 都不吞），短按就是一次完全正常的
        // 右键；长按除了呼出我们的菜单，还会让应用弹一下它自己的菜单 —— 我们在
        // UI 线程补一个 Esc 把它关掉。**没有注入 = 不可能再卡死系统。**
        // 代价只是长按那一瞬应用菜单可能闪一下（失败也是「闪一下」，不会挂）。
        return if was_in_menu { Some(LRESULT(1)) } else { None };
    }
    None
}

unsafe extern "system" fn mouse_hook_impl(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    // 只放行「我们自己补发的」事件（靠 dwExtraInfo 打标）。
    // ⚠️ 不能一刀切拦掉所有 injected 事件 —— 触摸板、远程桌面、自动化测试
    // 发的也是 injected，一刀切会让它们全部失效。
    if (*(l.0 as *const MsLlHook)).dw_extra_info == RL_MAGIC {
        // 诊断：数一下「我们自己注入的事件」到底有没有带着标记回来。
        // 如果这个数一直是 0，说明 dwExtraInfo 标记没生效 —— 那时我们注入的
        // 右键会被自己当成真实右键吞掉 → 补发 → 再注入 → **无限注入风暴**，
        // 表现为输入队列被灌满、UI 线程卡死（2026-10-08 排查用）。
        MAGIC_SEEN.fetch_add(1, Ordering::Relaxed);
        return CallNextHookEx(None, code, w, l);
    }

    let m = w.0 as u32;

    // ── 右键长按（呼出键之一）：**必须在状态分派之前处理** ──
    //
    // 它是**跨状态**的：按下时菜单可能没开、松开时菜单可能已经被这次按下关掉了。
    // 之前把 down 记在 `!MENU_OPEN` 分支、up 也在同一分支读，于是出现致命组合：
    //   ① 菜单开着时按右键 → MENU_OPEN 分支只清 R_DOWN_AT，**DOWN 透传给应用**
    //   ② 菜单立刻关掉（MENU_OPEN=false）
    //   ③ 松开 → 走 !MENU_OPEN 分支 → t0==0 → 什么都不做，但 `return LRESULT(1)` 吞掉
    //   ⇒ 应用收到 DOWN 收不到 UP → 以为右键一直按着 → 「右键点哪儿都不生效」
    // 现在整段逻辑收在 `rhold_step()` 里，状态无关、且有明确不变式。
    if let Some(r) = rhold_step(m) {
        return r;
    }

    if !MENU_OPEN.load(Ordering::Relaxed) {
        // 呼出键：中键 / 侧键，两个开关各自独立（可都开）
        let fire = match m {
            WM_MBUTTONDOWN => OPT_MIDDLE.load(Ordering::Relaxed),
            WM_XBUTTONDOWN => OPT_SIDE.load(Ordering::Relaxed),
            _ => false,
        };
        if fire {
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            CURSOR_X.store(p.x, Ordering::Relaxed);
            CURSOR_Y.store(p.y, Ordering::Relaxed);
            post(WM_APP_SHOW, 0, 0);
        }
        return CallNextHookEx(None, code, w, l);
    }
    match m {
    WM_MOUSEMOVE => {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        CURSOR_X.store(p.x, Ordering::Relaxed);
        CURSOR_Y.store(p.y, Ordering::Relaxed);
        // 按住左键在动：越过阈值算「拖」；**之后每次移动都通知** ——
        // 被拖图标跟手、紧张/疑惑表情的实时切换都靠连续的 drag_step。
        // 消息本身去重（队列里只留一条），坐标以原子量为准。
        if LB_DOWN.load(Ordering::Relaxed) {
            if !LB_MOVED.load(Ordering::Relaxed) {
                let dx = (p.x - LB_DOWN_X.load(Ordering::Relaxed) as i32) as f32;
                let dy = (p.y - LB_DOWN_Y.load(Ordering::Relaxed) as i32) as f32;
                if (dx * dx + dy * dy).sqrt() > DRAG_THRESH {
                    LB_MOVED.store(true, Ordering::Relaxed);
                }
            }
            if LB_MOVED.load(Ordering::Relaxed) && !DRAG_POSTED.swap(true, Ordering::Relaxed) {
                post(WM_APP_DRAGSTEP, 0, 0);
            }
        }
        // 去重：队列里只保留一条 hover 通知，坐标以原子量为准
        if !HOVER_POSTED.swap(true, Ordering::Relaxed) {
            post(WM_APP_HOVER, 0, 0);
        }
    }
        WM_MBUTTONUP | WM_XBUTTONUP => {
            // 松开触发键时左键仍按着 = 用户正拖着东西 → 进入拖放等待态，
            // 菜单保持显示等拖放落环，绝不在这时执行/取消
            let left_held =
                windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x01) as u16 & 0x8000 != 0;
            if left_held || DRAG_ACTIVE.load(Ordering::Relaxed) {
                post(WM_APP_DRAGEVENT, 0, 0);
            } else {
                post(WM_APP_COMMIT, 0, 0);
            }
        }
        WM_LBUTTONDOWN => {
            if DRAG_ACTIVE.load(Ordering::Relaxed) {
                // OLE 拖放中的左键是拖放内部事件，忽略
            } else {
                // 只记录按下，**不立刻执行** —— 否则「按住拖出去」会先把应用启动了。
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                LB_DOWN_X.store(p.x as isize, Ordering::Relaxed);
                LB_DOWN_Y.store(p.y as isize, Ordering::Relaxed);
                LB_MOVED.store(false, Ordering::Relaxed);
                LB_DOWN.store(true, Ordering::Relaxed);
            }
        }
        WM_LBUTTONUP => {
            let moved = LB_MOVED.swap(false, Ordering::Relaxed);
            LB_DOWN.store(false, Ordering::Relaxed);
            if moved {
                // 拖动松手：盘内=放弃、盘外=请求移除（由 UI 线程按光标位置判定）
                post(WM_APP_DRAGEND, 0, 0);
            } else if DRAG_ACTIVE.load(Ordering::Relaxed) {
                // OLE 拖放内部事件，忽略
            } else if DROP_MODE_ARMED.load(Ordering::Relaxed) {
                // 拖放等待态下在外面松手（没落环）= 收起菜单
                post(WM_APP_CANCEL, 0, 0);
            } else if LOCKED.load(Ordering::Relaxed) {
                post(WM_APP_CLICK, 0, 0);
            } else {
                post(WM_APP_CANCEL, 0, 0);
            }
        }
        WM_RBUTTONDOWN | WM_RBUTTONUP => {
            // 走到这里 = `OPT_RHOLD` 没开（开着的话上面已经 return 了）。
            // 此时右键长按呼出是关闭的，右键应当**完全透传**给应用 ——
            // 不能吞、也不能改 R_DOWN_AT（那个量只属于长按逻辑）。
            // 菜单开着时右键仍要能收起菜单。
            if m == WM_RBUTTONDOWN && !DRAG_ACTIVE.load(Ordering::Relaxed) {
                post(WM_APP_CANCEL, 0, 0);
            }
        }
        _ => {}
    }
    CallNextHookEx(None, code, w, l)
}

/// 低级键盘钩子的事件结构
#[repr(C)]
struct KbdLlHook {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    dw_extra_info: usize,
}

/// 低级键盘钩子：菜单打开时截获 Esc / 回车 / 方向键 / Tab
unsafe extern "system" fn key_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    let t0 = GetTickCount();
    let r = key_hook_impl(code, w, l);
    let dt = GetTickCount().wrapping_sub(t0);
    if dt > 30 {
        diag_push(DK_KEY_SLOW, dt.min(0x00ff_ffff));
    }
    r
}

unsafe extern "system" fn key_hook_impl(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && MENU_OPEN.load(Ordering::Relaxed) {
        let m = w.0 as u32;
        let kb = &*(l.0 as *const KbdLlHook);
        // 改名模式下先拦编辑键 —— Esc/Enter/Backspace 语义全变了
        if m == 0x0100 && EDIT_ARMED.load(Ordering::Relaxed) {
            match kb.vk_code {
                0x1B => {
                    post(WM_APP_KEY, KA_EDIT_CANCEL, 0);
                    return LRESULT(1);
                }
                0x0D => {
                    post(WM_APP_KEY, KA_EDIT_COMMIT, 0);
                    return LRESULT(1);
                }
                0x08 => {
                    post(WM_APP_KEY, KA_EDIT_BACK, 0);
                    return LRESULT(1);
                }
                0x2E => {
                    post(WM_APP_KEY, KA_EDIT_DEL, 0);
                    return LRESULT(1);
                }
                _ => {}
            }
        }
        if m == 0x0100 || m == 0x0104 {
            let vk = kb.vk_code;
            let mut act = 0usize;
            match vk {
                0x1B => act = KA_CLOSE,       // Esc
                0x0D => act = KA_CONFIRM,     // Enter
                0x25 | 0x26 => act = KA_PREV, // ← ↑
                0x27 | 0x28 => act = KA_NEXT, // → ↓
                0x56 => act = KA_PASTE, // V：剪贴板内容入环
                0x09 => {
                    // Tab：Shift 反向
                    let shift = windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x10)
                        as u16
                        & 0x8000
                        != 0;
                    act = if shift { KA_PREV } else { KA_NEXT };
                }
                _ => {}
            }
            if act != 0 {
                post(WM_APP_KEY, act, 0);
                return LRESULT(1); // 吞掉，不透传给前台程序
            }
            // 改名模式：其余可打印字符追加进缓冲
            if m == 0x0100 && EDIT_ARMED.load(Ordering::Relaxed) {
                if let Some(c) = key_to_char(kb.vk_code, kb.scan_code, kb.flags) {
                    if !c.is_control() {
                        post(WM_APP_CHAR, c as usize, 0);
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, w, l)
}

/// 键盘钩子事件 → 一个字符。走 `ToUnicodeEx` 用当前键盘布局转换，
/// 这样非英文布局（德语 ä、西语 ñ）也能正确输入。
/// ⚠️ 必须带 KEYEVENTF_UNICODE 之外的常规路径：ToUnicodeEx 需要 scan code
/// 与键盘状态，而低级别钩子的 flags 里已经带了扩展键位标记，要原样传。
unsafe fn key_to_char(vk: u32, scan: u32, flags: u32) -> Option<char> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, ToUnicodeEx, KEYEVENTF_EXTENDEDKEY,
    };
    let mut ks = [0u8; 256];
    if GetKeyboardState(&mut ks).is_err() {
        return None;
    }
    let mut buf = [0u16; 4];
    // dwhkl 传 HKL(None) = 用当前活动布局。非英文布局（德语 ä / 西语 ñ）靠这条。
    let n = ToUnicodeEx(vk, scan, &ks, &mut buf, flags, None);
    if n > 0 {
        return char::from_u32(buf[0] as u32);
    }
    // 扩展键（小键盘 Enter/方向键等）ToUnicodeEx 会失败
    if flags & (KEYEVENTF_EXTENDEDKEY.0 as u32) != 0 {
        return None;
    }
    if (0x30..=0x5A).contains(&vk) {
        return char::from_u32(vk);
    }
    None
}

/// 把键盘钩子同步到 `KEY_HOOK_WANT`。**只能在钩子线程里调** ——
/// 低级钩子必须由「安装它的那条线程」装卸，而那条线程要有消息循环。
unsafe fn sync_key_hook() {
    let want = KEY_HOOK_WANT.load(Ordering::Relaxed);
    let have = KEY_HOOK_LOCAL.load(Ordering::Relaxed) != 0;
    if want && !have {
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or(HMODULE(ptr::null_mut())).0);
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), hinst, 0) {
            Ok(h) => {
                KEY_HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
                KEY_HOOK_LOCAL.store(h.0 as isize, Ordering::Relaxed);
            }
            Err(_) => eprintln!("[!] 键盘钩子安装失败"),
        }
    } else if !want && have {
        let kh = KEY_HOOK_LOCAL.swap(0, Ordering::Relaxed);
        if kh != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(kh as *mut c_void));
        }
        KEY_HOOK_HANDLE.store(0, Ordering::SeqCst);
    }
}

/// 请求（或撤销）键盘钩子。任意线程可调；真正的装卸发生在钩子线程里。
fn need_key_hook(on: bool) {
    if KEY_HOOK_WANT.swap(on, Ordering::SeqCst) == on {
        return; // 状态没变，别白叫醒一次
    }
    let tid = HOOK_TID.load(Ordering::Relaxed);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_APP_HOOK_SYNC, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn stop_hook() {
    // 顺手把「要键盘钩子」的意愿清零 —— 否则下次 set_running(true) 时
    // 钩子线程会立刻把键盘钩子装回去（包括菜单其实没开的情况）。
    KEY_HOOK_WANT.store(false, Ordering::SeqCst);
    let h = HOOK_HANDLE.swap(0, Ordering::SeqCst);
    if h != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(h as *mut c_void));
    }
    let k = KEY_HOOK_HANDLE.swap(0, Ordering::SeqCst);
    if k != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(k as *mut c_void));
    }
    let tid = HOOK_TID.swap(0, Ordering::SeqCst);
    if tid != 0 {
        let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
    }
}

/// 优雅退出：**先摘钩子、等钩子线程真正退场，然后才结束进程**。
/// 直接 `std::process::exit(0)` 会把钩子线程连人带枪活埋 —— win32k 里的
/// 钩子没来得及卸载，僵尸壳的钩子仍留在系统输入链上：每个鼠标事件都
/// 等一个死线程（全系统鼠标卡顿），活钩子递进死钩子也永不返回（触发键
/// 全灭）。曾把整条输入链毒化到必须重启系统。退出纪律没有商量余地。
unsafe fn quit_app() -> ! {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenThread, THREAD_ALL_ACCESS, WaitForSingleObject};
    let h = HOOK_HANDLE.swap(0, Ordering::SeqCst);
    if h != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(h as *mut c_void));
    }
    let k = KEY_HOOK_HANDLE.swap(0, Ordering::SeqCst);
    if k != 0 {
        let _ = UnhookWindowsHookEx(HHOOK(k as *mut c_void));
    }
    let tid = HOOK_TID.load(Ordering::SeqCst);
    if tid != 0 {
        // WM_QUIT → 钩子线程的 GetMessage 返回 0 → 循环结束 → 线程退场
        let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        if let Ok(th) = OpenThread(THREAD_ALL_ACCESS, false, tid) {
            // 最多等 2 秒；超时也照退 —— 钩子已摘，剩下的壳无害
            let _ = WaitForSingleObject(th, 2000);
            let _ = CloseHandle(th);
        }
    }
    std::process::exit(0);
}

// ───────────────────── 外环胶囊右键菜单（改名 / 换色） ─────────────────────

/// 弹出右键菜单 / 改名对话框期间置 true：TrackPopupMenu 和模态对话框都是
/// 消息循环，队列里堆积的真实输入（点击/右键）会在其中被分发 —— 不挡住
/// 的话，一点杂散点击就会关菜单或误触 commit。
static CTX_OPEN: AtomicBool = AtomicBool::new(false);
/// 改名模式是否激活。键盘钩子在**别的线程**上跑，必须靠这个原子量才知道
/// 该不该把字符吞掉（读 st.editing 会跨线程，是 UB）。
static EDIT_ARMED: AtomicBool = AtomicBool::new(false);
// ── 右键菜单 / 原生输入框：**已按用户要求删除** ──
// 2026-10-07：用户明确「去掉换色去掉菜单，双击直接就在胶囊里改名」。
// 原 chip_ctx_menu()（右键 -> 改名/换色 TrackPopupMenu）+ input_dialog() + dlg_proc()
// 全部移除，改名走 editing（就地编辑胶囊，见 edit_push / edit_finish）。
// 保留 CTX_OPEN 是因为 WM_APP_CANCEL 还要用它判断「菜单开着时别误收起」。

// ───────────────────── 托盘 / 启停 ─────────────────────

unsafe fn add_tray_icon(hwnd: HWND) {
    let mut ni = NOTIFYICONDATAW::default();
    ni.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    ni.hWnd = hwnd;
    ni.uID = 1;
    ni.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    ni.uCallbackMessage = WM_APP_TRAY;
    ni.hIcon = app_icon();
    let tip: Vec<u16> = "径向启动器（Ctrl+Alt+R 开关监听）".encode_utf16().collect();
    for (i, c) in tip.iter().enumerate().take(127) {
        ni.szTip[i] = *c;
    }
    let _ = Shell_NotifyIconW(NIM_ADD, &ni);
}

/// 程序图标 = **exe 里那张资源图标**（中心头像球：黑球 + 两只白色药丸眼）。
///
/// 资源由 `build.rs` 调 Windows SDK 的 `rc.exe` 把 `app.rc`（`1 ICON "app.ico"`）
/// 编成 `.res` 再交给链接器；`.ico` 本身由 `make_icon.py` 按中心头像的**真实几何**生成。
///
/// ⚠️ 以前这里用 `IDI_APPLICATION`（系统默认程序图标），跟本程序毫无关系。
/// ⚠️ 尺寸必须用 `SM_CXSMICON`（高 DPI 下是 20/24px），**不要写死 16** ——
///    写死的话系统要把 16px 再拉大一次，托盘上会明显发虚。
/// ⚠️ 万一资源没编进去（本机没装 Windows SDK），退回系统默认图标而不是给个空 HICON，
///    否则托盘会出现一个**看不见的占位**。
unsafe fn app_icon() -> HICON {
    let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or(HMODULE(ptr::null_mut())).0);
    let s = GetSystemMetrics(SM_CXSMICON).max(16);
    // ID = 1，与 `app.rc` 里的 `1 ICON "app.ico"` 对应（MAKEINTRESOURCE 语义）
    // `LR_SHARED`：系统缓存这一个句柄，我们不必（也不该）去 DestroyIcon。
    LoadImageW(hinst, PCWSTR(1 as *const u16), IMAGE_ICON, s, s,
               LR_DEFAULTCOLOR | LR_SHARED)
        .map(|h| HICON(h.0))
        .unwrap_or_else(|_| {
            LoadIconW(None, IDI_APPLICATION).unwrap_or(HICON(ptr::null_mut()))
        })
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut ni = NOTIFYICONDATAW::default();
    ni.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    ni.hWnd = hwnd;
    ni.uID = 1;
    let _ = Shell_NotifyIconW(NIM_DELETE, &ni);
}

unsafe fn set_running(st: &mut State, on: bool) {
    if st.running == on {
        return;
    }
    if on {
        // 上一轮的钩子如果还挂着（托盘「停止监听」失败、或上次退出没走
        // quit_app 的路径），**必须先摘干净**再装新的 —— 否则系统输入链上
        // 会出现两条钩子，其中一条可能挂在死线程上，永远返回不了。
        if HOOK_HANDLE.load(Ordering::Relaxed) != 0 || KEY_HOOK_HANDLE.load(Ordering::Relaxed) != 0 {
            stop_hook();
            // 等旧线程真的退场（最多 1.5s）。不等的话新旧会短暂并存。
            let old = HOOK_TID.swap(0, Ordering::SeqCst);
            if old != 0 {
                use windows::Win32::Foundation::CloseHandle;
                use windows::Win32::System::Threading::{
                    OpenThread, WaitForSingleObject, THREAD_ALL_ACCESS,
                };
                if let Ok(th) = OpenThread(THREAD_ALL_ACCESS, false, old) {
                    let _ = WaitForSingleObject(th, 1500);
                    let _ = CloseHandle(th);
                }
            }
        }
        // 钩子装在专用线程：回调即时返回，不受 UI 线程渲染繁忙影响
        // （低级钩子回调在安装线程的消息循环里执行，装在 UI 线程上会被
        //   渲染帧阻塞，超时后被 Windows 静默移除 → 松开事件丢失 → 菜单卡死）
        std::thread::spawn(|| unsafe {
            let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or(HMODULE(ptr::null_mut())).0);
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hinst, 0) {
                Ok(h) => {
                    HOOK_HANDLE.store(h.0 as isize, Ordering::SeqCst);
                    MOUSE_HOOK_LOCAL.store(h.0 as isize, Ordering::Relaxed);
                }
                Err(_) => {
                    eprintln!("[!] 鼠标钩子安装失败");
                    return;
                }
            }
            // ⚠️ 键盘钩子**不在这里装** —— 它只在菜单开着时才需要，见
            // `KEY_HOOK_WANT` 的注释。由循环顶部的 `sync_key_hook()` 按需装卸。
            HOOK_TID.store(GetCurrentThreadId(), Ordering::SeqCst);
            diag_push(DK_HOOK_ON, 0);
            // ⚠️⚠️ 这个循环是**整个项目最要命的一行**。
            //
            // 旧写法 `while GetMessageW(...).0 > 0 {}` 把三件事混成一件：
            //   > 0  正常收到消息 → 继续
            //   == 0 收到 WM_QUIT → 正常退出
            //   == -1 **出错**     → 旧写法也退出 ←←← 灾难
            // 而 `SetWindowsHookExW` 注册的钩子**不随线程退出自动摘除**。
            // 一旦这里因 -1 退出，内核里就留着一个「挂在死线程上的钩子」：
            //   · 系统仍把每个鼠标事件投进那条链 → 事件卡住，无人处理
            //   · 表现 = 只有已经在等这个事件的窗口能点，任务栏/其它窗口全点不动
            //   · 中键也被同一条死链吃掉 → 径向菜单根本弹不出来（菜单不显示！）
            // `quit_app` 的注释里记过这个现象（「每个鼠标事件都等一个死线程」），
            // 但只防住了「主动退出」这一条路，漏了「循环自己因错误退出」。
            let mut msg = MSG::default();
            loop {
                // 每次醒来先把键盘钩子同步到「该不该挂着」的状态。
                // （`need_key_hook()` 会用 `PostThreadMessageW` 把我们叫醒。）
                sync_key_hook();
                let r = GetMessageW(&mut msg, None, 0, 0).0;
                if r > 0 {
                    continue;
                }
                if r == 0 {
                    break; // WM_QUIT：stop_hook 主动叫停，正常
                }
                // -1 = 出错。**必须先摘钩子再退出**，否则留下毒化输入链的死钩子。
                diag_line(
                    &format!("{}\radial_log.txt", exe_dir()),
                    "HOOK_LOOP_ERR GetMessage 返回 -1，先摘钩子再退出",
                );
                break;
            }
            // 兜底：只要走到了这里（无论正常还是异常退场），确保钩子被摘掉。
            // 重复 UnhookWindowsHookEx 是安全的（第二次返回 0）。
            let mh = MOUSE_HOOK_LOCAL.load(Ordering::Relaxed);
            if mh != 0 {
                let _ = UnhookWindowsHookEx(HHOOK(mh as *mut c_void));
                MOUSE_HOOK_LOCAL.store(0, Ordering::Relaxed);
            }
            let kh = KEY_HOOK_LOCAL.load(Ordering::Relaxed);
            if kh != 0 {
                let _ = UnhookWindowsHookEx(HHOOK(kh as *mut c_void));
                KEY_HOOK_LOCAL.store(0, Ordering::Relaxed);
            }
        });
    } else {
        stop_hook();
        if MENU_OPEN.load(Ordering::Relaxed) {
            MENU_OPEN.store(false, Ordering::SeqCst);
            need_key_hook(false);
            let rh = RING_HWND.load(Ordering::Relaxed);
            if rh != 0 {
                let _ = ShowWindow(HWND(rh as *mut c_void), SW_HIDE);
            }
            let mh = MAIN_HWND.load(Ordering::Relaxed);
            if mh != 0 {
                let _ = KillTimer(HWND(mh as *mut c_void), TIMER_FRAME);
            }
        }
    }
    st.running = on;
    ST_RUNNING_FLAG.store(on, Ordering::SeqCst);
    println!("[*] 监听已{}", if on { "开启" } else { "停止" });
}

// ───────────────────── 悬停命中 ─────────────────────

fn update_hover(st: &mut State, sx: i32, sy: i32) {
    if st.hiding {
        return;
    }
    let px = sx - st.win_x;
    let py = sy - st.win_y;
    let dx = px as f32 - CC;
    let dy = py as f32 - CC;
    let n = st.apps.len();

    // 内环：距离最近的图标中心 < SEL+2 → 锁定为当前应用
    // 锁定后移向盘面任意位置（含外环 chips）都不会丢失当前应用
    st.hit_app = None;
    st.hover_now = None;
    if n > 0 {
        let (r_icon, icon_sz) = ring_layout(n);
        let hit_half = icon_sz * 0.5 + 7.0;
        let rr = (dx * dx + dy * dy).sqrt();
        let mut hit: Option<usize> = None;
        if rr <= r_icon + hit_half + 4.0 * K {
            let deg = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
            let per = 360.0 / n as f32;
            let i = ((deg / per).round() as usize).rem_euclid(n);
            let (ix, iy) = polar(r_icon, app_angle(i, n));
            let ddx = px as f32 - ix;
            let ddy = py as f32 - iy;
            if (ddx * ddx + ddy * ddy).sqrt() <= hit_half {
                hit = Some(i);
            }
        }
        st.hit_app = hit;
        st.hover_now = hit; // 高亮严格跟随光标：移出即恢复
        if let Some(i) = hit {
            if st.active_app != Some(i) {
                st.active_app = Some(i);
                st.hover_chip = None;
                st.switch0 = now_s();
            }
        }
    }

    // 外环段：**直接调 hit_chip_at**。
    // 这里曾经内联复制了一份「反旋到局部坐标判盒子」的判定 —— 形状一改
    // （胶囊 → 梯形环）就会「画在这、点到那」。命中判定**只能有一份实现**。
    st.hover_chip = None;
    if let Some(ai) = st.active_app {
        if ai < n {
            st.hover_chip = hit_chip_at(st, ai, px as f32, py as f32);
        }
    }
}

/// 眨眼开合度：在关键帧队列里插值。
/// 1.0 = 正常，0.05 = 闭合，>1 = 睁大过冲（参考实现是 1.08）。
fn blink_open(q: &[(f64, f32)], now: f64) -> f32 {
    if q.is_empty() {
        return 1.0;
    }
    if now <= q[0].0 {
        return q[0].1;
    }
    for i in 1..q.len() {
        if now <= q[i].0 {
            let (t0, v0) = q[i - 1];
            let (t1, v1) = q[i];
            let f = clamp01(((now - t0) / (t1 - t0).max(1e-6)) as f32);
            return lerp(v0, v1, f);
        }
    }
    q[q.len() - 1].1
}

/// 头像状态推进：注视平滑 + 自动眨眼。
///
/// 注视用**帧率无关的指数平滑** `k = 1 - exp(-5.66*dt)`（与 my-pet 一致）——
/// 用固定系数的话高帧率下会追得过快、低帧率下会拖沓。
fn face_tick(st: &mut State, now: f64) {
    let dt = if st.face_prev <= 0.0 {
        0.016
    } else {
        (now - st.face_prev).clamp(0.0, 0.1)
    };
    st.face_prev = now;

    // 注视目标 = 鼠标相对盘心的方向，归一化到 -1..1
    let bx = st.win_x as f32 + CC;
    let by = st.win_y as f32 + CC;
    let cx = CURSOR_X.load(Ordering::Relaxed) as f32;
    let cy = CURSOR_Y.load(Ordering::Relaxed) as f32;
    let rng = (disc_r(st.apps.len()) * 1.6).max(1.0);
    let tx = ((cx - bx) / rng).clamp(-1.0, 1.0);
    let ty = ((cy - by) / rng).clamp(-1.0, 1.0);
    let k = 1.0 - (-5.66 * dt as f32).exp();
    st.gaze[0] += (tx - st.gaze[0]) * k;
    st.gaze[1] += (ty - st.gaze[1]) * k;

    // 自动眨眼：预备(睁大) → 闭 → 过冲睁大 → 回落
    if st.blink_next <= 0.0 {
        st.blink_next = now + 1.0;
    }
    if now >= st.blink_next {
        st.blink_q.clear();
        st.blink_q.push((now, 0.05));
        st.blink_q.push((now + 0.070, 0.05));
        st.blink_q.push((now + 0.150, 1.08));
        st.blink_q.push((now + 0.300, 1.00));
        // 14% 概率连眨两下（参考实现的行为）
        if (((now * 977.0).fract().abs() * 1000.0) as u32) % 100 < 14 {
            st.blink_q.push((now + 0.370, 0.05));
            st.blink_q.push((now + 0.480, 1.00));
        }
        let rnd = (((now * 131.0).fract().abs() * 1000.0) as u32) % 2200;
        st.blink_next = now + 2.4 + rnd as f64 / 1000.0;
    }
    while st.blink_q.len() > 1 && st.blink_q[1].0 <= now {
        st.blink_q.remove(0);
    }
}

fn nav_app(st: &mut State, dir: i32) {
    let n = st.apps.len();
    if n == 0 {
        return;
    }
    let cur = match st.active_app {
        Some(i) => i as i32,
        None => {
            if dir > 0 {
                -1
            } else {
                0
            }
        }
    };
    let next = ((cur + dir) % n as i32 + n as i32) % n as i32;
    let next = next as usize;
    if st.active_app != Some(next) {
        st.active_app = Some(next);
        st.hover_chip = None;
        st.switch0 = now_s();
    }
}

fn nav_chip(st: &mut State, dir: i32) {
    let Some(ai) = st.active_app else {
        nav_app(st, dir);
        return;
    };
    let m = st.apps.get(ai).map(|a| a.items.len()).unwrap_or(0);
    if m == 0 {
        nav_app(st, dir);
        return;
    }
    let cur = st.hover_chip.map(|c| c as i32).unwrap_or(-1);
    let next = ((cur + dir) % m as i32 + m as i32) % m as i32;
    st.hover_chip = Some(next as usize);
}

fn unlock(st: &mut State) {
    st.locked = false;
    LOCKED.store(false, Ordering::SeqCst);
}

// ───────────────────── 帧驱动 ─────────────────────

#[allow(unused_assignments)] // 末尾的 moving=true 已覆盖前面所有分支
unsafe fn on_timer(st: &mut State) {
    if !MENU_OPEN.load(Ordering::Relaxed) {
        return;
    }
    // 把 UI 私有状态推给看门狗（跨线程不能直接读 st，只能靠镜像）
    SNAP_HIDING.store(st.hiding, Ordering::Relaxed);
    SNAP_ENTERING.store(st.entering, Ordering::Relaxed);
    SNAP_PENDING_RM.store(!st.pending_rms.is_empty(), Ordering::Relaxed);
    SNAP_FOCUS.store(focus_app(st).unwrap_or(usize::MAX), Ordering::Relaxed);
    SNAP_HOVER_CHIP.store(st.hover_chip.unwrap_or(usize::MAX), Ordering::Relaxed);
    SNAP_HIT_APP.store(st.hit_app.unwrap_or(usize::MAX), Ordering::Relaxed);
    let now = now_s();
    let mut alpha = 255u8;

    // ── 闲置自动收起（安全阀）──
    // 菜单开着却没有关闭路径时，它会一直悬着：此后每个鼠标事件都要先穿过
    // 我们的钩子判断一遍，用户感觉像「鼠标被什么吸住了」。
    // 超过 IDLE_CLOSE_AFTER 没有任何输入就自己收起。
    // 常驻模式（persistent）**不自动收** —— 那是用户明确要的常驻行为。
    if !st.hiding && !st.persistent && now - st.last_input >= IDLE_CLOSE_AFTER {
        // 倒计时移除 / OLE 拖放进行中都不动：那两种状态下自动收起会丢东西
        if st.pending_rms.is_empty() && !DRAG_ACTIVE.load(Ordering::Relaxed) {
            hide_menu(st);
        }
    }

    if st.hiding {
        let k = clamp01(((now - st.hide0) / 0.16) as f32);
        alpha = (255.0 * (1.0 - k)) as u8;
        if k >= 1.0 {
            close_menu_now(st);
            return;
        }
    }
    let entering_anim = st.entering;
    if st.entering {
        let k = clamp01(((now - st.ent0) / 0.32) as f32);
        if k >= 1.0 {
            st.entering = false;
        }
    }

    let mut moving = false;
    // ── 拖出移除：情绪过渡 / 拖动淡出 / 取消按钮出现 / 倒计时到点 ──
    {
        // 情绪过渡（帧率无关，≈120ms 到位）
        if st.mood_k < 1.0 {
            st.mood_k = (st.mood_k + (now - st.face_prev).max(0.0) as f32 * 8.0).min(1.0);
            moving = true;
        }
        // 拖动中的图标/胶囊：拖出盘面就淡掉
        let want_fade = if st.drag_app.is_some() || st.drag_chip.is_some() { 1.0 } else { 0.0 };
        if (st.drag_fade - want_fade).abs() > 0.004 {
            st.drag_fade += (want_fade - st.drag_fade) * 0.25;
            moving = true;
        } else {
            st.drag_fade = want_fade;
        }
        // 取消按钮出现
        let want_btn = if st.pending_rms.is_empty() { 0.0 } else { 1.0 };
        if (st.cancel_a - want_btn).abs() > 0.004 {
            st.cancel_a += (want_btn - st.cancel_a) * 0.30;
            moving = true;
        } else {
            st.cancel_a = want_btn;
        }
        // 倒计时到点 → 落盘。队列里可能一次到点多个（各自有 deadline）。
        if !st.pending_rms.is_empty() {
            let next = rm_soonest(st).unwrap_or(f64::INFINITY);
            if now >= next {
                commit_expired(st, now);
                if st.pending_rms.is_empty() {
                    st.cancel_a = 0.0;
                    set_mood(st, 0);
                } else {
                    moving = true;
                }
            } else {
                moving = true; // 倒计时期间必须持续重绘（边框在走）
            }
        }
        // 短暂情绪（如拖放悬停的「忙碌」）到点回落；待确认移除期间则回到生气
        if st.mood_hold > 0.0 && now >= st.mood_hold {
            st.mood_hold = 0.0;
            if st.mood == 5 {
                set_mood(st, if !st.pending_rms.is_empty() { 2 } else { 0 });
            }
        }
        // 发呆：菜单开着、无拖动、无待确认、无拖放，且一段时间没有任何输入
        if st.mood == 0
            && st.mood_hold == 0.0
            && st.drag_app.is_none()
            && st.drag_chip.is_none()
            && st.pending_rms.is_empty()
            && !st.drop_mode
            && now - st.last_input >= IDLE_MOOD_AFTER
        {
            set_mood(st, 3);
        }
    }
    // ── 改名编辑：光标要闪 → **必须持续重绘** ──
    // ⚠️⚠️ 这是「进了编辑态却看不到光标闪」的根因：`editing` 从来没参与过 `moving`
    // 的判断，于是编辑态只画一帧就静止，光标相位固定在 `edit_caret` 那一刻 ——
    // 落在「灭」的半周期上就**完全看不见**（用户 2026-10-09 报「不知道这是编辑模式」）。
    if st.editing.is_some() {
        moving = true;
    }

    // 悬停压暗渐变。焦点必须来自**真实命中**（见 focus_app）：
    // 用粘性的 active_app 会让「移到中心头像后」其它应用依旧蒙着灰。
    let focus = focus_app(st);
    for i in 0..st.apps.len().min(MAX_APPS) {
        let target = if focus.is_some() && focus != Some(i) { 1.0 } else { 0.0 };
        let d = target - st.app_anim[i];
        if d.abs() > 0.004 {
            st.app_anim[i] += d * 0.28;
            moving = true;
        } else {
            st.app_anim[i] = target;
        }
    }
    // chip 出现渐变
    for v in st.chip_anim.iter_mut() {
        let target = if st.active_app.is_some() { 1.0 } else { 0.0 };
        let d = target - *v;
        if d.abs() > 0.004 {
            *v += d * 0.26;
            moving = true;
        } else {
            *v = target;
        }
    }
    // 内环磁贴 hover/选中动效：高亮跟随光标（移出即灭）；拖放入环时落到谁身上谁亮
    for i in 0..st.apps.len().min(MAX_APPS) {
        let target =
            if st.hover_now == Some(i) || st.drop_target_app == Some(i) { 1.0 } else { 0.0 };
        let d = target - st.app_hot[i];
        if d.abs() > 0.003 {
            st.app_hot[i] += d * 0.30;
            moving = true;
        } else {
            st.app_hot[i] = target;
        }
    }
    // 外环 chip hover 动效
    for j in 0..MAX_ITEMS {
        let target = if st.hover_chip == Some(j) { 1.0 } else { 0.0 };
        let d = target - st.chip_hot[j];
        if d.abs() > 0.003 {
            st.chip_hot[j] += d * 0.34;
            moving = true;
        } else {
            st.chip_hot[j] = target;
        }
    }
    if st.commit_pulse > 0.0 {
        st.commit_pulse -= 0.016 / 0.22;
        if st.commit_pulse < 0.0 {
            st.commit_pulse = 0.0;
        }
        moving = true;
    }

    // 头像一直在呼吸/眨眼/漂移 —— 菜单打开期间需要持续重绘。
    //
    // 这里**必须每帧都跑**：注视（眼睛跟着鼠标转）是高频动作，
    // 之前节流到 30fps 会让整幅菜单实际只有 30fps —— 表现就是「转动时掉帧」。
    // face_tick 本身只有几次浮点运算，代价可忽略；真正的开销在 draw_scene，
    // 那部分靠下面「投影早退」优化。
    // 拖放进行中暂停表情球连续动画：OLE 的 DragOver 是跨进程等待回复的
    // 调用，线程必须随时空闲应答 —— 60fps 连续重绘会让回复排队，
    // 系统因此挂出忙碌指针。只有状态变化才出帧。
    if !DRAG_ACTIVE.load(Ordering::Relaxed) && !DROP_MODE_ARMED.load(Ordering::Relaxed) {
        st.face_n = st.face_n.wrapping_add(1);
        face_tick(st, now);
        moving = true;
    }

    let animating = st.hiding || entering_anim || moving;
    if animating {
        let ft0 = GetTickCount();
        frame(st, alpha);
        let fdt = GetTickCount().wrapping_sub(ft0);
        if fdt > 100 {
            diag_push(DK_UI_SLOW, fdt.min(0x00ff_ffff));
        }
    }
}

/// ULW 的 `AC_SRC_ALPHA` 要求缓冲区是**预乘 alpha**。
///
/// `blend_over` 走的是标准 source-over，产出的是**非预乘** RGBA。
/// 直接提交时系统按预乘解释，等于把每个半透明像素的原色**全亮度**叠上去 ——
/// 表现就是：本该是淡淡一层投影的地方，变成一整块实色
/// （用户看到的「环外面还有个半透明蓝色的环形块」，颜色正好等于 SHADOW）。
///
/// 注意：离屏 dump 不走这里 —— 它是把 alpha 正确合成到不透明背景上，
/// 所以 dump 一直是对的，只有真机窗口错了。这也是这个 bug 藏这么久的原因。
unsafe fn premultiply(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            continue;
        }
        px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
    }
}

unsafe fn frame(st: &mut State, alpha: u8) {
    if st.dbi_bits.is_null() {
        return;
    }
    let n = (CANVAS * CANVAS * 4) as usize;
    let buf = std::slice::from_raw_parts_mut(st.dbi_bits, n);
    draw_scene(st, buf);
    premultiply(buf);

    let rh = RING_HWND.load(Ordering::Relaxed);
    if rh == 0 {
        return;
    }
    let dst = POINT { x: st.win_x, y: st.win_y };
    let src = POINT::default();
    let size = SIZE { cx: CANVAS, cy: CANVAS };
    let blend = BLENDFUNCTION {
        BlendOp: 0u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: 1u8,
    };
    let _ = UpdateLayeredWindow(
        HWND(rh as *mut c_void),
        HDC(ptr::null_mut()),
        Some(&dst),
        Some(&size),
        st.dbi_dc,
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
}

// ───────────────────── 菜单生命周期 ─────────────────────

unsafe fn show_menu(st: &mut State, sx: i32, sy: i32) {
    let rh = RING_HWND.load(Ordering::Relaxed);
    if rh == 0 {
        return;
    }
    let hwnd = HWND(rh as *mut c_void);

    // 画布钳位到虚拟屏幕
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    st.win_x = (sx - CANVAS / 2).clamp(vx, vx + vw - CANVAS);
    st.win_y = (sy - CANVAS / 2).clamp(vy, vy + vh - CANVAS);

    st.active_app = None;
    st.hover_chip = None;
    st.app_anim = [0.0; MAX_APPS];
    st.app_hot = [0.0; MAX_APPS];
    st.chip_anim = [0.0; MAX_ITEMS];
    st.chip_hot = [0.0; MAX_ITEMS];
    st.commit_pulse = 0.0;
    st.commit_app = usize::MAX;
    st.commit_chip = usize::MAX;
    st.hiding = false;
    st.entering = true;
    st.ent0 = now_s();
    st.face0 = now_s();
    st.face_prev = 0.0;
    st.gaze = [0.0, 0.0];
    st.last_input = now_s();
    st.cur_x = sx;
    st.cur_y = sy;
    st.mood_hold = 0.0;
    diag_push(DK_MENU_OPEN, 0);
    st.blink_q.clear();
    st.blink_next = 0.0;
    st.switch0 = now_s();
    st.locked = false;
    LOCKED.store(false, Ordering::SeqCst);

    MENU_OPEN.store(true, Ordering::SeqCst);
    // 菜单开了 → 现在才需要键盘钩子（Esc/回车/方向键/改名输入）
    need_key_hook(true);
    let _ = ShowWindow(hwnd, SW_HIDE);
    let _ = SetWindowPos(hwnd, HWND_TOPMOST, st.win_x, st.win_y, CANVAS, CANVAS, SWP_NOACTIVATE);
    frame(st, 255);
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        st.win_x,
        st.win_y,
        CANVAS,
        CANVAS,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let mh = MAIN_HWND.load(Ordering::Relaxed);
    if mh != 0 {
        let _ = SetTimer(HWND(mh as *mut c_void), TIMER_FRAME, 16, None);
    }
}

unsafe fn hide_menu(st: &mut State) {
    if st.hiding {
        return;
    }
    st.hiding = true;
    st.hide0 = now_s();
    let mh = MAIN_HWND.load(Ordering::Relaxed);
    if mh != 0 {
        let _ = SetTimer(HWND(mh as *mut c_void), TIMER_FRAME, 16, None);
    }
}

unsafe fn close_menu_now(st: &mut State) {
    MENU_OPEN.store(false, Ordering::SeqCst);
    // 菜单关了 → 键盘钩子立即摘掉（打字路径上不留我们的钩子）
    need_key_hook(false);
    LOCKED.store(false, Ordering::SeqCst);
    st.locked = false;
    st.hiding = false;
    // 菜单收起 = 待执行的延迟单击作废。否则它到期后会对着
    // 「菜单已经没了」的状态去 commit —— 凭空启动一个应用。
    //
    // ⚠️ `KillTimer` 的窗口必须和 `SetTimer` 的一致：`SetTimer` 是在
    // **MAIN_HWND** 上装的（`WM_APP_CLICK` 被 post 到 MAIN_HWND，所以那里
    // 的 `hwnd` 就是它），早先在 RING_HWND 上 Kill → 根本没杀掉，
    // 留下一个每 450ms 响一次的重复定时器。
    let mh = MAIN_HWND.load(Ordering::Relaxed);
    if mh != 0 {
        clear_delayed_click(st, HWND(mh as *mut c_void));
    } else {
        st.pending_click = None;
    }
    st.edit_last_click = -99.0;
    // ⚠️ 这里**不要**清 R_DOWN_AT！清了会造出反向 bug：
    // 右键用来关菜单时 down 已被吞、up 到达时 t0==0 → 被放行 → 应用收到一个
    // **孤立 RIGHTUP**（没有配对的 DOWN）→ 桌面会凭空弹出右键菜单。
    // R_DOWN_AT 的收尾交给配对的 UP（见 mouse_hook_impl 的不变式）。
    // 菜单收起 = 待移除的内容放回原位（倒计时只在菜单开着时走；
    // 内容此刻已从盘面摘下，不放回就等于偷偷删掉了）
    cancel_remove(st);
    st.drag_app = None;
    st.drag_chip = None;
    st.drag_over_app = None;
    st.drag_over_chip = None;
    st.drag_fade = 0.0;
    st.mood_hold = 0.0;
    // 改名态必须一起清：否则 EDIT_ARMED 残留，键盘钩子会一直吞字符
    st.editing = None;
    st.edit_buf.clear();
    EDIT_ARMED.store(false, Ordering::SeqCst);
    set_mood(st, 0);
    diag_push(DK_MENU_CLOSE, 0);
    let mh = MAIN_HWND.load(Ordering::Relaxed);
    if mh != 0 {
        let _ = KillTimer(HWND(mh as *mut c_void), TIMER_FRAME);
    }
    let rh = RING_HWND.load(Ordering::Relaxed);
    if rh != 0 {
        let _ = ShowWindow(HWND(rh as *mut c_void), SW_HIDE);
    }
}

// ───────────────────── 托盘宿主窗口过程 ─────────────────────

fn mif(on: bool) -> MENU_ITEM_FLAGS {
    MENU_ITEM_FLAGS(MF_STRING.0 | if on { MF_CHECKED.0 } else { MF_UNCHECKED.0 })
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if w.0 as i32 == HOTKEY_TOGGLE {
                let st = state_of(hwnd);
                let on = !st.running;
                let st2 = state_of(hwnd);
                set_running(st2, on);
            }
            LRESULT(0)
        }
        WM_APP_SHOW => {
            let st = state_of(hwnd);
            if !MENU_OPEN.load(Ordering::Relaxed) {
                let mut x = CURSOR_X.load(Ordering::Relaxed);
                let mut y = CURSOR_Y.load(Ordering::Relaxed);
                if x == 0 && y == 0 {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    x = p.x;
                    y = p.y;
                }
                show_menu(st, x, y);
            }
            LRESULT(0)
        }
        WM_APP_HOVER => {
            HOVER_POSTED.store(false, Ordering::Relaxed);
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                let mut x = CURSOR_X.load(Ordering::Relaxed);
                let mut y = CURSOR_Y.load(Ordering::Relaxed);
                if x == 0 && y == 0 {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    x = p.x;
                    y = p.y;
                }
                note_input(st, x, y);
                // 拖着外环胶囊时冻结 hover：光标一挪 active_app 就可能换人，
                // chip 布局跟着洗牌，抓在手里的那颗就不知道飞哪去了
                if st.drag_chip.is_none() {
                    update_hover(st, x, y);
                }
            }
            LRESULT(0)
        }
        WM_APP_COMMIT => {
            if CTX_OPEN.load(Ordering::Relaxed) {
                return LRESULT(0);
            }
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                if st.persistent {
                    // 常驻模式：松开触发键后保持显示，改由点击/回车确认
                    st.locked = true;
                    LOCKED.store(true, Ordering::SeqCst);
                    println!("[*] 已锁定 · 点击或回车确认，Esc 关闭");
                } else {
                    // 非常驻模式：松开即执行 —— 但必须真的停在某个目标上。
                    // 旧逻辑无条件 commit，导致「刚呼出就松手」也会启动上次悬停的应用。
                    let x = CURSOR_X.load(Ordering::Relaxed);
                    let y = CURSOR_Y.load(Ordering::Relaxed);
                    update_hover(st, x, y);
                    if st.hover_chip.is_some() || st.hit_app.is_some() {
                        commit(st);
                    } else {
                        unlock(st);
                        close_menu_now(st);
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_DRAGEVENT => {
            let st = state_of(hwnd);
            if !MENU_OPEN.load(Ordering::Relaxed) {
                return LRESULT(0);
            }
            let (sx, sy) = unpack_pt(DROP_PT.load(Ordering::Relaxed));
            match w.0 {
                // 进入 / 移动：高亮落点应用（图标磁贴，或当前应用的外环胶囊区）
                0 | 1 => {
                    st.drop_mode = true;
                    DROP_MODE_ARMED.store(true, Ordering::SeqCst);
                    st.drop_target_app = drop_force_app(st, sx, sy);
                    // 文件悬停在环上 = 收下前的「忙碌」；drag_step 在拖放开始时
                    // 可能设过「紧张」，进环就放松下来
                    set_mood(st, 0);
                }
                // 离开环区域
                2 => {
                    st.drop_target_app = None;
                    set_mood(st, 0);
                }
                // 落下：识别入库
                _ => {
                    let force_app = drop_force_app(st, sx, sy);
                    let items = DROP_PAYLOAD
                        .lock()
                        .map(|mut q| std::mem::take(&mut *q))
                        .unwrap_or_default();
                    DRAG_ACTIVE.store(false, Ordering::SeqCst);
                    DROP_MODE_ARMED.store(false, Ordering::SeqCst);
                    st.drop_mode = false;
                    if !items.is_empty() {
                        let n = process_drop(st, &items, force_app);
                        drag_log(&format!("Drop processed, added={}", n));
                        println!("[drop] 入环 {} 项", n);
                        st.commit_pulse = 1.0;
                        if n > 0 {
                            // 「忙碌」：低头消化刚进环的新成员（短暂情绪，到点自动回落）
                            set_mood(st, 5);
                            st.mood_hold = now_s() + 1.5;
                        }
                        // 常驻模式：保持打开，让用户亲眼看到新胶囊出现
                        if !(st.persistent && st.locked) {
                            unsafe { hide_menu(st) };
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_CHAR => {
            let st = state_of(hwnd);
            if st.editing.is_some() {
                if let Some(c) = char::from_u32(w.0 as u32) {
                    edit_push(st, c);
                    frame(st, 255);
                }
            }
            LRESULT(0)
        }
        // 仅自动化测试：双击改名要真鼠标 + 中文 IME，自动化跑不了。
        // 这条诊断消息和真实消息**调用同一批函数**，只是把输入摆出来。
        //
        // ⚠️ phase 0 会**自己**把 active_app 设成条目最多的那个应用 ——
        // 正常流程靠中键呼出菜单 + hover 选中，那条路要真鼠标钩子，
        // 在有僵尸实例的环境里会被它们的钩子吞掉，测不到真正的逻辑。
        WM_APP_EDITSIM => {
            let st = state_of(hwnd);
            // l 的高位（>= 1000）指定要选哪个应用下标
            let want_ai = if l.0 >= 1000 { Some((l.0 - 1000) as usize) } else { None };
            let ai = match want_ai {
                Some(a) if a < st.apps.len() => a,
                _ => st
                    .apps
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, a)| a.items.len())
                    .map(|(i, _)| i)
                    .unwrap_or(0),
            };
            if w.0 == 0 {
                st.active_app = Some(ai);
                st.hover_chip = Some(0);
            }
            if st.apps.get(ai).map(|a| !a.items.is_empty()).unwrap_or(false) {
                let j = if l.0 < 1000 { l.0 as usize } else { 0 };
                match w.0 {
                    0 => {
                        let now = now_s();
                        st.edit_last_click = now;
                        st.edit_last_pos =
                            (CURSOR_X.load(Ordering::Relaxed), CURSOR_Y.load(Ordering::Relaxed));
                        chip_click(st, ai, j, st.edit_last_pos.0, st.edit_last_pos.1);
                        println!("[editsim] 进入编辑 app={} chip={} buf={:?}",
                            ai, j, st.edit_buf);
                    }
                    1 => {
                        if let Some(c) = char::from_u32(l.0 as u32) {
                            edit_push(st, c);
                        }
                    }
                    2 => {
                        st.edit_buf.pop();
                        st.edit_caret = now_s();
                    }
                    3 => {
                        unsafe { edit_finish(st, true) };
                    }
                    4 => {
                        unsafe { edit_finish(st, false) };
                    }
                    5 => {
                        st.edit_buf.clear();
                        unsafe { edit_finish(st, true) };
                    }
                    // 仅测试：**只设** active_app / hover_chip，不进编辑态。
                    // 自动化要触发「单击胶囊」走真 `chip_click` 路径，但真光标会被
                    // 环境里的僵尸钩子吞掉、`update_hover()` 算不出 hover_chip。
                    // phase 0 也不行 —— 它会顺手 chip_click() 进编辑态，把后续全带偏。
                    9 => {
                        st.editing = None;
                        st.edit_buf.clear();
                        EDIT_ARMED.store(false, Ordering::SeqCst);
                        st.active_app = Some(ai);
                        st.hover_chip = Some(j);
                        FORCE_CHIP.store(j as i64, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_APP_KEY => {
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                // 改名模式优先：编辑键的语义与全局快捷键不同
                if st.editing.is_some() {
                    match w.0 {
                        KA_EDIT_COMMIT => {
                            edit_finish(st, true);
                            frame(st, 255);
                        }
                        KA_EDIT_CANCEL => {
                            edit_finish(st, false);
                            frame(st, 255);
                        }
                        KA_EDIT_BACK => {
                            st.edit_buf.pop();
                            if st.edit_started < 0.0 {
                                st.edit_started = 0.0;
                            }
                            st.edit_caret = now_s();
                            frame(st, 255);
                        }
                        KA_EDIT_DEL => {
                            // 光标在末尾，Delete 等价于 Backspace（单行编辑）
                            st.edit_buf.pop();
                            st.edit_caret = now_s();
                            frame(st, 255);
                        }
                        _ => {}
                    }
                    return LRESULT(0);
                } else {
                    match w.0 {
                        KA_CLOSE => {
                            unlock(st);
                            close_menu_now(st);
                        }
                        KA_CONFIRM => commit(st),
                        KA_PASTE => unsafe { paste_add(st) },
                        KA_PREV => {
                            if st.hover_chip.is_some() {
                                nav_chip(st, -1)
                            } else {
                                nav_app(st, -1)
                            }
                        }
                        KA_NEXT => {
                            if st.hover_chip.is_some() {
                                nav_chip(st, 1)
                            } else {
                                nav_app(st, 1)
                            }
                        }
                        _ => {}
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_CLICK => {
            if CTX_OPEN.load(Ordering::Relaxed) {
                return LRESULT(0);
            }
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                let x = CURSOR_X.load(Ordering::Relaxed);
                let y = CURSOR_Y.load(Ordering::Relaxed);
                note_input(st, x, y);
                update_hover(st, x, y);
                // 仅测试：`FORCE_CHIP` >= 0 时用它顶替 hover 判定。
                // `update_hover()` 依赖**真鼠标**写的 CURSOR_X/Y，自动化环境里
                // 僵尸钩子会吞掉鼠标事件 → hover 算不出来 → 走不到 chip_click。
                // 这个旁路只在本进程被诊断消息驱动时生效，正常路径永远走 hover。
                if FORCE_CHIP.load(Ordering::Relaxed) >= 0 {
                    if let Some(ai) = st.active_app {
                        st.hover_chip = Some(FORCE_CHIP.load(Ordering::Relaxed) as usize);
                        st.hit_app = Some(ai);
                    }
                }
                // 取消按钮优先：它在倒计时期间要能点
                if hit_rm_btn(st, (x - st.win_x) as f32, (y - st.win_y) as f32) {
                    cancel_remove(st);
                    frame(st, 255);
                } else if let Some(ai) = st.active_app {
                    // 胶囊点击：先判双击（改名），否则**延迟**这一击再决定执行与否。
                    if let Some(j) = st.hover_chip {
                        match chip_click(st, ai, j, x, y) {
                            Some(ChipClick::Double) => {
                                // 第二击 → 取消待执行的一击，进改名
                                clear_delayed_click(st, hwnd);
                                frame(st, 255);
                                return LRESULT(0);
                            }
                            Some(ChipClick::Defer) => {
                                // 第一击 → 排延迟执行，**这一击不执行**
                                arm_delayed_click(st, hwnd, ai, j); 
                                frame(st, 255);
                                return LRESULT(0);
                            }
                            None => {} // 已在改名态，点别处放弃编辑 → 继续往下
                        }
                    }
                    if st.hover_chip.is_some() || st.hit_app.is_some() {
                        // 只有真正点在「外环 chip」或「内环应用图标」上才执行。
                        // 旧逻辑用 st.active_app.is_some() 判定，但 active_app 是粘性的
                        // （移向外环时不丢），结果点屏幕任意位置都会启动当前应用。
                        commit(st);
                    } else {
                        unlock(st);
                        close_menu_now(st);
                    }
                } else {
                    unlock(st);
                    close_menu_now(st);
                }
            }
            LRESULT(0)
        }
        WM_APP_DRAGSTEP => {
            DRAG_POSTED.store(false, Ordering::Relaxed);
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) && !DRAG_ACTIVE.load(Ordering::Relaxed) {
                drag_step(
                    st,
                    CURSOR_X.load(Ordering::Relaxed),
                    CURSOR_Y.load(Ordering::Relaxed),
                );
            }
            LRESULT(0)
        }
        WM_APP_DRAGEND => {
            if CTX_OPEN.load(Ordering::Relaxed) {
                return LRESULT(0);
            }
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                drag_end(
                    st,
                    CURSOR_X.load(Ordering::Relaxed),
                    CURSOR_Y.load(Ordering::Relaxed),
                );
            }
            LRESULT(0)
        }
        // 仅自动化测试：拖放/拖出这套状态机没法用合成鼠标事件驱动
        // （低级钩子只认真实事件），所以留一条诊断消息直接走同一条代码路径。
        // w = (y<<16)|x 屏幕坐标；l = 0 拖动中 / 1 松手 / 2 只挪光标。
        WM_APP_DRAGSIM => {
            let st = state_of(hwnd);
            let x = (w.0 & 0xFFFF) as i32;
            let y = ((w.0 >> 16) & 0xFFFF) as i32;
            CURSOR_X.store(x, Ordering::Relaxed);
            CURSOR_Y.store(y, Ordering::Relaxed);
            LB_DOWN_X.store(x as isize, Ordering::Relaxed);
            LB_DOWN_Y.store(y as isize, Ordering::Relaxed);
            if MENU_OPEN.load(Ordering::Relaxed) {
                match l.0 {
                    0 => drag_step(st, x, y),
                    1 => drag_end(st, x, y),
                    _ => {} // 2 = 只挪光标（供测试点取消按钮）
                }
            }
            LRESULT(0)
        }
        WM_APP_RHOLDSIM => {
            // 仅测试：驱动右键长按状态机（见常量处的说明）。
            let ev = match w.0 {
                0 => Some(WM_RBUTTONDOWN),
                1 => Some(WM_RBUTTONUP),
                _ => None,
            };
            let mut consumed = 0usize;
            if let Some(m) = ev {
                if unsafe { rhold_step(m) }.is_some() {
                    consumed = 1;
                }
            }
            let st = (if R_DOWN_AT.load(Ordering::Relaxed) != 0 { 1usize } else { 0 }) << 1
                | (if R_DOWN_INMENU.load(Ordering::Relaxed) { 1 } else { 0 }) << 2
                | (if MENU_OPEN.load(Ordering::Relaxed) { 1 } else { 0 }) << 5
                | ((REINJECT_POSTED.load(Ordering::Relaxed) as usize) & 0xff) << 8
                | ((R_DOWN_SEEN.load(Ordering::Relaxed) as usize) & 0xff) << 16
                | ((MAGIC_SEEN.load(Ordering::Relaxed) as usize) & 0x7f) << 24
                | ((INJECT_DONE.load(Ordering::Relaxed) as usize) & 0x7f) << 31;
            LRESULT((consumed | st) as isize)
        }
WM_APP_DISMISS_FG => {
            // 长按右键呼出我们菜单时，被「放行」的那个 UP 会让前台程序弹出它自己
            // 的右键菜单 —— 这里补一次 Esc 把它关掉，免得我们的菜单背后挂着别人的菜单。
            //
            // ⚠️ 用 `PostMessage(WM_KEYDOWN/WM_KEYUP)` 这种**窗口消息**，
            // 不要用 `keybd_event`/`SendInput` 那种**输入注入**：
            // 本进程装着全局钩子时注入不可靠（可能永久卡住调用线程）。
            // 窗口消息走的是应用自己的消息队列，安全、异步、失败也无害。
            //
            // 也**不要**在钩子回调里做 —— 那里只许原子操作 + PostMessage。
            let fg = FG_HWND.load(Ordering::Relaxed);
            if fg != 0 {
                let h = HWND(fg as *mut c_void);
                unsafe {
                    let _ = PostMessageW(h, WM_KEYDOWN, WPARAM(0x1B), LPARAM(0));
                    let _ = PostMessageW(h, WM_KEYUP, WPARAM(0x1B), LPARAM(0));
                }
            }
            LRESULT(0)
        }
        WM_APP_CANCEL => {
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) && !CTX_OPEN.load(Ordering::Relaxed) {
                // 右键 / 点空白 / Esc = **一律收起菜单**。
                // 曾经这里会「右键命中胶囊 → 呼出改名/换色菜单」——
                // 那是用户明确否掉的方案（2026-10-07：「去掉换色去掉菜单，
                // 双击直接就在胶囊里改名」）。改名只有一条路：`WM_APP_CLICK`
                // 里 `chip_click()` 判定双击 → 进入 `editing` 就地编辑。
                unlock(st);
                close_menu_now(st);
            }
            LRESULT(0)
        }
        WM_APP_CHIPPOS => {
            // 仅测试：回报**第 `w` 号**胶囊中心的屏幕坐标。
            // 单击/拖拽路径都靠真实光标命中，自动化必须知道胶囊在哪；
            // 外环是极坐标形状（扇形段），坐标算不准就点不到/拖不中 —— 所以由程序侧给出。
            // 下标越界时返回 0（测试据此判断「没有这个段」）。
            let st = state_of(hwnd);
            let want = w.0;
            let mut packed: usize = 0;
            if let Some(ai) = st.active_app.filter(|&a| a < st.apps.len()) {
                let (segs, _rots, rads) = chip_layout(st, ai);
                if want < segs.len() {
                    let s = segs[want];
                    let (cx, cy) = polar(rads[want], (s.0 + s.1) * 0.5);
                    let sx = (cx as i32 + st.win_x).clamp(0, 65535);
                    let sy = (cy as i32 + st.win_y).clamp(0, 65535);
                    packed = ((sy as usize) << 16) | (sx as usize);
                }
            }
            LRESULT(packed as isize)
        }
        WM_APP_DUMP => {
            let st = state_of(hwnd);
            println!(
                "[dump] open={} active={:?} chip={:?} win=({},{})",
                MENU_OPEN.load(Ordering::Relaxed),
                st.active_app,
                st.hover_chip,
                st.win_x,
                st.win_y
            );
            if !st.dbi_bits.is_null() {
                let n = (CANVAS * CANVAS * 4) as usize;
                let buf = std::slice::from_raw_parts(st.dbi_bits, n);
                let path = format!("{}\\live_dump.bmp", exe_dir());
                let ok = save_bmp(&path, buf, CANVAS, CANVAS);
                println!("[dump] {} {}", if ok { "OK" } else { "FAIL" }, path);
            }
            LRESULT(0)
        }
        WM_APP_DEBUGPOS => {
            // 自动化测试：w = x, l = y
            let st = state_of(hwnd);
            if MENU_OPEN.load(Ordering::Relaxed) {
                CURSOR_X.store(w.0 as i32, Ordering::Relaxed);
                CURSOR_Y.store(l.0 as i32, Ordering::Relaxed);
                update_hover(st, w.0 as i32, l.0 as i32);
            }
            LRESULT(0)
        }
        WM_APP_DROPTEST => {
            let st = state_of(hwnd);
            let hit = drop_force_app(st, w.0 as i32, l.0 as i32);
            println!("[droptest] ({}, {}) -> {}", w.0 as i32, l.0 as i32,
                match hit { Some(i) => i.to_string(), None => "None".into() });
            LRESULT(0)
        }
        WM_TIMER => {
            if w.0 as usize == TIMER_FRAME {
                let st = state_of(hwnd);
                on_timer(st);
            } else if w.0 as usize == TIMER_CLICK {
                // 「延迟单击」到点 = 确认没有第二击 → 真的执行
                let st = state_of(hwnd);
                if let Some((ai, j)) = st.pending_click.take() {
                    let _ = KillTimer(hwnd, TIMER_CLICK);
                    // 期间菜单可能被收了 / hover 变了 → 这时候执行会启动错的东西
                    if MENU_OPEN.load(Ordering::Relaxed)
                        && st.active_app == Some(ai)
                        && st.hover_chip == Some(j)
                        && st.editing.is_none()
                    {
                        st.hover_chip = Some(j);
                        commit(st);
                    }
                }
            }
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let lp = l.0 as u32;
            if lp == WM_RBUTTONUP || lp == WM_LBUTTONUP {
                let st = state_of(hwnd);
                let menu = CreatePopupMenu().unwrap_or(HMENU(ptr::null_mut()));
                let t0: Vec<u16> = wide("设置…");
                let _ = AppendMenuW(menu, mif(false), MENU_SETTINGS as usize, PCWSTR(t0.as_ptr()));
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                let t1: Vec<u16> = wide(if st.running { "停止监听" } else { "开启监听" });
                let _ = AppendMenuW(menu, mif(false), MENU_TOGGLE as usize, PCWSTR(t1.as_ptr()));
                let t2: Vec<u16> = wide("常驻模式（松开后保持）");
                let _ = AppendMenuW(menu, mif(st.persistent), MENU_PERSIST as usize, PCWSTR(t2.as_ptr()));
                let t3: Vec<u16> = wide("深色盘面（piik 暗色）");
                let _ = AppendMenuW(menu, mif(st.glass_liquid), MENU_LIQUID as usize, PCWSTR(t3.as_ptr()));
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                let t5: Vec<u16> = wide("打开配置文件");
                let _ = AppendMenuW(menu, mif(false), MENU_OPEN_INI as usize, PCWSTR(t5.as_ptr()));
                let t6: Vec<u16> = wide("重新加载配置");
                let _ = AppendMenuW(menu, mif(false), MENU_RELOAD as usize, PCWSTR(t6.as_ptr()));
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                let t7: Vec<u16> = wide("退出");
                let _ = AppendMenuW(menu, mif(false), MENU_QUIT as usize, PCWSTR(t7.as_ptr()));

                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                let _ = SetForegroundWindow(hwnd);
                let cmd = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    p.x,
                    p.y,
                    0,
                    hwnd,
                    None,
                );
                let _ = DestroyMenu(menu);
                let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));

                let st = state_of(hwnd);
                match cmd.0 as u16 {
                    MENU_SETTINGS => open_settings(st),
                    MENU_TOGGLE => {
                        let on = !st.running;
                        set_running(st, on);
                        save_settings(st);
                    }
                    MENU_PERSIST => {
                        st.persistent = !st.persistent;
                        save_settings(st);
                        println!("[*] 常驻模式：{}", if st.persistent { "开" } else { "关" });
                    }
                    MENU_LIQUID => {
                        // 托盘这一项只在「亮 / 暗」之间切；跟随系统在设置页里选
                        st.theme = if st.glass_liquid { 0 } else { 1 };
                        apply_theme(st);
                        save_settings(st);
                        println!("[*] 盘面配色：{}", if st.glass_liquid { "暗色" } else { "亮色" });
                        if MENU_OPEN.load(Ordering::Relaxed) {
                            frame(st, 255);
                        }
                    }
                    MENU_OPEN_INI => {
                        let p: Vec<u16> = wide(&st.ini_path);
                        let _ = ShellExecuteW(
                            HWND(ptr::null_mut()),
                            PCWSTR::null(),
                            PCWSTR(p.as_ptr()),
                            PCWSTR::null(),
                            PCWSTR::null(),
                            SW_SHOWNORMAL,
                        );
                    }
                    MENU_RELOAD => load_ini(st),
                    MENU_QUIT => unsafe { quit_app() },
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            let _ = UnregisterHotKey(hwnd, HOTKEY_TOGGLE);
            unsafe { quit_app() };
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

// ═════════════════════ 设置窗口（GDI 自绘 · 扁平 piik） ═════════════════════
//
// 为什么全部自绘、不用子控件：整机视觉是黑白灰扁平语言，主题化的原生控件
// （按钮/滚动条/复选框）会立刻把风格破掉。这里所有控件都用 GDI RoundRect +
// DrawTextW 画在**窗口 DC** 上 —— 窗口 DC 上 GDI 是完全可靠的，不存在
// 32bpp 内存 DIB 那套 Pie/AngleArc/alpha 的坑。
// 命中判定与绘制共用同一套 rect 计算函数（和 chip_layout 同一个道理）。

const S_W: i32 = 780;
const S_H: i32 = 580;
const S_NAV_W: i32 = 172;
const S_TITLE_H: i32 = 52;
const S_X0: i32 = S_NAV_W + 26;      // 内容区左边界
const S_X1: i32 = S_W - 26;          // 内容区右边界
const S_LIST_TOP: i32 = S_TITLE_H + 58;
const S_LIST_BOT: i32 = S_H - 78;

// ── 渲染管线：整页先在 SS 倍大的 DIB 上用 GDI 画，再 SS×SS 盒式降采样 ──
//
// 为什么必须这样：**GDI 的 RoundRect / Ellipse 完全没有抗锯齿**，直接画到
// 窗口上就是楼梯（chip 圆角、开关胶囊、导航胶囊全是一格一格的）；
// 文字还会走 ClearType 出彩色边（放大看「鼠标中键」边上全是橙蓝锯齿）。
// 先在 3× 画再平均，等于给整页做了一次 AA —— 彩色边也在平均里被抹成灰阶，
// 和环那边「2× 采样 + SDF 覆盖率」是同一个思路。
const SS: i32 = 3;
/// 卡片外留一圈画投影；窗口 = 卡片 + 四周各 S_M
const S_M: i32 = 22;
const S_WW: i32 = S_W + S_M * 2;
const S_HH: i32 = S_H + S_M * 2;
const CARD_RAD: i32 = 18;
/// ⚠️ windows-rs 0.58 里 `WM_MOUSELEAVE` 在 `Win32::UI::Controls`，**不在** WindowsAndMessaging。
/// 直接写 `WM_MOUSELEAVE` 编译不报错 —— match 里的未知标识符会**静默变成匹配一切的绑定**，
/// 于是这条臂把 WM_NCCREATE 也吃掉，而那时 USERDATA 还没设 → state_of() 空指针 → 进程崩在回调里。
/// 所以这里显式给个自带名字的常量，别依赖导入。
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;

/// 逻辑坐标（卡片左上为 0,0）→ hi DIB 的设备坐标
#[inline]
fn sxv(v: i32) -> i32 {
    (v + S_M) * SS
}

/// 呼出键开关。钩子跑在**另一个线程**上，读不到 State，所以用原子量。
static OPT_MIDDLE: AtomicBool = AtomicBool::new(true);
static OPT_SIDE: AtomicBool = AtomicBool::new(true);
static OPT_RHOLD: AtomicBool = AtomicBool::new(false);
/// 右键按下的 tick（长按判定），0 = 当前没按着。
/// **非 0 = 我们吞掉了这次 RBUTTONDOWN，欠应用一个交代** ——
/// 这个「欠账」必须由配对的 RBUTTONUP 收尾（补发 or 呼出菜单）。
static R_DOWN_AT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 吞掉那次 RBUTTONDOWN 时，菜单是否已经开着。
/// 用途：松开时判断这次右键是「用户在关菜单」还是「用户想点桌面」——
/// 前者不补发（否则右键会透传到桌面弹出资源管理器菜单），后者才补发。
static R_DOWN_INMENU: AtomicBool = AtomicBool::new(false);
/// 诊断计数：一共发出过几次「补发右键」请求。自动化据此验证配对是否成立。
static REINJECT_POSTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 诊断计数：一共吞掉过几次 RBUTTONDOWN。
static R_DOWN_SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 长按右键那一刻的前台窗口 —— 稍后要给它补一次 Esc 关掉它的右键菜单。
static FG_HWND: AtomicIsize = AtomicIsize::new(0);
/// 诊断：钩子见到多少个「带 RL_MAGIC 标记」的事件（应为补发次数×2）。
static MAGIC_SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 诊断：补发请求实际执行完的次数。
static INJECT_DONE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SET_HWND: AtomicIsize = AtomicIsize::new(0);
/// 长按右键的判定门槛（毫秒）
const RHOLD_MS: u32 = 380;
/// 给「我们自己补发的合成鼠标事件」打的标记（写进 dwExtraInfo），
/// 钩子里据此放行，避免补发事件被自己再吞一次形成死循环
const RL_MAGIC: usize = 0x524C_4D31; // 'RLM1'

/// 设置页控件标识 —— 绘制与命中共用同一个枚举，杜绝「画在这、点在那」
#[derive(Clone, Copy, PartialEq)]
enum Ctl {
    None,
    Close,
    Nav(usize),
    Theme(usize),
    Key(usize),
    Sw(usize),
    AppHead(usize),
    AppDel(usize),
    ItemDel(usize, usize),
    AddApp,
    AddItem(usize),
}

// ── 应用页的行（随展开状态/滚动量变化，绘制与命中共用） ──
struct SRow {
    kind: u8, // 0 应用 1 条目 2 「＋ 添加条目」
    ai: usize,
    ii: usize,
    y: i32,
    h: i32,
}

fn s_rows(st: &State) -> (Vec<SRow>, i32) {
    let mut v = Vec::new();
    let mut y = 0i32;
    for (ai, a) in st.apps.iter().enumerate() {
        v.push(SRow { kind: 0, ai, ii: 0, y, h: 42 });
        y += 42;
        if st.set_expanded == ai {
            for ii in 0..a.items.len() {
                v.push(SRow { kind: 1, ai, ii, y, h: 32 });
                y += 32;
            }
            v.push(SRow { kind: 2, ai, ii: 0, y, h: 32 });
            y += 32;
        }
        y += 6;
    }
    (v, y)
}

fn s_scroll_max(st: &State) -> i32 {
    let (_, total) = s_rows(st);
    (total - (S_LIST_BOT - S_LIST_TOP)).max(0)
}

fn s_nav_rect(i: usize) -> RECT {
    let t = S_TITLE_H + 18 + i as i32 * 44;
    RECT { left: 14, top: t, right: S_NAV_W - 14, bottom: t + 40 }
}

fn s_row_rect(y: i32, h: i32) -> RECT {
    RECT { left: S_X0, top: y, right: S_X1, bottom: y + h - 6 }
}

/// 内容区里从左到右排一排等宽控件
fn s_strip(i: usize, top: i32, w: i32, gap: i32, h: i32) -> RECT {
    let l = S_X0 + i as i32 * (w + gap);
    RECT { left: l, top, right: l + w, bottom: top + h }
}

fn s_sw_rect(i: usize) -> RECT {
    let y = S_TITLE_H + 226 + i as i32 * 62;
    RECT { left: S_X1 - 46, top: y, right: S_X1, bottom: y + 26 }
}

/// 开关整行都是热区（点哪儿都算）
fn s_sw_row(i: usize) -> RECT {
    let r = s_sw_rect(i);
    RECT { left: S_X0, top: r.top - 16, right: S_X1, bottom: r.bottom + 16 }
}

/// 指定 y 的开关（高级页自成一套布局，不能复用「交互」页的序号）
fn s_sw_rect_at(y: i32) -> RECT {
    RECT { left: S_X1 - 46, top: y, right: S_X1, bottom: y + 26 }
}

fn s_sw_row_at(y: i32) -> RECT {
    RECT { left: S_X0, top: y - 16, right: S_X1, bottom: y + 42 }
}

/// 高级页开关的 y
const S_ADV_SW_Y: i32 = S_TITLE_H + 76;

// ── GDI 小工具 ──

fn rgb(c: (u8, u8, u8)) -> COLORREF {
    COLORREF((c.0 as u32) | ((c.1 as u32) << 8) | ((c.2 as u32) << 16))
}

unsafe fn g_rrect(dc: HDC, r: RECT, rad: i32, fill: (u8, u8, u8), edge: Option<(u8, u8, u8)>, ew: i32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let br = CreateSolidBrush(rgb(fill));
    let pen = match edge {
        Some(e) => CreatePen(PS_SOLID, ew * SS, rgb(e)),
        None => CreatePen(PS_NULL, 0, rgb(fill)),
    };
    let ob = SelectObject(dc, HGDIOBJ(br.0));
    let op = SelectObject(dc, HGDIOBJ(pen.0));
    let _ = RoundRect(
        dc,
        sxv(r.left),
        sxv(r.top),
        sxv(r.right),
        sxv(r.bottom),
        rad * 2 * SS,
        rad * 2 * SS,
    );
    let _ = SelectObject(dc, ob);
    let _ = SelectObject(dc, op);
    let _ = DeleteObject(HGDIOBJ(br.0));
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

/// 只描边不填充（卡片外框：画在内容之上，不能盖住内容）
unsafe fn g_rrect_stroke(dc: HDC, r: RECT, rad: i32, edge: (u8, u8, u8), ew: i32) {
    let pen = CreatePen(PS_SOLID, ew * SS, rgb(edge));
    let nb = GetStockObject(NULL_BRUSH);
    let op = SelectObject(dc, HGDIOBJ(pen.0));
    let ob = SelectObject(dc, nb);
    let _ = RoundRect(
        dc,
        sxv(r.left),
        sxv(r.top),
        sxv(r.right),
        sxv(r.bottom),
        rad * 2 * SS,
        rad * 2 * SS,
    );
    let _ = SelectObject(dc, op);
    let _ = SelectObject(dc, ob);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

unsafe fn g_fill(dc: HDC, r: RECT, c: (u8, u8, u8)) {
    let br = CreateSolidBrush(rgb(c));
    let rr = RECT { left: sxv(r.left), top: sxv(r.top), right: sxv(r.right), bottom: sxv(r.bottom) };
    let _ = FillRect(dc, &rr, br);
    let _ = DeleteObject(HGDIOBJ(br.0));
}

/// 硬阴影：向下偏移 3px 的同形圆角块（无模糊半径）
unsafe fn g_shadow(dc: HDC, r: RECT, rad: i32, pal: &Pal) {
    let sh = mixc(pal.paper, pal.ink, 0.13);
    let sr = RECT { left: r.left, top: r.top + 3, right: r.right, bottom: r.bottom + 3 };
    g_rrect(dc, sr, rad, sh, None, 0);
}

unsafe fn g_text(
    dc: HDC,
    text: &str,
    r: RECT,
    color: (u8, u8, u8),
    font: HFONT,
    center: bool,
) {
    let _ = SelectObject(dc, HGDIOBJ(font.0));
    let _ = SetBkMode(dc, TRANSPARENT);
    let _ = SetTextColor(dc, rgb(color));
    let mut t: Vec<u16> = text.encode_utf16().collect();
    let mut rr = RECT { left: sxv(r.left), top: sxv(r.top), right: sxv(r.right), bottom: sxv(r.bottom) };
    let fmt = DT_SINGLELINE
        | DT_VCENTER
        | DT_NOPREFIX
        | if center { DT_CENTER } else { DT_LEFT | DT_END_ELLIPSIS };
    let _ = DrawTextW(dc, &mut t, &mut rr, fmt);
}

/// 分段控件 / 多选胶囊：选中 = 墨底白字。
/// `hv` = hover 强度 0..1 —— **必须在这里参与填充色的计算**：
/// 早先在调用处「先铺一层 hover 底色、再画胶囊」，胶囊的不透明填充会把它整个盖掉，
/// 表现就是「hover 完全没反应」。
unsafe fn g_chip(
    dc: HDC,
    pal: &Pal,
    r: RECT,
    on: bool,
    label: &str,
    font: HFONT,
    hv: f32,
) {
    if on {
        if hv > 0.02 {
            g_shadow(dc, r, 11, pal); // 选中态 hover：浮起来一点
        }
        g_rrect(dc, r, 11, pal.ink, Some(pal.ink), 2);
        g_text(dc, label, r, pal.paper, font, true);
    } else {
        let fill = mixc(pal.paper, pal.ink, 0.07 * hv);
        let edge = mixc(pal.edge, pal.ink, 0.45 * hv);
        g_rrect(dc, r, 11, fill, Some(edge), 2);
        g_text(dc, label, r, pal.ink, font, true);
    }
}

/// 整页淡入用的调色板：把每个颜色朝卡片色插值。
/// 这样「切页淡入」只要换一个 pal 就行，不用改动每一处绘制。
fn fade_pal(p: &Pal, a: f32) -> Pal {
    let a = a.clamp(0.0, 1.0);
    let f = |c: (u8, u8, u8)| mixc(p.paper, c, a);
    Pal {
        wall: f(p.wall),
        wall2: f(p.wall2),
        paper: p.paper,
        ink: f(p.ink),
        action: f(p.action),
        edge: f(p.edge),
    }
}

unsafe fn g_switch(dc: HDC, pal: &Pal, r: RECT, anim: f32) {
    let rad = (r.bottom - r.top) / 2;
    let a = anim.clamp(0.0, 1.0);
    let fill = mixc(pal.wall2, pal.ink, a);
    let edge = mixc(pal.edge, pal.ink, a);
    g_rrect(dc, r, rad, fill, Some(edge), 2);
    let d = (r.bottom - r.top) - 8;
    // 圆钮位置随 anim 滑动（不是瞬间跳）
    let x_on = (r.right - 4 - d) as f32;
    let x_off = (r.left + 4) as f32;
    let kx = (x_off + (x_on - x_off) * a).round() as i32;
    let kr = RECT { left: kx, top: r.top + 4, right: kx + d, bottom: r.top + 4 + d };
    g_rrect(dc, kr, d / 2, pal.paper, None, 0);
}

// ── 绘制 ──

const S_PAGES: [&str; 4] = ["外观", "交互", "应用", "高级"];

/// 控件当前的 hover 强度 0..1：新 hover 淡入、旧 hover 淡出。
/// 用「渐变状态量」而不是布尔 —— 否则 hover 是硬切，没有过渡感。
fn hov(st: &State, c: Ctl) -> f32 {
    if st.set_hover == c {
        st.set_hov_a
    } else if st.set_hover_prev == c {
        1.0 - st.set_hov_a
    } else {
        0.0
    }
}

/// 开关的动画值：0 = 关、1 = 开（中间态用于滑动过渡）
fn sw_of(st: &State, i: usize) -> f32 {
    st.sw_a[i.min(2)]
}

/// 把一页画进 hi 画布（逻辑坐标；缩放与超采样由 g_* 负责）
unsafe fn settings_draw(st: &mut State, dc: HDC) {
    let mut pal = pal_of(st.glass_liquid);
    // 切页淡入：整块调色板朝卡片色插值，不用改每一处绘制
    if st.set_page_a < 0.999 {
        pal = fade_pal(&pal, st.set_page_a);
    }
    let full = RECT { left: 0, top: 0, right: S_W, bottom: S_H };
    g_fill(dc, full, pal.paper);

    // 侧栏
    let nav_bg = RECT { left: 0, top: S_TITLE_H, right: S_NAV_W, bottom: S_H };
    g_fill(dc, nav_bg, pal.wall2);
    g_fill(dc, RECT { left: S_NAV_W - 2, top: S_TITLE_H, right: S_NAV_W, bottom: S_H },
        mixc(pal.paper, pal.ink, 0.14));

    // 标题栏
    g_text(dc, "设置", RECT { left: 22, top: 0, right: 220, bottom: S_TITLE_H }, pal.ink, st.sf_title, false);
    let ver = format!("v3 · {} 个应用", st.apps.len());
    g_text(dc, &ver, RECT { left: S_W - 300, top: 0, right: S_W - 62, bottom: S_TITLE_H },
        mixc(pal.paper, pal.ink, 0.55), st.sf_small, true);
    g_fill(dc, RECT { left: 0, top: S_TITLE_H - 2, right: S_W, bottom: S_TITLE_H },
        mixc(pal.paper, pal.ink, 0.14));
    // 关闭按钮
    let cr = RECT { left: S_W - 46, top: 12, right: S_W - 16, bottom: 42 };
    let chv = hov(st, Ctl::Close);
    if chv > 0.01 {
        g_rrect(dc, cr, 9, mixc(pal.paper, pal.ink, 0.08 * chv), None, 0);
    }
    g_text(dc, "×", cr, mixc(pal.ink, pal.paper, 0.0), st.sf_bold, true);

    // 侧栏导航
    for i in 0..4 {
        let r = s_nav_rect(i);
        // 文字要**内缩**：pill 从 left=14 起，文字贴着它的左边缘会显得「顶到边」
        let tr = RECT { left: r.left + 15, top: r.top, right: r.right - 8, bottom: r.bottom };
        if st.set_page == i {
            g_rrect(dc, r, 10, pal.ink, None, 0);
            g_text(dc, S_PAGES[i], tr, pal.paper, st.sf_bold, false);
        } else {
            let hv = hov(st, Ctl::Nav(i));
            if hv > 0.01 {
                g_rrect(dc, r, 10, mixc(pal.wall2, pal.ink, 0.09 * hv), None, 0);
            }
            g_text(dc, S_PAGES[i], tr, pal.ink, st.sf_nav, false);
        }
    }

    match st.set_page {
        0 => s_paint_look(st, dc, &pal),
        1 => s_paint_act(st, dc, &pal),
        2 => s_paint_apps(st, dc, &pal),
        _ => s_paint_adv(st, dc, &pal),
    }

    // 卡片外框**最后画**：它是描边不填充，既不会被内容盖住，也不会盖住内容
    // （卡片外那圈靠 alpha 裁掉，所以这里的直角矩形也不用倒角）
    g_rrect_stroke(dc, full, CARD_RAD, pal.ink, 2);
}

unsafe fn s_paint_look(st: &mut State, dc: HDC, pal: &Pal) {
    let y = S_TITLE_H + 26;
    g_text(dc, "配色", RECT { left: S_X0, top: y, right: S_X1, bottom: y + 22 },
        pal.ink, st.sf_bold, false);
    g_text(dc, "跟随系统 = 读 Windows 的应用主题（设置 → 个性化 → 颜色）",
        RECT { left: S_X0, top: y + 26, right: S_X1, bottom: y + 44 },
        mixc(pal.paper, pal.ink, 0.55), st.sf_small, false);
    let labels = ["亮色", "暗色", "跟随系统"];
    for i in 0..3 {
        let r = s_strip(i, y + 62, 108, 10, 38);
        let on = st.theme == i as i32;
        g_chip(dc, pal, r, on, labels[i], st.sf_body, hov(st, Ctl::Theme(i)));
    }
}

unsafe fn s_paint_act(st: &mut State, dc: HDC, pal: &Pal) {
    let y = S_TITLE_H + 26;
    g_text(dc, "呼出方式（可多选）", RECT { left: S_X0, top: y, right: S_X1, bottom: y + 22 },
        pal.ink, st.sf_bold, false);
    let labels = ["鼠标中键", "鼠标侧键", "长按右键"];
    let on_flags = [st.key_middle, st.key_side, st.key_rhold];
    for i in 0..3 {
        let r = s_strip(i, y + 62, 118, 10, 38);
        let on = on_flags[i];
        let lbl = if on { format!("✓ {}", labels[i]) } else { labels[i].to_string() };
        g_chip(dc, pal, r, on, &lbl, st.sf_body, hov(st, Ctl::Key(i)));
    }
    g_text(dc, "长按右键 = 按住约 0.4 秒后松开呼出；短按仍会补发一次正常右键",
        RECT { left: S_X0, top: y + 108, right: S_X1, bottom: y + 126 },
        mixc(pal.paper, pal.ink, 0.55), st.sf_small, false);

    let y2 = S_TITLE_H + 190;
    g_text(dc, "行为", RECT { left: S_X0, top: y2, right: S_X1, bottom: y2 + 22 },
        pal.ink, st.sf_bold, false);
    let rows: [(&str, &str); 2] = [
        ("常驻模式", "松开触发键后菜单保持显示；点击 / 回车确认，Esc / 点空白关闭"),
        ("监听", "关掉后不再挂钩子，呼出键完全失效（省电 / 临时让位）"),
    ];
    for i in 0..2 {
        let rr = s_sw_row(i);
        let hv = hov(st, Ctl::Sw(i));
        if hv > 0.01 {
            g_rrect(dc, rr, 10, mixc(pal.paper, pal.ink, 0.05 * hv), None, 0);
        }
        let ar = RECT { left: S_X0 + 8, top: rr.top + 14, right: S_X1 - 60, bottom: rr.top + 34 };
        g_text(dc, rows[i].0, ar, pal.ink, st.sf_body, false);
        g_text(dc, rows[i].1, RECT { left: S_X0 + 8, top: rr.top + 34, right: S_X1 - 60, bottom: rr.top + 50 },
            mixc(pal.paper, pal.ink, 0.55), st.sf_small, false);
        g_switch(dc, pal, s_sw_rect(i), sw_of(st, i));
    }
}

unsafe fn s_paint_apps(st: &mut State, dc: HDC, pal: &Pal) {
    let y = S_TITLE_H + 22;
    g_text(dc, "内环应用（点一行展开外环条目）",
        RECT { left: S_X0, top: y, right: S_X1, bottom: y + 22 }, pal.ink, st.sf_bold, false);

    let saved = SaveDC(dc);
    // ⚠️ 裁剪区也要走 sxv 缩放 —— 这个 DC 是 3× 的，混着写会把列表裁到只剩第一行
    let _ = IntersectClipRect(dc, sxv(0), sxv(S_LIST_TOP - 6), sxv(S_W), sxv(S_LIST_BOT));
    let scroll = st.set_scroll;
    let (rows, _) = s_rows(st);
    for row in &rows {
        let ry = S_LIST_TOP - scroll + row.y;
        if ry + row.h < S_LIST_TOP - 6 || ry > S_LIST_BOT {
            continue;
        }
        let head = row.kind == 0;
        let r = if head {
            s_row_rect(ry, row.h)
        } else {
            RECT { left: S_X0 + 26, top: ry, right: S_X1, bottom: ry + row.h - 4 }
        };
        let del = RECT { left: r.right - 38, top: r.top + 8, right: r.right - 6, bottom: r.top + 30 };
        let del_ctl = if head { Ctl::AppDel(row.ai) } else { Ctl::ItemDel(row.ai, row.ii) };

        if row.kind == 2 {
            let ahv = hov(st, Ctl::AddItem(row.ai));
            g_rrect(dc, r, 9, mixc(pal.paper, pal.ink, 0.06 * ahv),
                Some(mixc(mixc(pal.paper, pal.ink, 0.14), pal.edge, ahv)), 2);
            g_text(dc, "＋ 添加条目…", r, mixc(mixc(pal.paper, pal.ink, 0.55), pal.ink, ahv),
                st.sf_small, false);
            continue;
        }

        let bg = if head {
            mixc(pal.paper, pal.ink, 0.05 + 0.05 * hov(st, Ctl::AppHead(row.ai)))
        } else {
            pal.paper
        };
        g_rrect(dc, r, 9, bg, Some(mixc(pal.paper, pal.ink, 0.14)), 2);

        let (name, target) = if head {
            (&st.apps[row.ai].name, st.apps[row.ai].target.clone())
        } else {
            (
                &st.apps[row.ai].items[row.ii].name,
                st.apps[row.ai].items[row.ii].target.clone(),
            )
        };
        let tx = r.left + 12;
        let nm_r = RECT { left: tx, top: r.top, right: tx + 170, bottom: r.bottom };
        let tg_r = RECT { left: tx + 176, top: r.top, right: r.right - 52, bottom: r.bottom };
        g_text(dc, name, nm_r,
            if head { pal.ink } else { mixc(pal.paper, pal.ink, 0.75) },
            if head { st.sf_body } else { st.sf_small }, false);
        g_text(dc, &target, tg_r, mixc(pal.paper, pal.ink, 0.45), st.sf_small, false);

        if head {
            let n = st.apps[row.ai].items.len();
            let cr = RECT { left: r.right - 92, top: r.top + 10, right: r.right - 44, bottom: r.top + 32 };
            let pill = format!("{} 条", n);
            g_rrect(dc, cr, 11, pal.paper, Some(pal.edge), 2);
            g_text(dc, &pill, cr, mixc(pal.paper, pal.ink, 0.75), st.sf_small, true);
        }
        let dhv = hov(st, del_ctl);
        if dhv > 0.01 {
            g_rrect(dc, del, 8, mixc(pal.paper, pal.ink, dhv), None, 0);
        }
        g_text(dc, "×", del, mixc(mixc(pal.paper, pal.ink, 0.5), pal.paper, dhv), st.sf_small, true);
    }
    let _ = RestoreDC(dc, saved);

    // 底部：添加入口 + 备份提示（hover 在「描边态」和「实心态」之间渐变）
    let ba = RECT { left: S_X0, top: S_H - 64, right: S_X0 + 190, bottom: S_H - 28 };
    let bhv = hov(st, Ctl::AddApp);
    if bhv > 0.02 {
        g_shadow(dc, ba, 10, pal);
    }
    g_rrect(dc, ba, 10, mixc(pal.paper, pal.ink, bhv), Some(mixc(pal.edge, pal.ink, bhv)), 2);
    g_text(dc, "＋ 添加应用…", ba, mixc(pal.ink, pal.paper, bhv), st.sf_body, true);
    g_text(dc, "每次改动都会先把旧配置备份成 settings.ini.bak",
        RECT { left: S_X0 + 206, top: S_H - 64, right: S_X1, bottom: S_H - 28 },
        mixc(pal.paper, pal.ink, 0.45), st.sf_small, false);
}

unsafe fn s_paint_adv(st: &mut State, dc: HDC, pal: &Pal) {
    let y = S_TITLE_H + 26;
    g_text(dc, "系统", RECT { left: S_X0, top: y, right: S_X1, bottom: y + 22 },
        pal.ink, st.sf_bold, false);
    let rr = s_sw_row_at(S_ADV_SW_Y);
    let ahv = hov(st, Ctl::Sw(2));
    if ahv > 0.01 {
        g_rrect(dc, rr, 10, mixc(pal.paper, pal.ink, 0.05 * ahv), None, 0);
    }
    g_text(dc, "开机自启", RECT { left: S_X0 + 8, top: rr.top + 14, right: S_X1 - 60, bottom: rr.top + 34 },
        pal.ink, st.sf_body, false);
    g_text(dc, "写 HKCU\\...\\Run，不落配置文件",
        RECT { left: S_X0 + 8, top: rr.top + 34, right: S_X1 - 60, bottom: rr.top + 50 },
        mixc(pal.paper, pal.ink, 0.55), st.sf_small, false);
    let ad_on = autostart_get();
    g_switch(dc, pal, s_sw_rect_at(S_ADV_SW_Y), if ad_on { 1.0 } else { 0.0 });

    let y2 = S_TITLE_H + 168;
    g_text(dc, "关于", RECT { left: S_X0, top: y2, right: S_X1, bottom: y2 + 22 },
        pal.ink, st.sf_bold, false);
    let sz = std::fs::metadata(exe_path()).map(|m| m.len()).unwrap_or(0);
    let items = st.apps.iter().map(|a| a.items.len()).sum::<usize>();
    let lines = [
        format!("径向启动器 Runtime v3 · 原生 Win32 + 软件光栅，零 UI 框架"),
        format!("单文件 {} KB · {} 个应用 / 共 {} 条外环条目", sz / 1024, st.apps.len(), items),
        format!("配置存于 settings.ini（页面是唯一入口，不必手改）"),
    ];
    for (i, l) in lines.iter().enumerate() {
        let t = y2 + 30 + i as i32 * 22;
        g_text(dc, l, RECT { left: S_X0, top: t, right: S_X1, bottom: t + 20 },
            mixc(pal.paper, pal.ink, 0.65), st.sf_small, false);
    }
}

// ── 命中 ──

fn s_hit(st: &State, x: i32, y: i32) -> Ctl {
    if x >= S_W - 50 && y < S_TITLE_H && x >= S_W - 46 - 6 {
        return Ctl::Close;
    }
    for i in 0..4 {
        if in_rect(s_nav_rect(i), x, y) {
            return Ctl::Nav(i);
        }
    }
    match st.set_page {
        0 => {
            let top = S_TITLE_H + 26 + 62;
            for i in 0..3 {
                if in_rect(s_strip(i, top, 108, 10, 38), x, y) {
                    return Ctl::Theme(i);
                }
            }
        }
        1 => {
            let top = S_TITLE_H + 26 + 62;
            for i in 0..3 {
                if in_rect(s_strip(i, top, 118, 10, 38), x, y) {
                    return Ctl::Key(i);
                }
            }
            for i in 0..2 {
                if in_rect(s_sw_row(i), x, y) {
                    return Ctl::Sw(i);
                }
            }
        }
        2 => {
            let ba = RECT { left: S_X0, top: S_H - 64, right: S_X0 + 190, bottom: S_H - 28 };
            if in_rect(ba, x, y) {
                return Ctl::AddApp;
            }
            if y < S_LIST_TOP - 6 || y > S_LIST_BOT {
                return Ctl::None;
            }
            let scroll = st.set_scroll;
            let (rows, _) = s_rows(st);
            for row in &rows {
                let ry = S_LIST_TOP - scroll + row.y;
                let r = if row.kind == 0 {
                    s_row_rect(ry, row.h)
                } else {
                    RECT { left: S_X0 + 26, top: ry, right: S_X1, bottom: ry + row.h - 4 }
                };
                if !in_rect(r, x, y) {
                    continue;
                }
                let del = RECT { left: r.right - 38, top: r.top + 8, right: r.right - 6, bottom: r.top + 30 };
                return match row.kind {
                    0 => {
                        if in_rect(del, x, y) {
                            Ctl::AppDel(row.ai)
                        } else {
                            Ctl::AppHead(row.ai)
                        }
                    }
                    1 => {
                        if in_rect(del, x, y) {
                            Ctl::ItemDel(row.ai, row.ii)
                        } else {
                            Ctl::None
                        }
                    }
                    _ => Ctl::AddItem(row.ai),
                };
            }
        }
        _ => {
            if in_rect(s_sw_row_at(S_ADV_SW_Y), x, y) {
                return Ctl::Sw(2); // 2 = 开机自启
            }
        }
    }
    Ctl::None
}

fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

// ── 动作 ──

unsafe fn open_settings(st: &mut State) {
    let h = SET_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    let hwnd = HWND(h as *mut c_void);
    let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    st.set_x = (sw - S_WW) / 2;
    st.set_y = (sh - S_HH) / 2;
    let _ = SetWindowPos(hwnd, None, st.set_x, st.set_y, S_WW, S_HH, SWP_NOZORDER);
    st.set_hover = Ctl::None;
    setup_present(st);
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = SetForegroundWindow(hwnd);
}

unsafe fn s_click(st: &mut State, hwnd: HWND, x: i32, y: i32) {
    let c = s_hit(st, x, y);
    match c {
        Ctl::None => {}
        Ctl::Close => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return;
        }
        Ctl::Nav(i) => {
            if st.set_page != i {
                st.set_page = i;
                st.set_page_a = 0.0; // 切页淡入
                st.set_hover = Ctl::None;
                st.set_hover_prev = Ctl::None;
            }
        }
        Ctl::Theme(i) => {
            st.theme = i as i32;
            apply_theme(st);
            apply_key_opts(st);
            save_settings(st);
        }
        Ctl::Key(i) => {
            match i {
                0 => st.key_middle = !st.key_middle,
                1 => st.key_side = !st.key_side,
                _ => st.key_rhold = !st.key_rhold,
            }
            apply_key_opts(st);
            save_settings(st);
        }
        Ctl::Sw(i) => match i {
            0 => {
                st.persistent = !st.persistent;
                save_settings(st);
            }
            1 => {
                let on = !st.running;
                set_running(st, on);
                save_settings(st);
            }
            _ => {
                let on = !autostart_get();
                autostart_set(on);
                let _ = on;
            }
        },
        Ctl::AppHead(ai) => {
            st.set_expanded = if st.set_expanded == ai { usize::MAX } else { ai };
            st.set_scroll = st.set_scroll.min(s_scroll_max(st));
        }
        Ctl::AppDel(ai) => {
            if ai < st.apps.len() {
                st.apps.remove(ai);
                if st.set_expanded == ai {
                    st.set_expanded = usize::MAX;
                }
                st.active_app = None;
                save_settings(st);
                load_icons(st);
            }
        }
        Ctl::ItemDel(ai, ii) => {
            if ai < st.apps.len() && ii < st.apps[ai].items.len() {
                st.apps[ai].items.remove(ii);
                save_settings(st);
            }
        }
        Ctl::AddApp => {
            if let Some(p) = pick_file() {
                let name = unique_app_name(st, &name_from_path(&p));
                st.apps.push(AppEntry {
                    name,
                    target: p,
                    icon: None,
                    items: Vec::new(),
                    grad: grad_for(""),
                });
                if st.apps.len() > MAX_APPS {
                    st.apps.truncate(MAX_APPS);
                }
                save_settings(st);
                load_icons(st);
            }
        }
        Ctl::AddItem(ai) => {
            if ai < st.apps.len() {
                if let Some(p) = pick_file() {
                    let name = unique_item_name(st, ai, &name_from_path(&p));
                    st.apps[ai]
                        .items
                        .push(ItemEntry { name, target: p, color: None, icon: None });
                    if st.apps[ai].items.len() > MAX_ITEMS {
                        st.apps[ai].items.truncate(MAX_ITEMS);
                    }
                    save_settings(st);
                }
            }
        }
    }
    // ⚠️ 分层窗口的 WM_PAINT 是空实现，`InvalidateRect` 不会重绘任何东西 ——
    // 之前点完按钮「等一秒才有反应」，就是状态变了但没人重画，直到鼠标一动
    // 触发了 hover 重绘。这里必须显式重渲染。
    setup_present(st);
    setup_kick(st, hwnd);
}

fn name_from_path(p: &str) -> String {
    let stem = std::path::Path::new(p)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string());
    let n = clamp_name(&stem);
    if n.is_empty() {
        "新项目".to_string()
    } else {
        n
    }
}

fn unique_app_name(st: &State, base: &str) -> String {
    let mut n = base.to_string();
    let mut i = 2;
    while st.apps.iter().any(|a| a.name == n) {
        n = format!("{} {}", base, i);
        i += 1;
    }
    n
}

fn unique_item_name(st: &State, ai: usize, base: &str) -> String {
    let mut n = base.to_string();
    let mut i = 2;
    while st.apps[ai].items.iter().any(|it| it.name == n) {
        n = format!("{} {}", base, i);
        i += 1;
    }
    n
}

/// 关窗时把超采样缓冲还回去：hi 那张是 SS² 倍大（3× 时 ~18MB），
/// 只有设置页开着时才需要它，常驻挂着纯属浪费
unsafe fn setup_free(st: &mut State) {
    if !st.s_hi_dc.0.is_null() {
        let _ = DeleteDC(st.s_hi_dc);
        let _ = DeleteObject(HGDIOBJ(st.s_hi_bmp.0));
        st.s_hi_dc = HDC(ptr::null_mut());
        st.s_hi_bmp = HBITMAP(ptr::null_mut());
        st.s_hi_bits = ptr::null_mut();
    }
    if !st.s_lo_dc.0.is_null() {
        let _ = DeleteDC(st.s_lo_dc);
        let _ = DeleteObject(HGDIOBJ(st.s_lo_bmp.0));
        st.s_lo_dc = HDC(ptr::null_mut());
        st.s_lo_bmp = HBITMAP(ptr::null_mut());
        st.s_lo_bits = ptr::null_mut();
    }
}

/// 设置页动画节拍
const TIMER_SETUP_FRAME: usize = 2;

/// 启动设置页动画（hover 淡入淡出 / 开关滑动 / 切页淡入）
unsafe fn setup_kick(st: &mut State, hwnd: HWND) {
    st.set_last = Instant::now();
    let _ = SetTimer(hwnd, TIMER_SETUP_FRAME, 16, None);
}

unsafe fn setup_tick(st: &mut State, hwnd: HWND) {
    let now = Instant::now();
    let dt = (now - st.set_last).as_secs_f32().clamp(0.001, 0.10);
    st.set_last = now;
    // 帧率无关的指数收敛；rate 取 26 ≈ 120ms 到位，手感接近 --spring
    let k = 1.0 - (-26.0 * dt).exp();
    let mut busy = false;

    if st.set_hov_a < 1.0 {
        st.set_hov_a += (1.0 - st.set_hov_a) * k;
        if st.set_hov_a > 0.99 {
            st.set_hov_a = 1.0;
        } else {
            busy = true;
        }
    }
    if st.set_page_a < 1.0 {
        st.set_page_a += (1.0 - st.set_page_a) * k;
        if st.set_page_a > 0.99 {
            st.set_page_a = 1.0;
        } else {
            busy = true;
        }
    }
    let mut targets = [
        if st.persistent { 1.0f32 } else { 0.0 },
        if st.running { 1.0 } else { 0.0 },
        st.sw_a[2],
    ];
    if st.set_page == 3 {
        targets[2] = if autostart_get() { 1.0 } else { 0.0 };
    }
    for i in 0..3 {
        let d = targets[i] - st.sw_a[i];
        if d.abs() > 0.003 {
            st.sw_a[i] += d * k;
            busy = true;
        } else {
            st.sw_a[i] = targets[i];
        }
    }

    if !busy {
        st.set_hover_prev = Ctl::None;
        let _ = KillTimer(hwnd, TIMER_SETUP_FRAME);
    }
    setup_present(st);
}

/// 设置窗口过程
unsafe extern "system" fn setup_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // 分层窗口自己渲染上屏，WM_PAINT 无事可做
    if msg == WM_PAINT {
        let mut ps = PAINTSTRUCT::default();
        let _ = BeginPaint(hwnd, &mut ps);
        let _ = EndPaint(hwnd, &ps);
        return LRESULT(0);
    }
    // 鼠标坐标是**窗口**坐标（卡片外还有一圈 S_M 的投影留白），
    // 布局用的是卡片内的逻辑坐标，所以先减掉 S_M
    let lx = || ((l.0 & 0xFFFF) as i16 as i32) - S_M;
    let ly = || (((l.0 >> 16) & 0xFFFF) as i16 as i32) - S_M;
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => {
            let x = (l.0 & 0xFFFF) as i16 as i32;
            let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
            // 标题条拖动（要排除关闭按钮那一段）
            if x >= S_M && x < S_M + S_W - 56 && y >= S_M && y < S_M + S_TITLE_H {
                return LRESULT(HTCAPTION as isize);
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_MOUSEMOVE => {
            let st = state_of(hwnd);
            // 注册一次 LEAVE 通知，否则指针移出窗口后 hover 会「粘」在最后一个控件上
            if !st.set_tracking {
                use windows::Win32::UI::Input::KeyboardAndMouse::{
                    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
                };
                st.set_tracking = true;
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            let c = s_hit(st, lx(), ly());
            if c != st.set_hover {
                // 旧的淡出、新的淡入：靠 set_hov_a 这一个进度量驱动
                st.set_hover_prev = st.set_hover;
                st.set_hover = c;
                st.set_hov_a = 0.0;
                setup_kick(st, hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            let st = state_of(hwnd);
            st.set_tracking = false;
            if st.set_hover != Ctl::None {
                st.set_hover_prev = st.set_hover;
                st.set_hover = Ctl::None;
                st.set_hov_a = 0.0;
                setup_kick(st, hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let st = state_of(hwnd);
            s_click(st, hwnd, lx(), ly());
            LRESULT(0)
        }
        WM_TIMER => {
            let st = state_of(hwnd);
            if w.0 as usize == TIMER_SETUP_FRAME {
                setup_tick(st, hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let st = state_of(hwnd);
            if st.set_page == 2 {
                let d = ((w.0 >> 16) & 0xFFFF) as i16 as i32 / 120;
                st.set_scroll = (st.set_scroll - d * 42).clamp(0, s_scroll_max(st));
                setup_present(st);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if w.0 as u32 == 0x1B {
                let _ = ShowWindow(hwnd, SW_HIDE);
                setup_free(state_of(hwnd));
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = ShowWindow(hwnd, SW_HIDE);
            setup_free(state_of(hwnd));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

// ── 持久化 ──

unsafe fn save_settings(st: &State) {
    // 无 git —— 破坏性写之前先留一份可回滚的副本
    let bak = format!("{}.bak", st.ini_path);
    let _ = std::fs::copy(&st.ini_path, &bak);
    let b = |v: bool| if v { 1 } else { 0 };
    let mut out = String::new();
    out.push_str("[设置]\r\n");
    out.push_str(&format!("dark={}\r\n", st.theme));
    out.push_str(&format!("persistent={}\r\n", b(st.persistent)));
    out.push_str(&format!("listening={}\r\n", b(st.running)));
    out.push_str(&format!("key_middle={}\r\n", b(st.key_middle)));
    out.push_str(&format!("key_side={}\r\n", b(st.key_side)));
    out.push_str(&format!("key_rhold={}\r\n", b(st.key_rhold)));
    out.push_str("\r\n[apps]\r\n");
    for a in &st.apps {
        out.push_str(&format!("{}={}\r\n", a.name, a.target));
    }
    for a in &st.apps {
        if a.items.is_empty() {
            continue;
        }
        out.push_str(&format!("\r\n[app.{}]\r\n", a.name));
        for it in &a.items {
            match it.color {
                Some(c) => out.push_str(&format!("{}={}|{}\r\n", it.name, it.target, c)),
                None => out.push_str(&format!("{}={}\r\n", it.name, it.target)),
            }
        }
    }
    let _ = std::fs::write(&st.ini_path, out);
}

fn apply_key_opts(st: &State) {
    OPT_MIDDLE.store(st.key_middle, Ordering::SeqCst);
    OPT_SIDE.store(st.key_side, Ordering::SeqCst);
    OPT_RHOLD.store(st.key_rhold, Ordering::SeqCst);
}

/// 跟随系统：读 Windows 的「应用模式」是否用浅色（0 = 深色）
fn system_is_dark() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let sub = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let val = wide("AppsUseLightTheme");
    let mut v: u32 = 1;
    let mut sz: u32 = 4;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut sz),
        )
    };
    if r.0 == 0 {
        v == 0
    } else {
        false
    }
}

unsafe fn apply_theme(st: &mut State) {
    st.glass_liquid = match st.theme {
        0 => false,
        1 => true,
        _ => system_is_dark(),
    };
}

fn autostart_key() -> (Vec<u16>, Vec<u16>) {
    (
        wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
        wide("RadialMenu"),
    )
}

fn autostart_get() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (sub, name) = autostart_key();
    let mut buf = [0u16; 1024];
    let mut sz: u32 = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut sz),
        )
    };
    r.0 == 0
}

fn autostart_set(on: bool) {
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY,
            HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        };
    let (sub, name) = autostart_key();
    unsafe {
        let mut hk = HKEY::default();
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hk,
            None,
        );
        if r.0 != 0 {
            return;
        }
        if on {
            let d: Vec<u16> = wide(&format!("\"{}\"", exe_path()));
            let bytes: &[u8] = std::slice::from_raw_parts(d.as_ptr() as *const u8, d.len() * 2);
            let _ = RegSetValueExW(hk, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(hk, PCWSTR(name.as_ptr()));
        }
        let _ = RegCloseKey(hk);
    }
}

fn exe_path() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

/// 选文件（原生对话框：中文路径输入交给系统，自己不造轮子）
unsafe fn pick_file() -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OPENFILENAMEW, OFN_DONTADDTORECENT, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST,
    };
    let filter: Vec<u16> = "程序 / 快捷方式\0*.exe;*.lnk;*.bat;*.cmd\0所有文件\0*.*\0\0"
        .encode_utf16()
        .collect();
    let defext: Vec<u16> = "lnk\0".encode_utf16().collect();
    let mut buf = [0u16; 1024];
    let mut ofn = OPENFILENAMEW::default();
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = PCWSTR(filter.as_ptr());
    ofn.lpstrFile = windows::core::PWSTR(buf.as_mut_ptr());
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrDefExt = PCWSTR(defext.as_ptr());
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT;
    if !GetOpenFileNameW(&mut ofn).as_bool() {
        return None;
    }
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..end]))
}

// ── 超采样渲染管线 ──

unsafe fn ensure_setup_surfaces(st: &mut State) {
    if st.s_hi_dc.0.is_null() {
        let (b, d, m) = make_dib(S_WW * SS, S_HH * SS);
        st.s_hi_bits = b;
        st.s_hi_dc = d;
        st.s_hi_bmp = m;
    }
    if st.s_lo_dc.0.is_null() {
        let (b, d, m) = make_dib(S_WW, S_HH);
        st.s_lo_bits = b;
        st.s_lo_dc = d;
        st.s_lo_bmp = m;
    }
}

/// 把当前页渲染进 lo 缓冲（**未预乘**）：hi 上 SS× 画 → SS×SS 盒式降采样 → 投影 + alpha
unsafe fn setup_render(st: &mut State) {
    let t0 = Instant::now();
    ensure_setup_surfaces(st);
    if st.s_hi_dc.0.is_null() || st.s_lo_dc.0.is_null() || st.s_hi_bits.is_null() {
        return;
    }
    let pal = pal_of(st.glass_liquid);
    let hi_w = (S_WW * SS) as usize;
    let hi_h = (S_HH * SS) as usize;
    let lo_w = S_WW as usize;
    let lo_h = S_HH as usize;

    // 1) hi 画布整体铺卡片色。**不能留黑** —— 降采样时边缘会把黑混进来，
    //    卡片边上就会出现一圈脏灰。
    g_fill(
        st.s_hi_dc,
        RECT { left: -S_M, top: -S_M, right: S_W + S_M, bottom: S_H + S_M },
        pal.paper,
    );
    // 2) 画整页
    settings_draw(st, st.s_hi_dc);

    // 3) SS×SS 盒式降采样（整数倍，等价于 SS² 级覆盖率）
    let hi = std::slice::from_raw_parts(st.s_hi_bits, hi_w * hi_h * 4);
    let lo = std::slice::from_raw_parts_mut(st.s_lo_bits, lo_w * lo_h * 4);
    let n = (SS * SS) as u32;
    for y in 0..lo_h {
        for x in 0..lo_w {
            let (mut a0, mut a1, mut a2) = (0u32, 0u32, 0u32);
            for dy in 0..SS as usize {
                let row = ((y * SS as usize + dy) * hi_w + x * SS as usize) * 4;
                for dx in 0..SS as usize {
                    let o = row + dx * 4;
                    a0 += hi[o] as u32;
                    a1 += hi[o + 1] as u32;
                    a2 += hi[o + 2] as u32;
                }
            }
            let o = (y * lo_w + x) * 4;
            lo[o] = (a0 / n) as u8;
            lo[o + 1] = (a1 / n) as u8;
            lo[o + 2] = (a2 / n) as u8;
        }
    }

    // 4) alpha：GDI 不写 alpha 通道，所以形状的透明区在这里**解析式**补上。
    //    卡片 = 圆角矩形（外扩 1px，让 2px 描边的外半边不被切掉）；
    //    卡片之外按「向下偏移 + 收紧模糊」铺一层投影。
    let hw = S_W as f32 / 2.0;
    let hh = S_H as f32 / 2.0;
    let ccx = (S_M + S_W / 2) as f32;
    let ccy = (S_M + S_H / 2) as f32;
    // 投影：向下偏移 + **同时向内收**，否则四周会糊出一圈对称光晕（就是「大黑边」）。
    // 收进去之后：下方只有 4px 实带 + 渐变，左右各剩一点点，上方几乎没有。
    let (sh_dy, sh_blur, sh_a) = (8.0f32, 14.0f32, 0.26f32);
    let shc = [SHADOW.2 as f32, SHADOW.1 as f32, SHADOW.0 as f32]; // 缓冲是 BGRA
    for y in 0..lo_h {
        for x in 0..lo_w {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            let o = (y * lo_w + x) * 4;
            let ac = cov1(-sdf_rounded_rect(fx - ccx, fy - ccy, hw + 1.0, hh + 1.0, CARD_RAD as f32));
            if ac >= 0.999 {
                lo[o + 3] = 255;
                continue;
            }
            let ds = -sdf_rounded_rect(
                fx - ccx,
                fy - ccy - sh_dy,
                (hw - 6.0).max(2.0),
                (hh - 4.0).max(2.0),
                CARD_RAD as f32,
            );
            let ash = if ds > 0.0 {
                let t = 1.0 - (ds / sh_blur).clamp(0.0, 1.0);
                t * t * sh_a
            } else {
                sh_a
            };
            let a = (ac + (1.0 - ac) * ash).clamp(0.0, 1.0);
            if a < 0.002 {
                lo[o + 3] = 0;
                continue;
            }
            let inv = 1.0 / a;
            for k in 0..3 {
                let c = lo[o + k] as f32;
                lo[o + k] = ((c * ac + shc[k] * (1.0 - ac) * ash) * inv).clamp(0.0, 255.0) as u8;
            }
            lo[o + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    if std::env::var("RL_SETDBG").is_ok() {
        eprintln!("[setdbg] setup_render {:.1}ms", t0.elapsed().as_secs_f64() * 1000.0);
    }
}

/// 渲染 + 上屏（分层窗口：ULW 要预乘 alpha）
unsafe fn setup_present(st: &mut State) {
    let h = SET_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    setup_render(st);
    if st.s_lo_bits.is_null() {
        return;
    }
    {
        let buf = std::slice::from_raw_parts_mut(st.s_lo_bits, (S_WW * S_HH * 4) as usize);
        premultiply(buf);
    }
    let dst = POINT { x: st.set_x, y: st.set_y };
    let src = POINT::default();
    let size = SIZE { cx: S_WW, cy: S_HH };
    let blend = BLENDFUNCTION {
        BlendOp: 0u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: 1u8,
    };
    let _ = UpdateLayeredWindow(
        HWND(h as *mut c_void),
        HDC(ptr::null_mut()),
        Some(&dst),
        Some(&size),
        st.s_lo_dc,
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
}

/// 设置页的离屏核对：把四个页面渲染成 BMP（配 bmp2png.py 看图，不用真机点）
unsafe fn settings_dump(dir: &str, st: &mut State) {
    for page in 0..4usize {
        st.set_page = page;
        st.set_hover = Ctl::None;
        st.set_scroll = 0;
        st.set_expanded = if page == 2 { 0 } else { usize::MAX };
        setup_render(st);
        let (dr, dtot) = s_rows(st);
        println!(
            "[setup] page={} apps={} rows={} total={} expanded={}",
            page,
            st.apps.len(),
            dr.len(),
            dtot,
            st.set_expanded
        );
        // 合成到中灰底上再看 —— BMP 不带 alpha，直接存的话圆角外和投影区
        // 显示的是原始 RGB，完全看不出投影浓淡和透明边界
        let bg = (205u8, 205u8, 208u8);
        let mut px = std::slice::from_raw_parts(st.s_lo_bits, (S_WW * S_HH * 4) as usize).to_vec();
        for p in px.chunks_mut(4) {
            let a = p[3] as f32 / 255.0;
            let ia = 1.0 - a;
            p[0] = (p[0] as f32 * a + bg.2 as f32 * ia).clamp(0.0, 255.0) as u8; // B
            p[1] = (p[1] as f32 * a + bg.1 as f32 * ia).clamp(0.0, 255.0) as u8; // G
            p[2] = (p[2] as f32 * a + bg.0 as f32 * ia).clamp(0.0, 255.0) as u8; // R
            p[3] = 255;
        }
        let p = format!("{}\\setup_{}.bmp", dir, page);
        let ok = save_bmp(&p, &px, S_WW, S_HH);
        println!("[setup] page={} {}", page, if ok { "OK" } else { "FAIL" });
    }
    st.set_page = 0;
    st.set_expanded = usize::MAX;
}

// ───────────────────── 入口 ─────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 2 && args[1] == "--quit" {
        unsafe { kill_other_instances() };
        println!("[*] 已发送退出指令");
        return;
    }
    if args.len() >= 2 && args[1] == "--dump" {
        let dir = args.get(2).cloned().unwrap_or_else(|| ".".into());
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
            dump_all(&dir)
        };
        return;
    }
    // 诊断：列出每个应用目标的图标提取结果（原生尺寸 / 是否有图标）
    if args.len() >= 2 && args[1] == "--icons" {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
            let mut st = State::new();
            st.ini_path = format!("{}\\settings.ini", exe_dir());
            load_ini(&mut st);
            for a in &st.apps {
                eprintln!(
                    "[app] {:<12} target={:<40} icon={}",
                    a.name,
                    a.target,
                    match &a.icon {
                        Some(t) => {
                            let base = t.levels.first().map(|l| l.0).unwrap_or(0);
                            let mips: Vec<String> =
                                t.levels.iter().skip(1).map(|l| l.0.to_string()).collect();
                            format!("{}px-texture + mip[{}]", base, mips.join(","))
                        }
                        None => "none(字母磁贴)".to_string(),
                    }
                );
            }
        }
        return;
    }

    // 诊断：把设置窗口的四个页面离屏渲染成 BMP（配 bmp2png.py 核对排版）
    if args.len() >= 2 && args[1] == "--settings-dump" {
        let dir = args.get(2).cloned().unwrap_or_else(|| ".".into());
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
            let mut st = State::new();
            st.ini_path = format!("{}\\settings.ini", dir);
            load_ini(&mut st);
            read_persistent(&mut st);
            init_fonts(&mut st);
            settings_dump(&dir, &mut st);
        }
        return;
    }

    // COM 初始化必须尽早：IShellItemImageFactory（高清图标）依赖 STA，
    // 而 load_ini() 里就会去解码图标 —— 漏了这句会让主流程拿不到高清图标。
    // 注意：拖放（RegisterDragDrop）需要 OleInitialize 的 OLE 服务层，
    // 只 CoInitializeEx 的线程上注册会返回 E_OUTOFMEMORY —— 两个都要。
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
        let hr = windows::Win32::System::Ole::OleInitialize(None);
        if hr.is_err() {
            drag_log(&format!("OleInitialize {:?}", hr));
        }
    }

    // ── 输入卡顿诊断：看门狗线程 + 日志（exe 目录 radial_log.txt）──
    // 排查「闲置后鼠标卡 30 秒」：HOOK_SLOW=我们的钩子回调慢；
    // WATCHDOG_GAP=我们整个进程被冻结（待机/换页）；两者都干净=卡在别处。
    {
        // 控制台 QuickEdit 是一个免费的假死源：用户（或误触）在控制台窗口里
        // 选中一段文本，所有 println! 的线程都会被挂起直到取消选中 —— 先禁掉。
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
        const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
        extern "system" {
            #[link_name = "GetStdHandle"]
            fn get_std_handle(which: u32) -> isize;
            #[link_name = "GetConsoleMode"]
            fn get_console_mode(h: isize, mode: *mut u32) -> i32;
            #[link_name = "SetConsoleMode"]
            fn set_console_mode(h: isize, mode: u32) -> i32;
        }
        unsafe {
            let h = get_std_handle(STD_OUTPUT_HANDLE);
            let mut mode = 0u32;
            if get_console_mode(h, &mut mode) != 0 {
                set_console_mode(h, (mode & !ENABLE_QUICK_EDIT_MODE) | ENABLE_EXTENDED_FLAGS);
            }
        }
        std::thread::spawn(|| {
            use std::time::Duration;
            let path = format!("{}\\radial_log.txt", exe_dir());
            // 超 8MB 重建，防止无限膨胀
            if let Ok(m) = std::fs::metadata(&path) {
                if m.len() > 8_000_000 {
                    let _ = std::fs::remove_file(&path);
                }
            }
            let build = std::fs::metadata(std::env::current_exe().unwrap_or_default())
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            diag_line(&path, &format!(
                "=== 启动 exe_build_unix={} ===", build));
            diag_line(&path,
                "--- 日志怎么读（出问题时按这个顺序看）---");
            diag_line(&path,
                "  SNAPSHOT 每 5 秒一条：列出**所有跨线程开关的当前值**。");
            diag_line(&path,
                "    · 「全清」= 正常。若某个开关该清没清（MENU_OPEN 尤其），就是它卡住了。");
            diag_line(&path,
                "    · last_mouse=N ms ago = 距上次鼠标事件的毫秒。持续涨 = 输入进不来。");
            diag_line(&path,
                "  MOUSE_IDLE 每 5 秒以上无鼠标记一条（比 HOOK_GAP 细 6 倍，用于定位失灵**起止时刻**）。");
            diag_line(&path,
                "  HOOK_SLOW/KEY_SLOW = 钩子回调耗时。成簇出现 = 事件在别处堵住后涌入。");
            diag_line(&path,
                "  POST_DROP = PostMessage 失败（UI 线程队列满/窗口已销毁 → 菜单不响应）。");
            diag_line(&path,
                "  HOOK_LOST = 钩子线程退场，程序会自行重启（已自愈）。");
            diag_line(&path,
                "  WATCHDOG_GAP = 本进程整体被冻结（待机/换页）。");
            diag_line(&path, "--- 开始 ---");
            let mut consumed: u64 = 0;
            let mut lines: u32 = 0;
            let mut hb_ticks: u32 = 0;
            loop {
                let t0 = std::time::Instant::now();
                std::thread::sleep(Duration::from_secs(1));
                let dt = t0.elapsed();
                if dt > Duration::from_secs(3) {
                    diag_line(&path, &format!(
                        "WATCHDOG_GAP {}ms —— 本进程整线程被冻结（待机/换页/系统级停顿）",
                        dt.as_millis()));
                    lines += 1;
                }
                // ── 钩子存活检查 ──
                // 钩子线程一旦非正常退场（GetMessage 出错等），内核里会残留一个
                // 挂在死线程上的钩子：系统继续往那条链投事件，但没人能处理 →
                // 症状是「只有当前窗口能点，任务栏和其它窗口全点不动，菜单也弹不出来」。
                // 这状态**无法在进程内修复**（死线程摘不到自己的钩子），唯一出路是重启。
                // 所以这里每 3 秒查一次：钩子该在却不在了 → 摘干净 + 重启自己。
                {
                    let listening = ST_RUNNING_FLAG.load(Ordering::Relaxed);
                    let tid = HOOK_TID.load(Ordering::Relaxed);
                    let mut dead = false;
                    if listening && tid != 0 {
                        use windows::Win32::Foundation::CloseHandle;
                        use windows::Win32::System::Threading::{
                            OpenThread, WaitForSingleObject, THREAD_ALL_ACCESS,
                        };
                        unsafe {
                            match OpenThread(THREAD_ALL_ACCESS, false, tid) {
                                Ok(th) => {
                                    // WAIT_OBJECT_0 = 0 → 线程已退场
                                    let w = WaitForSingleObject(th, 0);
                                    let _ = CloseHandle(th);
                                    dead = w.0 == 0;
                                }
                                Err(_) => dead = true, // 线程已销毁
                            }
                        }
                    }
                    if dead {
                        // 走到这里马上就要 exit，所以不需要「只打一次」的标志位
                        diag_line(&path,
                            "HOOK_LOST 钩子线程已退场，内核里可能残留死钩子 → 立即重启自愈");
                        diag_push(DK_HOOK_LOST, 0);
                        // 先摘（可能失败但无害），再重启
                        unsafe {
                            let h = HOOK_HANDLE.swap(0, Ordering::SeqCst);
                            if h != 0 {
                                let _ = UnhookWindowsHookEx(HHOOK(h as *mut c_void));
                            }
                            let k = KEY_HOOK_HANDLE.swap(0, Ordering::SeqCst);
                            if k != 0 {
                                let _ = UnhookWindowsHookEx(HHOOK(k as *mut c_void));
                            }
                        }
                        std::thread::sleep(Duration::from_millis(200));
                        std::process::exit(0x0000_0009); // 看门狗自愈码，父进程/脚本可识别
                    }
                }
                // 清空环形缓冲
                let head = LOG_HEAD.load(Ordering::Relaxed);
                let n = (head - consumed).min(LOG_RING_SLOTS as u64);
                for k in 0..n {
                    let i = ((head - n + k) & (LOG_RING_SLOTS as u64 - 1)) as usize;
                    let v = LOG_RING[i].load(Ordering::Acquire);
                    if v == 0 {
                        continue;
                    }
                    let tick = (v >> 32) as u32;
                    let kind = ((v >> 24) & 0xff) as u8;
                    let aux = (v & 0x00ff_ffff) as u32;
                    // SNAPSHOT 的 aux 是**位掩码**，单独展开成人能读的状态串；
                    // 其余事件的 aux 是毫秒数。
                    if kind == DK_SNAPSHOT {
                        diag_line(&path, &format!(
                            "SNAPSHOT      {}", state_str(aux)));
                    } else {
                        diag_line(&path, &format!(
                            "{:>10} {} aux={}ms", (tick as f64) / 1000.0, diag_kind_name(kind), aux));
                    }
                    lines += 1;
                }
                consumed = head;
                if lines > 20_000 {
                    diag_line(&path, "=== 单次会话日志行数超限，暂停记录（重启程序恢复）===");
                    break;
                }

                // ── 每 5 秒一条状态快照（用户要求「尽可能多打印」）──
                // 为什么必须落盘而不是丢进环形缓冲：环形缓冲 128 槽，
                // 一次鼠标风暴（拖动 = 每秒 60+ 事件）就能把快照冲掉。
                // 快照要**保证**每小时至少落 720 条，否则出事时正好没有现场。
                //
                // `st` 在 UI 线程，看门狗不能直接读（跨线程 &mut = UB）。
                // 绕法：原子量的部分在本线程直接读，UI 私有状态（hiding/entering 等）
                // 由 UI 线程自己每帧推一份到 SNAP_STATE 原子量。
                hb_ticks += 1;
                if hb_ticks % 5 == 0 {
                    let bits = state_bits(
                            SNAP_HIDING.load(Ordering::Relaxed),
                            SNAP_ENTERING.load(Ordering::Relaxed),
                            SNAP_PENDING_RM.load(Ordering::Relaxed),
                            SNAP_FOCUS.load(Ordering::Relaxed) != usize::MAX,
                            SNAP_HOVER_CHIP.load(Ordering::Relaxed) != usize::MAX,
                            SNAP_HIT_APP.load(Ordering::Relaxed) != usize::MAX,
                        );
                    let last_mouse = unsafe { GetTickCount() }.wrapping_sub(LAST_HOOK_TICK.load(Ordering::Relaxed));
                    diag_line(&path, &format!(
                        // `kb=` = 键盘钩子当前是否挂着（按需装卸，见 KEY_HOOK_WANT）
                        "SNAPSHOT      {}  | hook_tid={} listening={} kb={} last_mouse={}ms ago",
                        state_str(bits),
                        HOOK_TID.load(Ordering::Relaxed),
                        ST_RUNNING_FLAG.load(Ordering::Relaxed) as u8,
                        (KEY_HOOK_LOCAL.load(Ordering::Relaxed) != 0) as u8,
                        last_mouse,
                    ));
                    lines += 1;
                }
            }
        });
    }

    // 单实例保护
    unsafe {
        // 互斥体名可用环境变量 `RL_INSTANCE` 覆盖：自动化测试要能并行跑多份实例，
        // 而正常使用时不要设（否则能开出好几份，钩子会打架）。
        let suffix = std::env::var("RL_INSTANCE").unwrap_or_default();
        let mname: Vec<u16> = wide(&format!(
            "Global\\RadialMenuLauncher_SingleInstance_v3{}",
            suffix
        ));
        let h = CreateMutexW(None, BOOL(0), PCWSTR(mname.as_ptr()));
        if h.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS {
            // 互斥体被占**不等于**「已有一个可用的实例在跑」。
            // 踩过的坑：进程异常终止 / 会话切换会在进程表里留下「已退出但对象未回收」
            // 的僵尸，它们的句柄还持着互斥体 —— 于是每次启动都被静默取消，
            // 而用户看到的是「程序起不来」，且 taskkill 只会回「没有此任务的实例在运行」。
            // 判据：**真正活着的实例一定有一个宿主窗口**（RLHost）。
            // 没有窗口 = 占位的只是僵尸 → 忽略互斥体，继续正常启动。
            let host_cls: Vec<u16> = wide("RLHost");
            let live = FindWindowW(PCWSTR(host_cls.as_ptr()), PCWSTR::null()).is_ok();
            if live {
                eprintln!("[!] 径向启动器已在运行，本次启动已取消。");
                return;
            }
            eprintln!("[*] 检测到残留互斥体（无存活实例），已忽略并继续启动。");
        }
    }

    unsafe {
        let _ = SetProcessDPIAware();
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or(HMODULE(ptr::null_mut())).0);
        let mut st = State::new();
        let p: *mut State = &mut st;

        // 配置
        st.ini_path = format!("{}\\settings.ini", exe_dir());
        load_ini(&mut st);
        read_persistent(&mut st);

        // 渲染资源
        init_surfaces(&mut st);
        init_fonts(&mut st);

        // 托盘宿主窗口（不可见普通窗口：托盘菜单需要前台窗口）
        let cls: Vec<u16> = wide("RLHost");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(cls.as_ptr()),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(cls.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND(ptr::null_mut()),
            HMENU(ptr::null_mut()),
            hinst,
            None,
        )
        .unwrap_or(HWND(ptr::null_mut()));

        // 圆环分层窗口
        let rcls: Vec<u16> = wide("RLRing");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ring_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(rcls.as_ptr()),
            ..Default::default()
        });
        let rhwnd = CreateWindowExW(
            WINDOW_EX_STYLE(
                WS_EX_LAYERED.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0,
            ),
            PCWSTR(rcls.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            CANVAS,
            CANVAS,
            HWND(ptr::null_mut()),
            HMENU(ptr::null_mut()),
            hinst,
            None,
        )
        .unwrap_or(HWND(ptr::null_mut()));

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize);
        SetWindowLongPtrW(rhwnd, GWLP_USERDATA, p as isize);
        MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        RING_HWND.store(rhwnd.0 as isize, Ordering::SeqCst);

        // 设置窗口：分层窗口（ULW）+ 自绘。
        // 必须分层 —— 普通窗口的圆角只能靠 SetWindowRgn，那条边是**硬边**，
        // 放大看就是楼梯；分层窗口能带 AA 圆角和我们自己画的投影。
        let scls: Vec<u16> = wide("RLSetup");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(setup_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(scls.as_ptr()),
            ..Default::default()
        });
        let shwnd = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_LAYERED.0 | WS_EX_TOOLWINDOW.0),
            PCWSTR(scls.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            S_WW,
            S_HH,
            HWND(ptr::null_mut()),
            HMENU(ptr::null_mut()),
            hinst,
            None,
        )
        .unwrap_or(HWND(ptr::null_mut()));
        SetWindowLongPtrW(shwnd, GWLP_USERDATA, p as isize);
        SET_HWND.store(shwnd.0 as isize, Ordering::SeqCst);
        // --settings：启动后直接把设置页调出来（便于做快捷方式 / 自动化核对）
        if args.iter().any(|a| a == "--settings") {
            open_settings(&mut st);
        }

        // 拖放入环：注册 OLE 放置目标 + 放行跨完整性级别的拖放消息（提权场景）
        let drop_target: IDropTarget = RingDropTarget.into();
        match RegisterDragDrop(rhwnd, &drop_target) {
            Ok(()) => drag_log("RegisterDragDrop OK"),
            Err(e) => {
                drag_log(&format!("RegisterDragDrop FAIL {:?}", e.code()));
                eprintln!("[!] 拖放目标注册失败：{:?}", e.code());
            }
        }
        DragAcceptFiles(rhwnd, true);
        let _ = ChangeWindowMessageFilterEx(rhwnd, 0x0233, MSGFLT_ALLOW, None); // WM_DROPFILES
        let _ = ChangeWindowMessageFilterEx(rhwnd, 0x004A, MSGFLT_ALLOW, None); // WM_COPYDATA
        let _ = ChangeWindowMessageFilterEx(rhwnd, 0x0049, MSGFLT_ALLOW, None);

        add_tray_icon(hwnd);
        let _ = RegisterHotKey(hwnd, HOTKEY_TOGGLE, MOD_CONTROL | MOD_ALT, 0x52); // Ctrl+Alt+R

        // 钩子装在专用线程里（见 set_running 注释）
        let listen_on = st.listening;
        set_running(&mut st, listen_on);

        println!("[*] 径向启动器已就绪：中键呼出 · 托盘菜单 / Ctrl+Alt+R 开关监听");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
        std::mem::forget(st);
    }
}
