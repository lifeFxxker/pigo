# -*- coding: utf-8 -*-
"""聚焦（压暗遮罩）回归：悬停某应用时其它应用应该压暗；**指针离开后必须恢复**。

用户实测的问题：悬停过 A 之后把指针挪到中心头像，其它应用的压暗遮罩一直挂着不散。
根因是压暗目标拿的是粘性的 `active_app`（为了「移向外环不丢选中」故意不清），
而不是「真实命中」——和之前「点屏幕任意位置都启动应用」是同一类坑。

判定完全基于像素：压暗是「图标 alpha × 0.62」，深色图标会因此明显变亮（白底透出来）。
先自己找一颗最深的图标做探针，再对比「悬停别人时」和「指针回到中心后」的亮度。
"""
import ctypes
import os
import subprocess
import sys
import time
from ctypes import wintypes as w

from PIL import Image

u = ctypes.WinDLL('user32', use_last_error=True)
FindWindowW = u.FindWindowW
FindWindowW.restype = w.HWND
FindWindowW.argtypes = [w.LPCWSTR, w.LPCWSTR]
SendMessageW = u.SendMessageW
SendMessageW.restype = ctypes.c_ssize_t
SendMessageW.argtypes = [w.HWND, w.UINT, ctypes.c_size_t, ctypes.c_ssize_t]

WM_APP = 0x8000
WM_APP_DUMP = WM_APP + 6
WM_APP_DEBUGPOS = WM_APP + 7
MIDDLEDOWN, MIDDLEUP = 0x0020, 0x0040

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ring_geom as G

CC = G.CC
EXE_DIR = os.path.dirname(os.path.abspath(__file__))
DUMP = os.path.join(EXE_DIR, 'live_dump.bmp')
results = []


def check(name, ok, extra=''):
    results.append((name, ok))
    print('%s %s %s' % ('PASS' if ok else 'FAIL', name, extra))


def apps():
    a, _ = G.read_settings(G.settings_path(EXE_DIR))
    return a


def tile_center(idx, n):
    r_icon, _ = G.ring_layout(n)
    return G.polar(r_icon, idx * (360.0 / n))


def brightness(idx, n, half=5):
    """图标中心 half×half 方块的平均亮度"""
    im = Image.open(DUMP).convert('RGB')
    px = im.load()
    cx, cy = tile_center(idx, n)
    cx, cy = int(cx), int(cy)
    vals = []
    for y in range(cy - half, cy + half + 1):
        for x in range(cx - half, cx + half + 1):
            if 0 <= x < im.width and 0 <= y < im.height:
                r, g, b = px[x, y]
                vals.append((r + g + b) / 3.0)
    return sum(vals) / len(vals) if vals else 0.0


def main():
    subprocess.run([os.path.join(EXE_DIR, 'RadialLauncher.exe'), '--quit'],
                   capture_output=True)
    time.sleep(1.2)
    p = subprocess.Popen([os.path.join(EXE_DIR, 'RadialLauncher.exe')],
                         creationflags=0x00000008 | 0x00000200,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2.5)
    host = FindWindowW('RLHost', None)
    if not host:
        print('FAIL: 找不到窗口')
        p.terminate()
        return 1

    cur = (1000, 560)
    u.SetCursorPos(*cur)
    time.sleep(0.2)
    u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
    time.sleep(0.5)
    u.mouse_event(MIDDLEUP, 0, 0, 0, 0)
    time.sleep(0.4)

    names = apps()
    n = len(names)
    wx, wy = cur[0] - CC, cur[1] - CC

    def dump_and(sample_idx):
        SendMessageW(host, WM_APP_DUMP, 0, 0)
        time.sleep(0.30)
        return brightness(sample_idx, n)

    # 1) 指针停在圆心（= 头像），量出所有应用的「原色」，挑一颗最深的做探针
    SendMessageW(host, WM_APP_DEBUGPOS, cur[0], cur[1])
    time.sleep(0.55)
    base = {}
    SendMessageW(host, WM_APP_DUMP, 0, 0)
    time.sleep(0.30)
    for i in range(n):
        base[i] = brightness(i, n)
    probe = min(range(n), key=lambda i: base[i])
    hover = (probe + n // 2) % n          # 挑一个离探针最远的应用来悬停
    print('  探针应用 = %s（亮度 %.0f）· 悬停应用 = %s' % (names[probe], base[probe], names[hover]))

    # 2) 悬停到 hover → 探针应该被压暗（图标变淡 → 深色图标变亮）
    px, py = tile_center(hover, n)
    SendMessageW(host, WM_APP_DEBUGPOS, wx + int(px), wy + int(py))
    time.sleep(0.75)
    dimmed = dump_and(probe)

    # 3) 指针移回中心头像 → 压暗必须**完全撤掉**
    SendMessageW(host, WM_APP_DEBUGPOS, cur[0], cur[1])
    time.sleep(0.75)
    after = dump_and(probe)

    u.keybd_event(0x1B, 0, 0, 0)
    time.sleep(0.05)
    u.keybd_event(0x1B, 0, 2, 0)
    time.sleep(0.3)
    p.terminate()

    print('  探针亮度：原始 %.1f · 悬停别人时 %.1f · 回到中心后 %.1f' % (base[probe], dimmed, after))
    check('悬停某应用 → 其它应用被压暗', dimmed > base[probe] + 8.0,
          '(%.1f → %.1f)' % (base[probe], dimmed))
    check('指针离开 → 压暗完全撤销', abs(after - base[probe]) <= 6.0,
          '(%.1f vs 原始 %.1f)' % (after, base[probe]))

    try:
        os.remove(DUMP)
    except Exception:
        pass
    bad = [r for r in results if not r[1]]
    print()
    print('合计 %d 项，失败 %d 项' % (len(results), len(bad)))
    return 1 if bad else 0


sys.exit(main())
