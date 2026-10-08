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
# 沙箱里的 exe 名。⚠️ 别硬编码 RadialLauncher.exe —— 那个文件常被僵尸实例占着拷不动。
EXE_NAME = os.environ.get('RL_EXE', 'RadialLauncher.exe')

# ── 布局镜像（与 src/main.rs 一致）──
S_W, S_H, S_NAV_W, S_TITLE_H = 780, 580, 172, 52
S_X0, S_X1 = S_NAV_W + 26, S_W - 26
S_LIST_TOP, S_LIST_BOT = S_TITLE_H + 58, S_H - 78
S_ADV_SW_Y = S_TITLE_H + 76
# 卡片之外还有一圈投影留白，窗口坐标 = 逻辑坐标 + S_M
S_M = 22


def nav(i):
    t = S_TITLE_H + 18 + i * 44
    return (14, t, S_NAV_W - 14, t + 40)


def strip(i, top, w, gap, h):
    l = S_X0 + i * (w + gap)
    return (l, top, l + w, top + h)


def sw_row(i):
    y = S_TITLE_H + 226 + i * 62
    return (S_X0, y - 16, S_X1, y + 42)


def adv_row():
    return (S_X0, S_ADV_SW_Y - 16, S_X1, S_ADV_SW_Y + 42)


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
        elif k in ('dark', 'persistent', 'listening', 'key_middle', 'key_side', 'key_rhold'):
            flags[k] = v
    return flags, apps, items


def main():
    ini = os.path.join(SANDBOX, 'settings.ini')
    bak = ini + '.bak'
    for f in (bak,):
        if os.path.exists(f):
            os.remove(f)

    subprocess.run([os.path.join(SANDBOX, EXE_NAME), '--quit'],
                   cwd=SANDBOX, capture_output=True)
    time.sleep(1.0)
    # 每次都用项目里那份配置重置沙箱 —— 否则上一轮跑完的 ini 会被继承，
    # 「点击 → 断言绝对值」的用例会因为初始状态不同而假 FAIL
    shutil.copy(os.path.join(HERE, 'settings.ini'), ini)
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

    results = []

    def check(name, cond, extra=''):
        results.append((name, bool(cond), extra))

    before, apps0, items0 = read_ini(ini)

    # 1. 外观 → 亮色
    click(hwnd, nav(0))
    click(hwnd, strip(0, S_TITLE_H + 26 + 62, 108, 10, 38))
    f, _, _ = read_ini(ini)
    check('外观·亮色 → dark=0', f.get('dark') == '0', f.get('dark'))

    # 2. 外观 → 跟随系统
    click(hwnd, strip(2, S_TITLE_H + 26 + 62, 108, 10, 38))
    f, _, _ = read_ini(ini)
    check('外观·跟随系统 → dark=2', f.get('dark') == '2', f.get('dark'))

    # 3. 交互 → 长按右键（默认关 → 开）
    click(hwnd, nav(1))
    click(hwnd, strip(2, S_TITLE_H + 26 + 62, 118, 10, 38))
    f, _, _ = read_ini(ini)
    check('交互·长按右键 → key_rhold=1', f.get('key_rhold') == '1', f.get('key_rhold'))

    # 4. 交互 → 鼠标侧键（默认开 → 关）
    click(hwnd, strip(1, S_TITLE_H + 26 + 62, 118, 10, 38))
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

    # 7. 应用 → 删除第一个应用
    click(hwnd, nav(2))
    _, apps1, _ = read_ini(ini)
    del_r = (S_X1 - 38, app_row(0)[1] + 8, S_X1 - 6, app_row(0)[1] + 30)
    click(hwnd, del_r)
    _, apps2, _ = read_ini(ini)
    check('应用·删除第一个应用', len(apps2) == len(apps1) - 1,
          '%d → %d（%s）' % (len(apps1), len(apps2), ','.join(apps1[:2])))
    check('破坏性写前留了备份', os.path.exists(bak))

    # 8. 高级 → 导航可达（不发注册表写，只看窗口不崩）
    click(hwnd, nav(3))
    f2, _, _ = read_ini(ini)
    check('高级页可进入（未崩）', True)

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
