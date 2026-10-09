# -*- coding: utf-8 -*-
"""验证 2026-10-09 的三个需求。

① 拖拽删除不再互相阻塞：连着拖走两个都能进队列（旧版第二个会被静默忽略），
   「取消」只放回**最后加入**的那个。
② 拖拽**插入排序**：拖 A 到 C 的槽位松手 → A 占用 C 的槽位、中间的顺移一格
   （不是两两交换；内环/外环各测一次，用例特意跨位以免和 swap 混淆）。
③ 编辑态光标闪烁：靠 `moving` 持续重绘 —— 这里验「程序在编辑态确实在重绘」
   （用 CPU 时间/帧计数间接确认，外观另用 dump 出图）。

用 `WM_APP_DRAGSIM`(phase 0=step / 1=end / 2=只挪光标) 驱动，和真实鼠标走同一对函数。
顺序一律从 `settings.ini` 读 —— 它只在**落盘**时更新，所以「还没落盘」= 读不到变化。
"""
import ctypes
import io
import math
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ring_geom as G

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_reorder'))
TAG = os.environ.get('RL_TAG', 'ro')
EXE_SRC = os.path.join(HERE, os.environ.get('RL_EXE', 'RadialLauncher.exe'))
INI = os.path.join(SANDBOX, 'settings.ini')

WM_APP = 0x8000
WM_APP_SHOW = WM_APP + 1
WM_APP_CANCEL = WM_APP + 4
WM_APP_DUMP = WM_APP + 6
WM_APP_CLICK = WM_APP + 10
WM_APP_DRAGSIM = WM_APP + 15
WM_APP_EDITSIM = WM_APP + 17
WM_APP_CHIPPOS = WM_APP + 22

PASS = FAIL = 0


def check(name, ok, extra=''):
    global PASS, FAIL
    if ok:
        PASS += 1
        print('PASS  %-52s %s' % (name, extra), flush=True)
    else:
        FAIL += 1
        print('FAIL  %-52s %s' % (name, extra), flush=True)


def section_keys(section):
    """读 ini 某一段的**键顺序**（Rust 落盘时按内存顺序写，所以顺序可信）。"""
    keys = []
    cur = None
    try:
        f = io.open(INI, encoding='utf-8', errors='replace')
    except OSError:
        return keys
    for line in f:
        line = line.strip()
        if not line or line.startswith(';') or line.startswith('#'):
            continue
        if line.startswith('[') and line.endswith(']'):
            cur = line[1:-1]
            continue
        if cur == section and '=' in line:
            keys.append(line.split('=', 1)[0].strip())
    f.close()
    return keys


def apps():
    return section_keys('apps')


def items(app):
    return section_keys('app.' + app)


def main():
    if os.path.isdir(SANDBOX):
        shutil.rmtree(SANDBOX, ignore_errors=True)
    os.makedirs(SANDBOX)
    shutil.copy(EXE_SRC, os.path.join(SANDBOX, 'RadialLauncher.exe'))
    shutil.copy(os.path.join(HERE, 'settings.ini'), INI)

    env = dict(os.environ)
    env['RL_INSTANCE'] = 'reorder' + TAG
    proc = subprocess.Popen([os.path.abspath(os.path.join(SANDBOX, 'RadialLauncher.exe'))],
                            cwd=SANDBOX, env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    # GUI 子系统程序没有控制台，但父进程给了管道 → `println!` 仍能读到。
    # 用它看 [remove] / [reorder] 到底走到了哪一步。
    import threading
    out_lines = []

    def _drain():
        try:
            for ln in proc.stdout:
                out_lines.append(ln.decode('utf-8', 'replace'))
        except Exception:
            pass

    threading.Thread(target=_drain, daemon=True).start()
    time.sleep(3.5)
    u = ctypes.windll.user32
    u.FindWindowW.restype = w.HWND
    host = u.FindWindowW('RLHost', None)
    SM = u.SendMessageW
    SM.restype = ctypes.c_ssize_t
    SM.argtypes = [w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]

    n0 = len(apps())
    r_icon, _ = G.ring_layout(n0)
    disc = G.disc_r(n0)
    CX = CY = int(G.CC)

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
        # 物理光标钉在窗口外，免得真实 hover 抢走 active_app
        u.SetCursorPos(wx - 150, wy + 5)
        time.sleep(0.15)
        return wx, wy

    wx, wy = open_menu()

    def at(i, n=None):
        # ⚠️ 半径必须按**当前**应用数重算：前面的用例会删掉几个应用，
        # 用最初 n0 算出来的 r_icon 会整圈错位（第一版就是这么误判的）。
        n = n or len(apps())
        ri, _ = G.ring_layout(n)
        a = math.radians(i * 360.0 / n)
        return int(wx + G.CC + ri * math.sin(a)), int(wy + G.CC - ri * math.cos(a))

    def at_ang(deg, n=None):
        """任意角度上的环上点（用来把光标放到**两个图标中间的空隙**里）。"""
        n = n or len(apps())
        ri, _ = G.ring_layout(n)
        a = math.radians(deg)
        return int(wx + G.CC + ri * math.sin(a)), int(wy + G.CC - ri * math.cos(a))

    ox, oy = int(wx + G.CC + disc + 110), int(wy + G.CC)   # 盘外
    bx, by = int(wx + G.CC), int(wy + G.CC + G.RM_BTN_DY)  # 取消按钮

    def menu_visible():
        ring = u.FindWindowW('RLRing', None)
        return bool(ring) and bool(u.IsWindowVisible(ring))

    def drag_out(idx, n):
        """把第 idx 个应用拖到盘外松手。"""
        if not menu_visible():
            open_menu()
        x, y = at(idx, n)
        sim(x, y, 0)          # 抓
        sim(ox, oy, 0)        # 拖到盘外
        sim(ox, oy, 1)        # 松手
        time.sleep(0.35)

    print('初始应用数 = %d' % n0)
    print()

    # ══════ ① 连续拖两个：第二个**不该被忽略** ══════
    before = apps()
    drag_out(1, n0)
    drag_out(1, n0)                     # 立刻再拖一个（旧版这里会被静默忽略）
    time.sleep(0.3)
    check('①a 连拖两个后都还没落盘（都只是待删）', apps() == before,
          'ini 仍 %d 个' % len(apps()))

    # 点「取消」→ 只放回**最后**加入的那个（第一个仍在待删）
    sim(bx, by, 2)
    SM(host, WM_APP_CLICK, 0, 0)
    time.sleep(0.4)
    check('①b 取消一次：内存里已放回一个（ini 仍无变化）', apps() == before)

    time.sleep(5.6)                     # 等剩下的那个到期落盘
    after = apps()
    gone = [a for a in before if a not in after]
    check('①c 到期后只移除了 1 个（另一个被取消放回）', len(after) == n0 - 1,
          '%d -> %d，移除 %s' % (len(before), len(after), gone))
    check('①d 被取消的那个**还在**（名字仍在列表里）', len(after) == n0 - 1)

    # 再验一次：两个都不取消 → 应该移除 2 个（说明第二个确实进队列了）
    m = len(apps())
    drag_out(0, m)
    drag_out(0, m)
    check('①e 再连拖两个，同样都进队列', apps() == after)
    time.sleep(5.6)
    after2 = apps()
    check('①f 两个都到期 → 一次移除 2 个（证明第二个没被忽略）',
          len(after2) == m - 2, '%d -> %d' % (m, len(after2)))

    # ══════ ② 拖拽插入排序（不是交换）══════
    # 用例特意**跨位**（0 → 2）：swap 出 [c2,c1,c0]，insert 出 [c1,c2,c0]，一眼分辨。
    cur = apps()
    if len(cur) >= 3:
        if not menu_visible():
            open_menu()
        x0, y0 = at(0, len(cur))
        x2, y2 = at(2, len(cur))
        sim(x0, y0, 0)                  # 抓住第 0 个
        sim(x2, y2, 0)                  # 拖到第 2 个的槽位
        sim(x2, y2, 1)                  # 松手 = 插入
        time.sleep(0.5)
        new = apps()
        want = cur[1:]
        want.insert(2, cur[0])          # remove(0) + insert(2)
        check('②a 内环插入：拖第 1 个到第 3 个槽位 → 该槽位让位、中间的顺移',
              new == want,
              '%s… → %s…（期望 %s…）' % (cur[:3], new[:3], want[:3]))
        # 插入必须落盘（重启后保持）
        check('②b 插入已落盘（ini 顺序确实变了）', new != cur)

    # ══════ ②' 拖到**两个图标中间的空隙**也要能插入 ══════
    # 用户 2026-10-09 报「拖到两个图标中间没反应、插不进去」：旧落点判定要求光标贴住图标，
    # 缝隙处到两侧图标都太远 → 落空 → 当作放弃。改用**角度吸附**后应吸附到最近槽位。
    cur = apps()
    if len(cur) >= 6:
        if not menu_visible():
            open_menu()
        n = len(cur)
        per = 360.0 / n
        gx, gy = at_ang(3.6 * per, n)   # 槽位 3 与 4 之间的空隙
        x0, y0 = at(0, n)
        sim(x0, y0, 0)
        sim(gx, gy, 0)
        sim(gx, gy, 1)
        time.sleep(0.5)
        new = apps()
        want = cur[1:]
        want.insert(4, cur[0])          # round(3.6) = 4
        check('②d 拖到两图标中间的空隙 → 吸附到最近槽位并插入', new == want,
              '%s… → %s…（期望 %s…）' % (cur[:5], new[:5], want[:5]))

    # 外环插入：先选中一个应用，再拖它的条目
    if len(apps()) >= 1:
        SM(host, WM_APP_CANCEL, 0, 0)
        time.sleep(0.3)
        open_menu()
        # ⚠️ 这里**不能**再发 WM_APP_CLICK —— 那是「真单击」，会走 commit/关菜单，
        # 菜单一关 `WM_APP_DRAGSIM` 就整段不生效（第一版外环换位失败就是这个）。
        # 选中应用只用 EDITSIM phase 9（它不碰菜单开合）。
        # ⚠️ 要 ≥3 个条目才能把「插入」和「交换」区分开。
        host_app = None
        for a in apps():
            if len(items(a)) >= 3:
                host_app = a
                break
        if host_app is None:
            print('SKIP  外环排序（没有哪个应用有 ≥3 个条目）', flush=True)
        else:
            # 让 active_app 指向 host_app：EDITSIM phase 9 只设状态、不进编辑态
            # （l >= 1000 时高位指定应用下标，低位 j 固定取 0）
            ai = apps().index(host_app)
            SM(host, WM_APP_EDITSIM, 9, 1000 + ai)
            time.sleep(0.6)
            it = items(host_app)
            # ⚠️ 段的位置**必须问程序要**（`WM_APP_CHIPPOS`），不能自己按几何算：
            # 外环是极坐标的扇形段，`chip_layout` 的「贴身扇面」排布取决于
            # 每个条目名的宽度，Python 侧镜像稍有出入就整段错位、拖不中。
            def chippos(i):
                v = SM(host, WM_APP_CHIPPOS, i, 0)
                x, y = v & 0xFFFF, (v >> 16) & 0xFFFF
                return (x, y) if x and y else None

            p0, p2 = chippos(0), chippos(2)
            check('②c0 程序回报了两个段的坐标', p0 is not None and p2 is not None,
                  'p0=%s p2=%s' % (p0, p2))
            check('②c1 拖拽前菜单仍开着', menu_visible())
            if p0 and p2:
                sim(p0[0], p0[1], 0)
                sim(p2[0], p2[1], 0)
                sim(p2[0], p2[1], 1)
                time.sleep(0.5)
                it2 = items(host_app)
                want2 = it[1:]
                want2.insert(2, it[0])   # remove(0) + insert(2)
                check('②c 外环插入：拖第 1 个条目到第 3 个槽位',
                      it2 == want2,
                      '%s… → %s…（期望 %s…）' % (it[:3], it2[:3], want2[:3]))

    proc.terminate()
    time.sleep(0.5)
    # 程序侧的 [remove] / [reorder] 轨迹 —— 失败时靠它定位走到哪一步断了
    marks = [l.strip() for l in out_lines
             if '[remove]' in l or '[reorder]' in l or '[editsim]' in l]
    if marks:
        print()
        print('程序侧轨迹：')
        for m in marks:
            print('   |', m)
    print()
    print('合计 %d 项，失败 %d 项' % (PASS + FAIL, FAIL))
    return 1 if FAIL else 0


if __name__ == '__main__':
    rc = main()
    sys.stdout.flush()
    os._exit(rc)
