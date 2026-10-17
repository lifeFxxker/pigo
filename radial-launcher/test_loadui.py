# -*- coding: utf-8 -*-
"""点网址时的「不卡住 + loading 反馈」回归（两个场景）。

背景：点网址后要联网抓标题 + UI Automation 扫标签（0.5~2 秒）。原来同步跑在 UI 线程上 →
整圈冻住 2~3 秒（用户 2026-10-09 报）。现在改成后台线程 + 菜单保持可见（宠物「忙碌」+ 该段跳动的点）。

场景 A：点完鼠标**留在**梯形里 → 应该展示 loading 并跳转。
场景 B：点完鼠标**立刻移出**梯形 → 也应该照常展示 loading 并跳转
        （这是用户报的 bug：移开就没反应 —— 延迟单击到期时用 hover 复核落点被悄悄丢弃）。

每场景验三件事：
  ① 加载期间 UI 线程仍能及时回消息（连续 ping 取最大延迟）
  ② loading 期间环窗口可见，且截到的帧确实是 loading 状态
  ③ 最终真的切过去了
"""
import ctypes
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SB = os.path.join(HERE, '_loadui')
EXE = os.path.join(HERE, os.environ.get('RL_EXE', 'Pigo.exe'))
UA = 'RLTABTEST_ALPHA'
UB = 'RLTABTEST_BETA'

WM_APP = 0x8000
WM_APP_SHOW = WM_APP + 1
WM_APP_DUMP = WM_APP + 6
WM_APP_CLICK = WM_APP + 10
WM_APP_EDITSIM = WM_APP + 17
WM_APP_DEBUGPOS = WM_APP + 7     # 测试用：设置光标 → 会触发 update_hover
WM_APP_CHIPPOS = WM_APP + 22

PASS = FAIL = 0
u = ctypes.windll.user32


def check(name, ok, extra=''):
    global PASS, FAIL
    if ok:
        PASS += 1
    else:
        FAIL += 1
    print('%s  %-50s %s' % ('PASS' if ok else 'FAIL', name, extra), flush=True)


def find_edge():
    for p in [r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe',
              r'C:\Program Files\Microsoft\Edge\Application\msedge.exe']:
        if os.path.exists(p):
            return p
    return None


def chromewins():
    out = []

    @ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def cb(h, lp):
        b = ctypes.create_unicode_buffer(64)
        u.GetClassNameW(h, b, 64)
        if b.value == 'Chrome_WidgetWin_1' and u.IsWindowVisible(h):
            t = ctypes.create_unicode_buffer(512)
            u.GetWindowTextW(h, t, 512)
            out.append((h, t.value))
        return True
    u.EnumWindows(cb, 0)
    return out


def title_of(hwnd):
    t = ctypes.create_unicode_buffer(512)
    u.GetWindowTextW(hwnd, t, 512)
    return t.value


def main():
    edge = find_edge()
    if not edge:
        print('找不到 Edge，跳过')
        return 0
    if os.path.isdir(SB):
        shutil.rmtree(SB, ignore_errors=True)
    os.makedirs(SB)
    pages = {}
    for tag, title in (('a', UA), ('b', UB)):
        p = os.path.join(SB, '%s.html' % tag)
        with open(p, 'w', encoding='utf-8') as f:
            f.write('<!doctype html><meta charset="utf-8"><title>%s</title><h1>%s</h1>' % (title, title))
        pages[tag] = 'file:///' + p.replace('\\', '/')

    shutil.copy(EXE, os.path.join(SB, 'Pigo.exe'))
    with open(os.path.join(SB, 'settings.ini'), 'w', encoding='utf-8') as f:
        f.write('[设置]\ndark=0\npersistent=1\nlistening=0\nkey_middle=1\n\n'
                '[apps]\nEdge=%s\n\n[app.Edge]\n去B=%s\n' % (edge, pages['b']))

    prof = os.path.join(SB, 'edgeprof_%d' % int(time.time()))
    eproc = subprocess.Popen([edge, '--user-data-dir=' + prof, '--no-first-run',
                              '--no-default-browser-check', pages['a'], pages['b']])
    wins = []
    for _ in range(30):
        time.sleep(1.5)
        wins = [(h, t) for (h, t) in chromewins() if 'RLTABTEST' in t]
        if wins:
            break
    if not wins:
        print('Edge 没起来，跳过')
        eproc.terminate()
        return 1
    hw, _ = wins[0]

    env = dict(os.environ)
    env['RL_INSTANCE'] = 'loadui'
    env['PIGO_CONFIG_DIR'] = SB
    proc = subprocess.Popen([os.path.join(SB, 'Pigo.exe')], cwd=SB, env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    import threading
    out = []
    threading.Thread(target=lambda: [out.append(l.decode('utf-8', 'replace'))
                                     for l in proc.stdout], daemon=True).start()
    time.sleep(3.5)
    u.FindWindowW.restype = w.HWND
    host = u.FindWindowW('RLHost', None)
    ring = u.FindWindowW('RLRing', None)
    SM = u.SendMessageW
    SM.restype = ctypes.c_ssize_t
    SM.argtypes = [w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]

    def ring_rect():
        r = w.RECT()
        u.GetWindowRect(ring, ctypes.byref(r))
        return r.left, r.top

    def scenario(name, move_out):
        # 确定性地回到「活动标签 = A」
        subprocess.run([EXE, '--focusurl', pages['a']], capture_output=True, timeout=60)
        time.sleep(1.0)
        if UA not in title_of(hw):
            print('   （警告：没能切回 A，title=%r）' % title_of(hw)[:40])

        SM(host, WM_APP_SHOW, 0, 0)
        time.sleep(0.8)
        SM(host, WM_APP_EDITSIM, 9, 1000 + 0)     # active_app=0, hover_chip=Some(0)
        time.sleep(0.3)

        n_before = len(out)
        t0 = time.time()
        SM(host, WM_APP_CLICK, 0, 0)
        if move_out:
            wxr, wyr = ring_rect()
            # 挪到环窗口之外的远处 → update_hover 会让 hover_chip 变成 None
            SM(host, WM_APP_DEBUGPOS, wxr + 900, wyr + 900)
        moved = move_out

        worst = 0.0
        dumped = False
        vis = False
        while time.time() - t0 < 2.2:
            t1 = time.time()
            SM(host, WM_APP_CHIPPOS, 0, 0)        # ping：UI 线程必须及时回
            worst = max(worst, time.time() - t1)
            # 单击有 450ms 的双击判定延迟；**不要**用固定时刻截帧（查得快就错过了）。
            # 看到「查询是否已打开」= commit 刚发生 → 立刻截帧，最稳。
            started = any('查询是否已打开' in l for l in out[n_before:])
            if not dumped and (started or time.time() - t0 > 1.2):
                vis = bool(ring) and bool(u.IsWindowVisible(ring))
                SM(host, WM_APP_DUMP, 0, 0)
                dumped = True
            time.sleep(0.02)

        loading_seen = any('loading=Some' in l for l in out[n_before:])
        ok = False
        for _ in range(16):
            time.sleep(0.5)
            if UB in title_of(hw):
                ok = True
                break

        tag = '移出梯形' if moved else '留在梯形'
        check('%s·加载期间 UI 线程最大延迟 <300ms' % tag, worst < 0.3,
              '%.0f ms' % (worst * 1000))
        check('%s·加载期间环窗口可见' % tag, vis)
        check('%s·截到的帧是 loading 状态' % tag, loading_seen)
        check('%s·最终真的切到目标页 B' % tag, ok, 'title=%r' % title_of(hw)[:44])

    scenario('A', False)
    scenario('B', True)

    bmp = os.path.join(SB, 'live_dump.bmp')
    if os.path.exists(bmp):
        subprocess.run([sys.executable, os.path.join(HERE, 'bmp2png.py'), bmp], capture_output=True)
        png = os.path.splitext(bmp)[0] + '.png'
        if os.path.exists(png):
            os.makedirs(os.path.join(HERE, 'shots'), exist_ok=True)
            shutil.move(png, os.path.join(HERE, 'shots', 'url_loading.png'))

    proc.terminate()
    u.PostMessageW(hw, 0x0010, 0, 0)
    time.sleep(0.8)
    eproc.terminate()
    marks = [l.strip() for l in out if '[*]' in l or '[dump]' in l]
    if marks:
        print('\n程序侧轨迹：')
        for m in marks:
            print('   |', m)
    print('\n合计 %d 项，失败 %d 项' % (PASS + FAIL, FAIL))
    return 1 if FAIL else 0


if __name__ == '__main__':
    rc = main()
    sys.stdout.flush()
    os._exit(rc)
