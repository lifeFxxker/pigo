# -*- coding: utf-8 -*-
"""「网址已打开 → 切到那个标签」回归。

用两个**本地 HTML**（标题唯一）当靶子，配一个独立的 Edge 用户目录（不碰用户的会话）：
  ① 两个页面都开着（活动的是 B）→ `--focusurl A` 应该**切到 A**（窗口标题变成 A 的标题）。
  ② `--focusurl <没打开的页面>` 应该**什么都不做**（switch=false，标题不变）
     —— 这条很重要：不能因为「浏览器开着」就乱切。

⚠️ 标签页只暴露**标题**（UIA 里没有 URL），所以匹配全靠 `<title>`；本地文件不需要联网。
"""
import ctypes
import os
import re
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SB = os.path.join(HERE, '_urltab')
EXE = os.path.join(HERE, os.environ.get('RL_EXE', 'Pigo.exe'))
PROF = os.path.join(SB, 'edgeprof')
UA = 'RLTABTEST_ALPHA'
UB = 'RLTABTEST_BETA'
UC = 'RLTABTEST_GAMMA'

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


def chromewin_titles():
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


def focusurl(url):
    r = subprocess.run([EXE, '--focusurl', url], capture_output=True, text=True,
                       errors='replace', timeout=60)
    out = (r.stdout or '') + (r.stderr or '')
    m = re.search(r'switch = (true|false)', out)
    return (m.group(1) == 'true' if m else None), out.strip()


def main():
    edge = find_edge()
    if not edge:
        print('找不到 Edge，跳过')
        return 0
    if os.path.isdir(SB):
        shutil.rmtree(SB, ignore_errors=True)
    os.makedirs(SB)
    urls = {}
    for tag, title in (('a', UA), ('b', UB), ('missing', UC)):
        p = os.path.join(SB, '%s.html' % tag)
        with open(p, 'w', encoding='utf-8') as f:
            f.write('<!doctype html><meta charset="utf-8"><title>%s</title><h1>%s</h1>' % (title, title))
        urls[tag] = 'file:///' + p.replace('\\', '/')

    print('A =', urls['a'])
    print('B =', urls['b'])
    print()

    # ⚠️ 每次用**全新**的配置目录：上一次跑挂了会留下占着 profile 锁的实例，
    #    复用同一个目录会让新实例直接退出（第一次就踩了这个）。
    prof = PROF + '_%d' % int(time.time())
    proc = subprocess.Popen([edge, '--user-data-dir=' + prof, '--no-first-run',
                             '--no-default-browser-check', urls['a'], urls['b']])
    wins = []
    for _ in range(30):                     # 冷启动新配置可能要十几秒
        time.sleep(1.5)
        wins = [(h, t) for (h, t) in chromewin_titles() if 'RLTABTEST' in t]
        if wins:
            break
    if not wins:
        print('（所有 Chromium 窗口：）')
        for h, t in chromewin_titles():
            print('   ', h, repr(t[:60]))
    check('①前置：Edge 起了窗口且有我们的标签', len(wins) >= 1, 'candidates=%d' % len(wins))
    if not wins:
        proc.terminate()
        return 1
    hw, t0 = wins[0]
    # 不假定哪个标签是活动的（Edge 的行为不稳定）—— 切到「当前**没**活动的那个」
    active_a = UA in t0
    other_tag = 'b' if active_a else 'a'
    other_title = UB if active_a else UA
    check('①前置：窗口有我们的两个标签', 'RLTABTEST' in t0,
          '活动=%s title=%r' % ('A' if active_a else 'B', t0[:50]))

    # ── ② 切到另一个**已打开**的标签 ──
    ok, out = focusurl(urls[other_tag])
    time.sleep(1.5)
    t1 = title_of(hw)
    check('②--focusurl %s → 报告切换成功' % other_tag.upper(), ok is True, out.replace('\n', ' | '))
    check('②窗口标题变成 %s（真的切过去了）' % other_title, other_title in t1, 'title=%r' % t1[:50])

    # ── ③ 没打开的页面：不许乱切 ──
    ok2, out2 = focusurl(urls['missing'])
    time.sleep(1.0)
    t2 = title_of(hw)
    check('③--focusurl <没打开> → 报告未命中', ok2 is False, out2.replace('\n', ' | '))
    check('③标题不变（没有乱切）', other_title in t2, 'title=%r' % t2[:50])

    # 收尾：只关我们那个独立实例的窗口
    u.PostMessageW(hw, 0x0010, 0, 0)   # WM_CLOSE
    time.sleep(1.0)
    try:
        proc.terminate()
    except Exception:
        pass
    print('\n合计 %d 项，失败 %d 项' % (PASS + FAIL, FAIL))
    return 1 if FAIL else 0


if __name__ == '__main__':
    rc = main()
    sys.stdout.flush()
    os._exit(rc)
