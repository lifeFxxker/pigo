# -*- coding: utf-8 -*-
"""「已打开 → 直接展示」回归。

场景（用一个确定存在的经典 Win32 程序 charmap.exe 当靶子）：
  ① 没在跑 → 单击 = 正常启动（进程数 0 → 1）。
  ② 已在跑 → 单击 = **不再重复启动**，而是把已开窗口还原 + 置前
     （进程数不变；把窗口先最小化，再点，应该被还原并成为前台窗口）。

驱动力：`WM_APP_EDITSIM` phase 9（只设 active_app，不进编辑态）+ `WM_APP_CLICK`（走 commit）。
"""
import ctypes
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_focus'))
EXE_SRC = os.path.join(HERE, os.environ.get('RL_EXE', 'Pigo.exe'))
TARGET = r'C:\Windows\System32\charmap.exe'

WM_APP = 0x8000
WM_APP_SHOW = WM_APP + 1
WM_APP_CLICK = WM_APP + 10
WM_APP_EDITSIM = WM_APP + 17

PASS = FAIL = 0
u = ctypes.windll.user32
k = ctypes.windll.kernel32


def check(name, ok, extra=''):
    global PASS, FAIL
    if ok:
        PASS += 1
    else:
        FAIL += 1
    print('%s  %-48s %s' % ('PASS' if ok else 'FAIL', name, extra), flush=True)


def charmap_pids():
    out = subprocess.run(['tasklist', '/FI', 'IMAGENAME eq charmap.exe', '/FO', 'CSV', '/NH'],
                         capture_output=True, text=True, errors='replace').stdout
    pids = []
    for ln in out.splitlines():
        parts = [p.strip('"') for p in ln.split('","')]
        if len(parts) >= 2 and parts[0].lower() == 'charmap.exe':
            pids.append(int(parts[1]))
    return pids


def enum_windows():
    res = []

    @ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def cb(hwnd, lp):
        res.append(hwnd)
        return True
    u.EnumWindows(cb, 0)
    return res


def win_pid(hwnd):
    pid = w.DWORD()
    u.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def titled_windows_of(pids):
    out = []
    for h in enum_windows():
        if win_pid(h) in pids and u.IsWindowVisible(h) and u.GetWindowTextLengthW(h) > 0:
            out.append(h)
    return out


def foreground():
    fg = u.GetForegroundWindow()
    return fg, win_pid(fg)


def main():
    if os.path.isdir(SANDBOX):
        shutil.rmtree(SANDBOX, ignore_errors=True)
    os.makedirs(SANDBOX)
    shutil.copy(EXE_SRC, os.path.join(SANDBOX, 'Pigo.exe'))
    # ⚠️ `WM_APP_EDITSIM` phase 9 只在「该应用有条目」时才设 active_app/hover_chip
    #    （源码里的守卫），所以这里必须给应用挂一个条目；commit 会启动这个条目。
    with open(os.path.join(SANDBOX, 'settings.ini'), 'w', encoding='utf-8') as f:
        f.write('[设置]\ndark=0\npersistent=1\nlistening=0\nkey_middle=1\n\n'
                '[apps]\n字符映射表=%s\n第二个=%s\n\n[app.字符映射表]\n打开=%s\n'
                % (TARGET, TARGET, TARGET))

    # 先清干净
    subprocess.run(['taskkill', '/F', '/IM', 'charmap.exe'], capture_output=True)
    time.sleep(0.6)
    check('①前置：charmap 未运行', len(charmap_pids()) == 0)

    env = dict(os.environ)
    env['RL_INSTANCE'] = 'foctest'
    env['PIGO_CONFIG_DIR'] = SANDBOX
    proc = subprocess.Popen([os.path.join(SANDBOX, 'Pigo.exe')], cwd=SANDBOX, env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    import threading
    out = []
    threading.Thread(target=lambda: [out.append(l.decode('utf-8', 'replace'))
                                     for l in proc.stdout], daemon=True).start()
    time.sleep(3.5)

    u.FindWindowW.restype = w.HWND
    host = u.FindWindowW('RLHost', None)
    SM = u.SendMessageW
    SM.restype = ctypes.c_ssize_t
    SM.argtypes = [w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]

    def click_app0():
        SM(host, WM_APP_SHOW, 0, 0)
        time.sleep(0.7)
        SM(host, WM_APP_EDITSIM, 9, 1000 + 0)   # active_app = 0（不进编辑态）
        time.sleep(0.25)
        SM(host, WM_APP_CLICK, 0, 0)
        time.sleep(2.5)

    # ── ① 没在跑 → 正常启动 ──
    click_app0()
    pids = charmap_pids()
    check('①没在跑时单击 → 正常启动（进程 0 → 1）', len(pids) == 1,
          'pids=%s' % pids)

    wins = titled_windows_of(pids)
    check('①启动后有可见带标题窗口', len(wins) >= 1, 'n=%d' % len(wins))

    # ── ② 已在跑 + 窗口最小化 → 单击应该还原 + 置前，且**不新开** ──
    if wins:
        hw = wins[0]
        u.ShowWindow(hw, 6)                     # SW_MINIMIZE
        time.sleep(0.6)
        check('②前置：窗口已最小化', bool(u.IsIconic(hw)))
        fg_before, _ = foreground()
        click_app0()
        pids2 = charmap_pids()
        check('②已在跑时单击 → **不重复启动**（进程数不变）', len(pids2) == 1,
              'pids=%s（原 %s）' % (pids2, pids))
        check('②窗口被还原（不再最小化）', not bool(u.IsIconic(hw)))
        fg_after, fg_pid = foreground()
        check('②已开窗口成为前台', fg_pid == pids2[0],
              'fg pid=%s, charmap pid=%s, fg changed=%s' % (fg_pid, pids2[0], fg_after != fg_before))
    else:
        check('②窗口最小化/还原', False, '没有拿到 charmap 窗口，跳过')

    proc.terminate()
    time.sleep(0.4)
    subprocess.run(['taskkill', '/F', '/IM', 'charmap.exe'], capture_output=True)
    marks = [l.strip() for l in out if '[*]' in l]
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
