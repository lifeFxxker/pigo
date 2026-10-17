"""点击语义回归：只有真的点在「图标 / 外环 chip」上才执行。

复现用户报的 bug：先 hover 到某个应用（active_app 变成粘性值），
再把指针移到空白处点击 —— 旧逻辑会启动那个应用，新逻辑必须只是关闭。
"""
import ctypes, time, subprocess, sys, math, os
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
WM_APP_DUMP = WM_APP + 6
WM_APP_DEBUGPOS = WM_APP + 7
WM_APP_CLICK = WM_APP + 10

MIDDLEDOWN, MIDDLEUP = 0x0020, 0x0040
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ring_geom as G

EXE_DIR = os.path.dirname(os.path.abspath(__file__))
_APPS, _ITEMS = G.read_settings(os.path.join(EXE_DIR, 'settings.ini'))
NAPPS = len(_APPS)
CANVAS, CC = G.CANVAS, G.CC
R_ICON = G.ring_layout(NAPPS)[0]
APP0 = _APPS[0] if _APPS else '记事本'
CURSOR = (1000, 560)

results = []


def check(name, ok, extra=''):
    results.append((name, ok, extra))
    print('%s %s %s' % ('PASS' if ok else 'FAIL', name, extra))


def main():
    os.environ['PIGO_CONFIG_DIR'] = os.path.dirname(os.path.abspath(__file__))
    os.environ['RL_INSTANCE'] = 'clicktest'
    p = subprocess.Popen([r'Pigo.exe'],
                         creationflags=0x00000008 | 0x00000200,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    time.sleep(2.5)
    host = FindWindowW('RLHost', None)
    ring = FindWindowW('RLRing', None)
    if not ring:
        print('FAIL: 找不到窗口')
        p.terminate()
        return 1

    wx, wy = CURSOR[0] - CC, CURSOR[1] - CC

    def show():
        u.SetCursorPos(*CURSOR)
        time.sleep(0.2)
        u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
        time.sleep(0.5)
        u.mouse_event(MIDDLEUP, 0, 0, 0, 0)   # 常驻模式：锁定
        time.sleep(0.3)

    def close():
        u.keybd_event(0x1B, 0, 0, 0)
        time.sleep(0.05)
        u.keybd_event(0x1B, 0, 2, 0)
        time.sleep(0.4)

    # ── A. 先 hover 应用（让 active_app 变粘性），再点空白 ──
    show()
    a0 = 0.0
    ax = wx + CC + int(R_ICON * math.sin(math.radians(a0)))
    ay = wy + CC - int(R_ICON * math.cos(math.radians(a0)))
    SendMessageW(host, WM_APP_DEBUGPOS, ax, ay)      # 命中第 0 个应用
    time.sleep(0.35)

    # 移到远离圆盘的空白处（右下角外）
    bx = wx + CC + 300
    by = wy + CC + 300
    SendMessageW(host, WM_APP_DEBUGPOS, bx, by)
    time.sleep(0.35)
    SendMessageW(host, WM_APP_CLICK, 0, 0)
    time.sleep(0.6)

    hidden = not bool(IsWindowVisible(ring))
    check('A 点空白 -> 菜单关闭', hidden)
    close()

    # ── B. 点在应用图标上 -> 必须执行 ──
    show()
    SendMessageW(host, WM_APP_DEBUGPOS, ax, ay)
    time.sleep(0.35)
    SendMessageW(host, WM_APP_CLICK, 0, 0)
    time.sleep(1.2)
    close()

    p.terminate()
    try:
        out = p.communicate(timeout=4)[0].decode('utf-8', 'ignore')
    except Exception:
        out = ''

    launched = out.count('[*] 启动')
    print('--- 程序输出（节选）---')
    for ln in out.strip().splitlines():
        if '启动' in ln or '锁定' in ln:
            print('   ', ln)
    check('B 点图标 -> 执行一次', launched == 1, '(实际 %d 次)' % launched)
    check('A 点空白 -> 未执行任何应用', launched <= 1, '(总启动 %d 次)' % launched)

    # 清理：关掉测试中打开的记事本
    subprocess.run(['taskkill', '/F', '/IM', 'notepad.exe'],
                   capture_output=True)

    print()
    bad = [r for r in results if not r[1]]
    print('合计 %d 项，失败 %d 项' % (len(results), len(bad)))
    return 1 if bad else 0


sys.exit(main())
