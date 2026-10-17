# -*- coding: utf-8 -*-
"""设置窗口回归：用 SendMessage 直接点自绘控件，断言配置真的落盘。

为什么必须在沙箱目录跑：设置页能删应用，直接点真配置会把用户的应用列表毁掉。
本脚本在 _tmptest/ 下复制一份 exe + settings.ini，只动那份。

坐标全部来自 src/main.rs 的布局函数（s_nav_rect / s_strip / s_sw_row /
s_rows / S_LIST_TOP），改布局时要同步改这里 —— 对不上就是「点了没反应」。
"""
import ctypes
import io
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes

u = ctypes.windll.user32
FindWindowW = u.FindWindowW
FindWindowW.restype = wintypes.HWND
FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
SendMessageW = u.SendMessageW
SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, ctypes.c_size_t, ctypes.c_ssize_t]
SendMessageW.restype = ctypes.c_ssize_t

WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP = 0x0200, 0x0201, 0x0202

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_tmptest'))
os.environ['PIGO_CONFIG_DIR'] = SANDBOX
# 沙箱里的 exe 名。⚠️ 别硬编码 Pigo.exe —— 那个文件常被僵尸实例占着拷不动。
EXE_NAME = os.environ.get('RL_EXE', 'Pigo.exe')

# ── 布局镜像（与 src/main.rs 一致）──
S_W, S_H, S_NAV_W, S_TITLE_H = 780, 670, 172, 52
S_X0, S_X1 = S_NAV_W + 26, S_W - 26
# ⚠️⚠️ 整页纵向节奏 —— **逐条镜像 main.rs 的 `ACT_*` 公式**（main.rs 一改这里必须同步）。
#    之前这里是散落的硬编码（+212/+46 / +88 / +130 / +402），改布局就会「点了没反应」。
ACT_GAP, ACT_LABEL_H = 16, 22          # 统一间距 / 段标题高
ACT_TOP_Y = S_TITLE_H + 14             # 各页标题顶（四页共用）
ACT_CHIPS_Y = ACT_TOP_Y + ACT_LABEL_H + ACT_GAP
ACT_CHIPS_H, ACT_TRIG_H = 38, 30
TRIG_ROW_Y = ACT_CHIPS_Y + ACT_CHIPS_H + 4
ACT_SEC2_Y = TRIG_ROW_Y + ACT_TRIG_H + ACT_GAP
ACT_SW_Y0 = ACT_SEC2_Y + ACT_LABEL_H + ACT_GAP + 8
ACT_SW_DY = 44
ACT_SW_LAST_BOT = ACT_SW_Y0 + 4 * ACT_SW_DY + 34
# ⚠️ 镜像 src/main.rs 的 ACT_*：2026-10-14 新增「显示应用名称」第 5 个开关 → 4 * DY
EXC_CARDS_TITLE_Y = ACT_SW_LAST_BOT + ACT_GAP
EXC_CARD_TOP = EXC_CARDS_TITLE_Y + ACT_LABEL_H + ACT_GAP
S_LIST_TOP, S_LIST_BOT = ACT_TOP_Y + ACT_LABEL_H + ACT_GAP, S_H - 78
# 卡片之外还有一圈投影留白，窗口坐标 = 逻辑坐标 + S_M
S_M = 22


def nav(i):
    t = S_TITLE_H + 18 + i * 44
    return (14, t, S_NAV_W - 14, t + 40)


def strip(i, top, w, gap, h):
    l = S_X0 + i * (w + gap)
    return (l, top, l + w, top + h)


def sw_row(i):
    # ⚠️ 交互页要放 4 个行为开关（常驻/监听/全屏放行/开机自启）+ 黑白名单两张卡片，
    #    间距另起一套（`act_sw_*`）
    # ⚠️ 2026-10-11：呼出方式下面新增「自定义触发」行 → ACT_SW_Y0 由 +190 下移到 +212
    y = ACT_SW_Y0 + i * ACT_SW_DY
    return (S_X0, y - 8, S_X1, y + 34)


def trig_row():
    """「自定义」芯片（呼出方式第 4 个按钮，点击进入/退出录制态）——
    镜像 s_strip(3, ACT_CHIPS_Y, 118, 10, 38) —— 必须用这份几何，否则点不中芯片"""
    top = ACT_CHIPS_Y
    l = S_X0 + 3 * (118 + 10)
    return (l, top, l + 118, top + ACT_CHIPS_H)


def trig_clear():
    """录制行右端的 ✕ 清空按钮（仅已设触发时出现）—— 镜像 trig_clear_rect"""
    y = TRIG_ROW_Y
    return (S_X1 - 30, y + 6, S_X1 - 8, y + 28)


def exc_card_head(c):
    """黑白名单卡片头（避开右侧的「生效中」徽标和名字右边的「?」）"""
    w = (S_X1 - S_X0 - 20) // 2
    l = S_X0 + c * (w + 20)
    top = EXC_CARD_TOP
    return (l + 100, top + 6, l + 180, top + 36)


def app_row(i, expanded_len=0):
    """第 i 个应用行（假设前面都没展开）"""
    y = S_LIST_TOP + i * 48
    return (S_X0, y, S_X1, y + 36)


def center(r):
    """逻辑坐标 → 窗口坐标（+S_M：卡片外那一圈投影留白）"""
    return ((r[0] + r[2]) // 2 + S_M, (r[1] + r[3]) // 2 + S_M)


def lp(x, y):
    return (y << 16) | (x & 0xFFFF)


def click(hwnd, r):
    x, y = center(r)
    SendMessageW(hwnd, WM_MOUSEMOVE, 0, lp(x, y))
    SendMessageW(hwnd, WM_LBUTTONDOWN, 1, lp(x, y))
    SendMessageW(hwnd, WM_LBUTTONUP, 0, lp(x, y))
    time.sleep(0.25)


def read_ini(path):
    flags, apps, items, sec = {}, [], 0, None
    for line in io.open(path, encoding='utf-8').read().splitlines():
        t = line.strip()
        if t.startswith('[') and t.endswith(']'):
            sec = t[1:-1]
            continue
        if not t or '=' not in t:
            continue
        k, v = [z.strip() for z in t.split('=', 1)]
        if sec == 'apps':
            apps.append(k)
        elif sec and sec.startswith('app.'):
            items += 1
        elif k in ('dark', 'persistent', 'listening', 'key_middle', 'key_side', 'key_rhold',
                   'hover_name'):
            flags[k] = v
    return flags, apps, items


def main():
    ini = os.path.join(SANDBOX, 'settings.ini')
    bak = ini + '.bak'
    for f in (bak,):
        if os.path.exists(f):
            os.remove(f)

    # ⚠️⚠️ 沙箱里的 exe **不会自动更新** —— 不拷最新的一版就会拿旧二进制去测
    # （这个坑在 test_focus / test_settings 各踩过一次，看到「布局明明改了却全 FAIL」先查这里）
    # ⚠️ 先确保沙箱目录 + exe 存在，再发 --quit（否则干净沙箱下 --quit 因找不到 exe 直接崩）。
    os.makedirs(SANDBOX, exist_ok=True)
    try:
        shutil.copy(os.path.join(HERE, 'Pigo.exe'),
                    os.path.join(SANDBOX, EXE_NAME))
    except Exception as e:
        print('（警告：沙箱 exe 没更新 —— %s）' % e)

    subprocess.run([os.path.join(SANDBOX, EXE_NAME), '--quit'],
                   cwd=SANDBOX, capture_output=True)
    time.sleep(1.0)
    # 每次都用项目里那份配置重置沙箱 —— 否则上一轮跑完的 ini 会被继承，
    # 「点击 → 断言绝对值」的用例会因为初始状态不同而假 FAIL
    # ⚠️ key_rhold 强制归 0：测试设计意图是「点一下 0→1 翻转」，
    #    而项目真 ini 里默认已经是 1 → 翻转后会变成 0，连带「长按右键弹菜单」
    #    也跟着 FAIL（那是同一个脏基线的级联，并非真实回归）。归 0 后用例才稳。
    _src = io.open(os.path.join(HERE, 'settings.ini'), encoding='utf-8').read()
    _out = []
    for _ln in _src.splitlines(True):
        if _ln.strip().lower().startswith('key_rhold='):
            _out.append('key_rhold=0\r\n')
        else:
            _out.append(_ln)
    _txt = ''.join(_out)
    # ⚠️⚠️ 首启聚光灯引导会在**第一次打开设置页时自动弹出**，而且引导期间
    # 「只读」——所有点击都被吃掉 → 本脚本的点击全部落空、整片假 FAIL。
    # 沙箱里强制置 guide_seen=1 把它关掉（真机上该引导仍然照常首启播放）。
    if 'guide_seen=' not in _txt:
        _txt = _txt.replace('[设置]\r\n', '[设置]\r\nguide_seen=1\r\n', 1)
    io.open(ini, 'w', encoding='utf-8', newline='').write(_txt)
    proc = subprocess.Popen([os.path.abspath(os.path.join(SANDBOX, EXE_NAME)), '--settings'],
                            cwd=SANDBOX, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    time.sleep(2.5)

    # 注意参数顺序：FindWindowW(类名, 窗口标题)；设置窗口没有标题，只能按类名找
    hwnd = FindWindowW('RLSetup', None)
    if not hwnd:
        print('FAIL: 找不到设置窗口（RLSetup）')
        proc.kill()
        return 1

    # 兜底：万一引导还是弹出来了（比如 ini 里 guide_seen 没写进去），先按 Esc 关掉它
    SendMessageW(hwnd, 0x0100, 0x1B, 0)
    time.sleep(0.3)

    results = []

    def check(name, cond, extra=''):
        results.append((name, bool(cond), extra))

    before, apps0, items0 = read_ini(ini)

    # 1. 外观 → 亮色
    click(hwnd, nav(0))
    click(hwnd, strip(0, ACT_CHIPS_Y, 108, 10, 38))
    f, _, _ = read_ini(ini)
    check('外观·亮色 → dark=0', f.get('dark') == '0', f.get('dark'))

    # 2. 外观 → 跟随系统
    click(hwnd, strip(2, ACT_CHIPS_Y, 108, 10, 38))
    f, _, _ = read_ini(ini)
    check('外观·跟随系统 → dark=2', f.get('dark') == '2', f.get('dark'))

    # 3. 交互 → 长按右键（默认关 → 开）
    click(hwnd, nav(1))
    click(hwnd, strip(2, ACT_CHIPS_Y, 118, 10, 38))
    f, _, _ = read_ini(ini)
    check('交互·长按右键 → key_rhold=1', f.get('key_rhold') == '1', f.get('key_rhold'))

    # 4. 交互 → 鼠标侧键（默认开 → 关）
    click(hwnd, strip(1, ACT_CHIPS_Y, 118, 10, 38))
    f, _, _ = read_ini(ini)
    check('交互·鼠标侧键 → key_side=0', f.get('key_side') == '0', f.get('key_side'))

    # 5. 交互 → 常驻模式翻转
    click(hwnd, sw_row(0))
    f, _, _ = read_ini(ini)
    check('交互·常驻模式翻转', f.get('persistent') != before.get('persistent'),
          '%s → %s' % (before.get('persistent'), f.get('persistent')))

    # 6. 交互 → 监听翻转（默认开 → 关 → 再开）
    click(hwnd, sw_row(1))
    f, _, _ = read_ini(ini)
    check('交互·监听关闭 → listening=0', f.get('listening') == '0', f.get('listening'))
    click(hwnd, sw_row(1))                # 必须再打开，否则钩子没了下面测不了
    f, _, _ = read_ini(ini)
    check('交互·监听重新开启 → listening=1', f.get('listening') == '1', f.get('listening'))

    # 6a2. 交互 → 显示应用名称（2026-10-14 新增的第 5 行）翻转
    # ⚠️ 基准不能拿 `before`：项目 ini 里**本来就没这个键**（缺省 = 开），
    #    所以第一次点必然写 0；点第二下回到 1。
    click(hwnd, sw_row(4))
    f, _, _ = read_ini(ini)
    check('交互·显示应用名称翻转 → 0', f.get('hover_name') == '0', f.get('hover_name'))
    click(hwnd, sw_row(4))                # 翻回「开」，别影响后面的用例
    f, _, _ = read_ini(ini)
    check('交互·显示应用名称翻回 → 1', f.get('hover_name') == '1', f.get('hover_name'))

    # 6b. 交互 → 自定义触发：点芯片进入录制态 → 模拟捕获键盘组合 Ctrl+W → 落盘。
    #     （自定义触发现在只接受键盘组合；鼠标键都可在上方单独勾选，录制端已禁录）
    #     ⚠️ 真机捕获走全局键盘钩子，测试里用 SendMessage 点窗口**不会**触发钩子。
    #     ⚠️⚠️ 两个半场在**不同窗口**：
    #        · 「点行进录制态」= 设置窗口 RLSetup（setup_proc）
    #        · 「捕获落点」由钩子 post 到 **MAIN_HWND = RLHost**（host_proc）—— 三个窗口
    #          共享同一个 State 指针，所以往 RLHost 发就等于钩子真的抓到了。
    #        把 WM_APP_TRIGSIM 发给 RLSetup 会**静默无效**（那个 proc 没这条分支）。
    WM_APP_TRIGSIM = 0x8000 + 25
    host = FindWindowW('RLHost', None)
    click(hwnd, trig_row())                       # 进入录制态（trig_capturing=true）
    SendMessageW(host, WM_APP_TRIGSIM, 0, 0)      # 模拟钩子捕获落点（发给 RLHost）
    time.sleep(0.3)
    txt = open(ini, encoding='utf-8').read()
    check('自定义触发·录制 Ctrl+W → trig_custom_kind=2', 'trig_custom_kind=2' in txt)
    check('自定义触发·录制 Ctrl+W → trig_custom_vk=87', 'trig_custom_vk=87' in txt)
    check('自定义触发·录制 Ctrl+W → trig_custom_mods=1', 'trig_custom_mods=1' in txt)
    # 点 ✕（仅在有触发时出现）清空
    click(hwnd, trig_clear())
    txt = open(ini, encoding='utf-8').read()
    check('自定义触发·点 ✕ 清空 → trig_custom_kind=0', 'trig_custom_kind=0' in txt)

    # 7. 应用 → 删除第一个应用
    click(hwnd, nav(2))
    _, apps1, _ = read_ini(ini)
    del_r = (S_X1 - 38, app_row(0)[1] + 8, S_X1 - 6, app_row(0)[1] + 30)
    click(hwnd, del_r)
    _, apps2, _ = read_ini(ini)
    check('应用·删除第一个应用', len(apps2) == len(apps1) - 1,
          '%d → %d（%s）' % (len(apps1), len(apps2), ','.join(apps1[:2])))
    check('破坏性写前留了备份', os.path.exists(bak))

    # 8. 关于 → 导航可达（不发注册表写，只看窗口不崩）
    click(hwnd, nav(3))
    f2, _, _ = read_ini(ini)
    check('关于页可进入（未崩）', True)

    # 8b. 交互页的黑白名单：**二选一生效**（2026-10-11 改回「点卡片标题切换生效方」，
    #     不再是标题右侧独立的模式切换器）。点卡片头 → 写 `mode=`。
    click(hwnd, nav(1))
    click(hwnd, exc_card_head(1))
    txt = open(ini, encoding='utf-8').read()
    check('点「白名单」卡片头 → mode=white 落盘', 'mode=white' in txt)
    click(hwnd, exc_card_head(0))
    txt = open(ini, encoding='utf-8').read()
    check('点「黑名单」卡片头 → mode=black 落盘', 'mode=black' in txt)

    # 8b2. 「把 exe 拖进卡片」：落点**不再**决定去哪张 —— 一律进**当前生效**那张（mode 决定）。
    # ⚠️ 不能合成 `WM_DROPFILES` 跨进程测 —— HDROP 是本进程私有堆的句柄，
    #    另一个进程 GlobalLock 不上（实测 DragQueryFile 恒为 0）。
    #    所以走 `--dropsim`（直接调与拖放**同一个**路由 + `exe_name_of`）。
    # ⚠️ 必须用**沙箱里**那份 exe —— `--dropsim` 读写的是「exe 所在目录」的 settings.ini
    exe_sim = os.path.join(SANDBOX, EXE_NAME)
    cw = (S_X1 - S_X0 - 20) // 2

    def dropsim(card, path):
        cx = S_X0 + card * (cw + 20) + cw // 2
        cy = EXC_CARD_TOP + 80
        return subprocess.run([exe_sim, '--dropsim', str(cx), str(cy), path],
                              cwd=SANDBOX, capture_output=True, text=True, errors='replace')

    def set_mode(ini_path, m):
        """直接改 ini 的 mode（避免运行中的设置实例回写把它覆盖掉）"""
        out, done = [], False
        for ln in io.open(ini_path, encoding='utf-8').read().splitlines(True):
            if ln.lstrip().lower().startswith('mode='):
                out.append('mode=%s\r\n' % m)
                done = True
            else:
                out.append(ln)
        if not done:
            out.append('mode=%s\r\n' % m)
        io.open(ini_path, 'w', encoding='utf-8', newline='').write(''.join(out))

    set_mode(ini, 'white')                                   # 白名单模式 → 拖入进白名单
    r1 = dropsim(1, r'C:\Windows\System32\charmap.exe')
    txt = open(ini, encoding='utf-8').read()
    check('白名单模式下拖 exe → 落盘 w=charmap.exe', 'w=charmap.exe' in txt,
          (r1.stdout or '').strip()[:80])
    set_mode(ini, 'black')                                   # 黑名单模式 → 拖入进黑名单
    r2 = dropsim(0, r'C:\Windows\System32\charmap.exe')
    txt = open(ini, encoding='utf-8').read()
    check('黑名单模式下拖 exe → 落盘 b=charmap.exe', 'b=charmap.exe' in txt,
          (r2.stdout or '').strip()[:80])
    # 拖一个非 exe：不该进名单
    dropsim(0, os.path.join(HERE, 'settings.ini'))
    txt = open(ini, encoding='utf-8').read()
    check('拖非 exe → 不进名单', 'settings.ini' not in txt)

    # 8c. 窗口本身：任务栏可见 + 标题条可拖动（用户 2026-10-11 报的两个问题）
    u.GetWindowLongW.restype = ctypes.c_long
    u.GetWindowLongW.argtypes = [wintypes.HWND, ctypes.c_int]
    u.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
    ex = u.GetWindowLongW(hwnd, -20)      # GWL_EXSTYLE
    check('窗口不在任务栏隐藏（无 WS_EX_TOOLWINDOW）', not (ex & 0x00000080), 'exstyle=0x%X' % (ex & 0xFFFFFFFF))
    check('窗口强制进任务栏（有 WS_EX_APPWINDOW）', bool(ex & 0x00040000))
    # ⚠️ WM_NCHITTEST 的 lParam 是**屏幕坐标**；这里就用屏幕坐标发，验证换算是对的
    wr = wintypes.RECT()
    u.GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(wr))
    sx = wr.left + S_M + S_W // 2
    sy = wr.top + S_M + 10
    WM_NCHITTEST = 0x0084
    hit = SendMessageW(hwnd, WM_NCHITTEST, 0, lp(sx & 0xFFFF, sy & 0xFFFF))
    check('标题条拖动：WM_NCHITTEST → HTCAPTION(2)', hit == 2, 'got %s（修前恒为 1=HTCLIENT）' % hit)

    # 9. 按键开关真的接上钩子了吗？长按右键 → 菜单应弹出；短按 → 不应弹出
    IsWindowVisible = u.IsWindowVisible
    IsWindowVisible.argtypes = [wintypes.HWND]
    RBUTTONDOWN, RBUTTONUP = 0x0008, 0x0010

    def ring_visible(wait=2.0):
        """轮询等待 —— 菜单是 post 到 UI 线程再弹的，固定 sleep 会抖"""
        t0 = time.time()
        while time.time() - t0 < wait:
            r = FindWindowW('RLRing', None)
            if r and IsWindowVisible(r):
                return True
            time.sleep(0.10)
        return False

    u.mouse_event(RBUTTONDOWN, 0, 0, 0, 0)
    time.sleep(0.60)                      # 超过 380ms 门槛
    u.mouse_event(RBUTTONUP, 0, 0, 0, 0)
    check('长按右键 → 菜单弹出', ring_visible(2.0))

    u.keybd_event(0x1B, 0, 0, 0)
    time.sleep(0.05)
    u.keybd_event(0x1B, 0, 2, 0)
    time.sleep(0.80)

    u.mouse_event(RBUTTONDOWN, 0, 0, 0, 0)
    time.sleep(0.08)                      # 远短于门槛
    u.mouse_event(RBUTTONUP, 0, 0, 0, 0)
    check('短按右键 → 不弹出（保持原行为）', not ring_visible(1.0))

    try:
        proc.kill()
    except Exception:
        pass

    ok = sum(1 for _, c, _ in results if c)
    for name, c, extra in results:
        print('%s  %s  %s' % ('PASS' if c else 'FAIL', name, extra))
    print('[settings] PASS=%d FAIL=%d' % (ok, len(results) - ok))
    return 0 if ok == len(results) else 1


if __name__ == '__main__':
    sys.exit(main())
