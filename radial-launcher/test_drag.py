# -*- coding: utf-8 -*-
"""拖出移除的回归：全程用 SendMessage 驱动，**不依赖真实鼠标**。

低级钩子只认真实鼠标事件（合成事件会被放行），所以这里走诊断消息
`WM_APP_DRAGSIM` —— 它和真实消息调用的是同一对函数（`drag_step` / `drag_end`）。

跑法：python test_drag.py
"""
import ctypes
import math
import os
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_dragtest'))
# 沙箱里的 exe 名。⚠️ 别硬编码 RadialLauncher.exe —— 那个文件常被僵尸实例占着，
# 复制会失败。测试统一用**正式名**（沙箱里放副本，别碰项目根目录那个）。
EXE_NAME = os.environ.get('RL_EXE', 'RadialLauncher.exe')
WM_APP = 0x8000
WM_APP_SHOW = WM_APP + 1
WM_APP_CANCEL = WM_APP + 4
WM_APP_DUMP = WM_APP + 6
WM_APP_KEY = WM_APP + 9
WM_APP_CLICK = WM_APP + 10
WM_APP_DRAGSIM = WM_APP + 15

sys.path.insert(0, HERE)
import ring_geom as G  # noqa: E402

PASS = [0]
FAIL = [0]


def check(name, ok, extra=''):
    if ok:
        PASS[0] += 1
    else:
        FAIL[0] += 1
    print('%s  %-44s %s' % ('PASS' if ok else 'FAIL', name, extra), flush=True)


def read_apps():
    return G.read_settings(os.path.join(SANDBOX, 'settings.ini'))[0]


def count_items(app_name):
    """读 settings.ini 里该应用的条目数（胶囊增删的落盘断言用）"""
    return len(G.read_settings(os.path.join(SANDBOX, 'settings.ini'))[1].get(app_name, []))


def snap(host, out):
    """让程序把当前帧写成 live_dump.bmp，再转 PNG（看清表情/按钮）"""
    ctypes.windll.user32.SendMessageW(host, WM_APP_DUMP, 0, 0)
    time.sleep(0.45)
    bmp = os.path.join(SANDBOX, 'live_dump.bmp')
    if not os.path.exists(bmp):
        bmp = os.path.join(HERE, 'live_dump.bmp')
    # bmp2png.py 只接受输入文件，输出固定写成 <同名>.png
    subprocess.run([sys.executable, os.path.join(HERE, 'bmp2png.py'), bmp],
                   capture_output=True)
    png = os.path.splitext(bmp)[0] + '.png'
    if os.path.exists(png):
        import shutil
        shutil.move(png, os.path.join(HERE, 'shots', out))


def seed_chip_if_needed():
    """沙盒里没有任何外环胶囊时，给第一个应用种一条（⑦ 会消耗）。
    必须在实例启动前做 —— ⑤ 的落盘会用启动时加载的内存态重写 ini。"""
    ini = os.path.join(SANDBOX, 'settings.ini')
    apps, items = G.read_settings(ini)
    if any(items.get(a) for a in apps) or not apps:
        return
    with open(ini, 'a', encoding='utf-8') as f:
        f.write('\n[app.%s]\n测试胶囊=ms-settings:about\n' % apps[0])
    print('[seed] 给 %s 种了一条测试胶囊' % apps[0], flush=True)


def main():
    subprocess.run([os.path.join(SANDBOX, EXE_NAME), '--quit'],
                   cwd=SANDBOX, capture_output=True)
    time.sleep(1.2)
    seed_chip_if_needed()
    env = dict(os.environ)
    env['RL_INSTANCE'] = os.environ.get('RL_TAG', 'd2')
    p = subprocess.Popen([os.path.join(SANDBOX, EXE_NAME)], cwd=SANDBOX,
                         env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2.5)
    u = ctypes.windll.user32
    u.FindWindowW.restype = w.HWND
    host = u.FindWindowW('RLHost', None)
    if not host:
        print('FAIL  找不到 RLHost', flush=True)
        return 1
    SM = ctypes.windll.user32.SendMessageW
    SM.restype = ctypes.c_ssize_t
    SM.argtypes = [w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]
    u.GetWindowRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
    u.IsWindowVisible.argtypes = [w.HWND]

    CX, CY = 900, 520
    n0 = len(read_apps())
    r_icon, icon = G.ring_layout(n0)
    disc = G.disc_r(n0)

    def sim(x, y, phase):
        SM(host, WM_APP_DRAGSIM, (int(y) << 16) | int(x), phase)

    def open_menu():
        sim(CX, CY, 2)
        SM(host, WM_APP_SHOW, 0, 0)
        time.sleep(0.7)
        ring = u.FindWindowW('RLRing', None)
        rect = w.RECT()
        u.GetWindowRect(ring, ctypes.byref(rect))
        wx, wy = rect.left, rect.top
        # 把物理光标钉在窗口外：它一进盘面，真实 hover 就会把 active_app
        # 抢到光标下的应用上，后面按「导航选中的应用」算的坐标全都会落空
        u.SetCursorPos(wx - 150, wy + 5)
        time.sleep(0.15)
        return wx, wy

    def ensure_menu():
        ring = u.FindWindowW('RLRing', None)
        if ring and u.IsWindowVisible(ring):
            return
        return open_menu()

    wx, wy = open_menu()

    def at(i):
        a = math.radians(i * 360.0 / n0)
        return int(wx + G.CC + r_icon * math.sin(a)), int(wy + G.CC - r_icon * math.cos(a))

    ix, iy = at(1)
    ox, oy = int(wx + G.CC + disc + 90), int(wy + G.CC)

    # ① 盘内拖动后松手 = 放弃
    ensure_menu()
    sim(ix, iy, 0)
    sim(ix + 18, iy + 9, 0)
    sim(ix + 18, iy + 9, 1)
    time.sleep(0.4)
    check('盘内拖动松手 = 放弃（不进倒计时）', len(read_apps()) == n0)

    # ② 拖到盘外 = 紧张表情
    ensure_menu()
    sim(ix, iy, 0)
    sim(ox, oy, 0)
    time.sleep(0.5)
    snap(host, 'live_drag_nervous.png')

    # ③ 盘外松手 = 生气 + 取消按钮 + 倒计时（此刻还没移除）
    sim(ox, oy, 1)
    time.sleep(0.4)
    snap(host, 'live_drag_pending.png')
    check('盘外松手 → 进入倒计时（尚未移除）', len(read_apps()) == n0)

    # ④ 点「取消」→ 不移除
    bx, by = int(wx + G.CC), int(wy + G.CC + G.RM_BTN_DY)
    sim(bx, by, 2)
    time.sleep(0.2)
    SM(host, WM_APP_CLICK, 0, 0)
    time.sleep(0.4)
    check('倒计时期间点「取消」→ 已取消', len(read_apps()) == n0)
    time.sleep(5.5)
    check('取消后 5 秒过去仍不移除', len(read_apps()) == n0)
    snap(host, 'live_drag_cancelled.png')

    # ⑤ 不点取消 → 5 秒后真的移除并落盘
    wx, wy = ensure_menu() or (wx, wy)
    ix, iy = at(1)
    ox, oy = int(wx + G.CC + disc + 90), int(wy + G.CC)
    sim(ix, iy, 0)
    sim(ox, oy, 1)
    time.sleep(0.6)
    snap(host, 'live_drag_count.png')
    time.sleep(5.6)
    apps = read_apps()
    check('5 秒到 → 真的移除并落盘', len(apps) == n0 - 1, '%d -> %d' % (n0, len(apps)))
    check('留了 settings.ini.bak（无 git 的唯一回滚点）',
          os.path.exists(os.path.join(SANDBOX, 'settings.ini.bak')))

    # ⑥ 外环胶囊拖出窗口 = 请求移除（立即消失），点「取消」放回。
    #    胶囊只属于「当前亮着的应用」，所以先收起菜单重开（active_app 归零），
    #    再用键盘导航选中**第一个带胶囊的应用**（上一轮的 ⑦ 会消耗掉一条）。
    SM(host, WM_APP_CANCEL, 0, 0)
    time.sleep(0.5)
    wx, wy = open_menu()
    apps0, items0 = G.read_settings(os.path.join(SANDBOX, 'settings.ini'))
    ci = next((i for i, nm in enumerate(apps0) if items0.get(nm)), None)
    if ci is None:
        check('找到带胶囊的应用（可测）', False, '沙盒里没有任何外环胶囊')
    else:
        for _ in range(ci + 1):
            SM(host, WM_APP_KEY, 4, 0)  # KA_NEXT → nav_app（None 起步第一下就到 0）
            time.sleep(0.15)
        time.sleep(0.6)
        owner = apps0[ci]
        chip_names = items0[owner]
        m0 = len(chip_names)
        check('找到带胶囊的应用（可测）', True, '%s: %d 条' % (owner, m0))
        cl = G.chip_layout(chip_names, ci * 360.0 / len(apps0), len(apps0))
        ang, r = cl[0]
        a = math.radians(ang)
        cx, cy = int(wx + G.CC + r * math.sin(a)), int(wy + G.CC - r * math.cos(a))
        ox2 = wx - 200  # 窗口外
        sim(cx, cy, 0)
        sim(wx + G.CC, wy + G.CC, 0)   # 掠过盘内（疑惑）
        sim(ox2, wy + G.CC, 0)         # 拖出窗口（紧张）
        sim(ox2, wy + G.CC, 1)
        time.sleep(0.5)
        snap(host, 'live_chip_pending.png')
        check('胶囊拖出窗口 → 进入倒计时（尚未落盘）', count_items(owner) == m0)
        bx, by = int(wx + G.CC), int(wy + G.CC + G.RM_BTN_DY)
        sim(bx, by, 2)
        time.sleep(0.2)
        SM(host, WM_APP_CLICK, 0, 0)
        time.sleep(0.4)
        check('胶囊点「取消」→ 放回', count_items(owner) == m0)
        snap(host, 'live_chip_cancelled.png')

        # ⑦ 胶囊不取消 → 5 秒后落盘。等待期若混入真实输入（用户正在用机器）
        # 会关掉菜单、取消倒计时 —— 计数没变就重开菜单重试，最多 3 次。
        removed = False
        for _ in range(3):
            ring2 = u.FindWindowW('RLRing', None)
            if not (ring2 and u.IsWindowVisible(ring2)):
                wx, wy = open_menu()
                for _ in range(ci + 1):
                    SM(host, WM_APP_KEY, 4, 0)  # KA_NEXT → nav_app
                    time.sleep(0.15)
                time.sleep(0.5)
            sim(cx, cy, 0)
            sim(ox2, wy + G.CC, 0)
            sim(ox2, wy + G.CC, 1)
            t_end = time.time() + 5.6
            while time.time() < t_end:
                if count_items(owner) == m0 - 1:
                    break
                time.sleep(0.4)
            if count_items(owner) == m0 - 1:
                removed = True
                break
        check('胶囊 5 秒到 → 真的移除并落盘', removed,
              '%d -> %d' % (m0, count_items(owner)))
        snap(host, 'live_chip_removed.png')

    SM(host, WM_APP_CANCEL, 0, 0)
    time.sleep(0.4)
    p.terminate()
    print('\n合计 %d 项，失败 %d 项' % (PASS[0] + FAIL[0], FAIL[0]), flush=True)
    return 1 if FAIL[0] else 0


if __name__ == '__main__':
    sys.exit(main())
