"""头像注视回归：把指针放到盘的左/右/上/下，眼睛应该跟着偏移。

判定方式完全基于像素：头像 = GrokBot 式「纯色球 + 两只药丸眼」，
**眼睛与球体颜色相反**（亮主题 深球亮眼 / 暗主题 亮球暗眼），所以
用「与球心颜色差异」来找眼睛，不要写死亮色。
半径必须收在球体以内（HUB_R≈44px），否则会把盘面的边算进来淹没信号。
"""
import ctypes, time, subprocess, sys, math, os
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
CANVAS, CC = G.CANVAS, G.CC
CURSOR = (1000, 560)
EXE_DIR = os.path.dirname(os.path.abspath(__file__))
DUMP = os.path.join(EXE_DIR, 'live_dump.bmp')
results = []


def check(name, ok, extra=''):
    results.append((name, ok))
    print('%s %s %s' % ('PASS' if ok else 'FAIL', name, extra))


def pupils():
    """返回球内「眼睛」像素质心 (x, y) 与数量（相对画布坐标）。

    判定**不能写死「亮色」** —— 深色主题下球是亮的、眼睛是暗的，写死亮色
    会把整颗球当瞳孔，测出来 Δ≈0（假 FAIL）。改成：先取球心像素作为
    「球体本色」，再找与它差异明显的像素 = 眼睛，深浅主题都成立。
    """
    im = Image.open(DUMP).convert('RGB')
    px = im.load()
    W, H = im.size
    reg = []
    for y in range(H):
        for x in range(W):
            dx, dy = x - CC, y - CC
            if dx * dx + dy * dy > 40 * 40:      # 只看球体内部（不含盘面白边）
                continue
            reg.append((x, y, px[x, y]))
    if not reg:
        return (CC, CC, 0)
    # 球体本色 = 该区域里**出现最多**的颜色。
    # 不能取球心像素：注视到极限时右侧那只眼正好滑到 x=0，球心就变成眼睛颜色了，
    # 那样后面整颗球都会被当成「眼睛」（像素数从 ~600 暴涨到 ~4500）。
    from collections import Counter
    br, bg, bb = [v * 8 for v in Counter((r // 8, g // 8, b // 8) for _, _, (r, g, b) in reg)
                  .most_common(1)[0][0]]
    sx = sy = n = 0.0
    for x, y, (r, g, b) in reg:
        if abs(r - br) + abs(g - bg) + abs(b - bb) > 120:   # 与球体本色差异明显 = 眼睛
            sx += x; sy += y; n += 1
    return (sx / n, sy / n, n) if n else (CC, CC, 0)


def main():
    # 先清场：上一个用例的实例可能还没退干净，单实例互斥会把本次启动吞掉，
    # 表现出来就是「菜单没弹 → 四张 dump 一模一样 → Δ=0」的假 FAIL
    subprocess.run([r'Pigo.exe', '--quit'], capture_output=True)
    time.sleep(1.0)
    os.environ['PIGO_CONFIG_DIR'] = os.path.dirname(os.path.abspath(__file__))
    os.environ['RL_INSTANCE'] = 'facetest'
    p = subprocess.Popen([r'Pigo.exe'],
                         creationflags=0x00000008 | 0x00000200,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2.5)
    host = FindWindowW('RLHost', None)
    if not host:
        print('FAIL: 找不到窗口')
        p.terminate()
        return 1

    u.SetCursorPos(*CURSOR)
    time.sleep(0.2)
    u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
    time.sleep(0.5)
    u.mouse_event(MIDDLEUP, 0, 0, 0, 0)   # 常驻锁定
    time.sleep(0.4)

    wx, wy = CURSOR[0] - CC, CURSOR[1] - CC
    got = {}
    for tag, ox, oy in [('left', -300, 0), ('right', 300, 0), ('up', 0, -300), ('down', 0, 300)]:
        SendMessageW(host, WM_APP_DEBUGPOS, wx + CC + ox, wy + CC + oy)
        time.sleep(0.45)                   # 让注视平滑收敛
        SendMessageW(host, WM_APP_DUMP, 0, 0)
        time.sleep(0.30)
        got[tag] = pupils()

    u.keybd_event(0x1B, 0, 0, 0); time.sleep(0.05)
    u.keybd_event(0x1B, 0, 2, 0); time.sleep(0.3)
    p.terminate()

    for k, v in got.items():
        print('  注视 %-5s -> 瞳孔质心 (%.1f, %.1f)  像素数 %d' % (k, v[0], v[1], v[2]))

    if any(v[2] < 60 for v in got.values()):
        check("瞳孔可识别", False, "(亮色像素太少)")
        return 1
    check('瞳孔可识别', True)

    dx = got['right'][0] - got['left'][0]
    dy = got['down'][1] - got['up'][1]
    check('右看时瞳孔右移', dx > 1.0, '(Δx=%.2f)' % dx)
    check('下看时瞳孔下移', dy > 1.0, '(Δy=%.2f)' % dy)

    try:
        os.remove(DUMP)
    except Exception:
        pass
    bad = [r for r in results if not r[1]]
    print()
    print('合计 %d 项，失败 %d 项' % (len(results), len(bad)))
    return 1 if bad else 0


sys.exit(main())
