import ctypes, time, subprocess, sys
from ctypes import wintypes as w

u = ctypes.WinDLL('user32', use_last_error=True)

FindWindowW = u.FindWindowW
FindWindowW.restype = w.HWND
FindWindowW.argtypes = [w.LPCWSTR, w.LPCWSTR]
IsWindowVisible = u.IsWindowVisible
IsWindowVisible.restype = w.BOOL
IsWindowVisible.argtypes = [w.HWND]
SendMessageW = u.SendMessageW
SendMessageW.restype = ctypes.c_ssize_t
SendMessageW.argtypes = [w.HWND, w.UINT, ctypes.c_size_t, ctypes.c_ssize_t]

WM_APP = 0x8000
WM_APP_SHOW = WM_APP + 1
WM_APP_DUMP = WM_APP + 6
WM_APP_DEBUGPOS = WM_APP + 7

MIDDLEDOWN, MIDDLEUP = 0x0020, 0x0040
VK_ESCAPE = 0x1B
KEYUP = 2

import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ring_geom as G

EXE_DIR = os.path.dirname(os.path.abspath(__file__))


def find(cls):
    return FindWindowW(cls, None)


def visible(h):
    return bool(IsWindowVisible(h)) if h else False


def main():
    os.environ['PIGO_CONFIG_DIR'] = os.path.dirname(os.path.abspath(__file__))
    os.environ['RL_INSTANCE'] = 'livetest'
    p = subprocess.Popen([r'Pigo.exe'],
                         creationflags=0x00000008 | 0x00000200,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    time.sleep(2.5)
    host = find('RLHost')
    ring = find('RLRing')
    print('host=%s ring=%s' % (host, ring))
    if not ring:
        print('FAIL: 找不到窗口')
        p.terminate()
        return 1

    # 呼出菜单（光标定位到右上区域，便于截到真实桌面内容）
    u.SetCursorPos(1000, 560)
    time.sleep(0.2)
    u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
    time.sleep(0.5)
    print('菜单可见:', visible(ring))

    win_x = 1000 - int(G.CC)
    win_y = 560 - int(G.CC)
    if win_x < 0 or win_y < 0:
        # 被钳位到虚拟屏左上，按 0 处理
        win_x = max(win_x, 0)
        win_y = max(win_y, 0)

    import math
    # 几何全部从 settings.ini 推导：应用数一变，内环半径就跟着变，
    # 硬编码坐标会整体落空（hover 打不中 → 截图里没有 chips）。
    apps, items = G.read_settings(os.path.join(EXE_DIR, 'settings.ini'))
    n = len(apps)
    r_icon, _sz = G.ring_layout(n)
    target, names = max(items.items(), key=lambda kv: len(kv[1]))
    ai = apps.index(target) if target in apps else 0
    a0 = ai * 360.0 / n
    print('apps=%d 目标=%s idx=%d a0=%.1f items=%d' % (n, target, ai, a0, len(names)))

    ax = win_x + int(G.CC + r_icon * math.sin(math.radians(a0)))
    ay = win_y + int(G.CC - r_icon * math.cos(math.radians(a0)))
    SendMessageW(host, WM_APP_DEBUGPOS, ax, ay)
    time.sleep(0.45)

    # 外环 chip 的方位角/半径由 chip_layout 给出（整圈均分）
    ca, cr = G.chip_layout(names, a0)[0]
    cx = win_x + int(G.CC + cr * math.sin(math.radians(ca)))
    cy = win_y + int(G.CC - cr * math.cos(math.radians(ca)))
    SendMessageW(host, WM_APP_DEBUGPOS, cx, cy)
    time.sleep(0.45)
    SendMessageW(host, WM_APP_DUMP, 0, 0)
    time.sleep(0.35)

    # 收起
    u.mouse_event(MIDDLEUP, 0, 0, 0, 0)
    u.keybd_event(VK_ESCAPE, 0, 0, 0)
    time.sleep(0.05)
    u.keybd_event(VK_ESCAPE, 0, KEYUP, 0)
    time.sleep(0.3)

    p.terminate()
    try:
        out = p.communicate(timeout=3)[0].decode('utf-8', 'ignore')
    except Exception:
        out = ''
    print('--- 程序输出 ---')
    print(out.strip()[:800])
    return 0


sys.exit(main())
