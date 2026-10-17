# -*- coding: utf-8 -*-
"""例外名单（黑白名单）回归。

呼出键走的是**全局低级钩子**，合成鼠标事件驱动不了（钩子只认真实事件），
所以这里用 `WM_APP_EXEMSIM` —— 它在 UI 线程上跑**与钩子同一条**判定
（`update_exempt` → `EXEMPT`），并回报 `fg / fullscreen / mode / n / exempt`。

覆盖（判定见 `update_exempt`，**二选一**由 `mode=black|white` 决定）：
  ① mode=black + 前台在**黑名单**里             → exempt=1（呼出键放行）
  ② mode=white + 前台在**白名单**里             → exempt=0（白名单里才正常）
  ③ mode=white + 前台**不在**白名单里           → exempt=1（白名单外一律放行）
  ④ mode=black + 黑名单里是别人                 → exempt=0
  ⑤ mode=white + **空白名单**                   → exempt=0（⚠️ 空表不能判成「全部例外」，否则启动器永远失效）
  ⑥ 全屏自动放行（开关开 + 前台全屏）           → exempt=1
  ⚠️ ①② 用**同一份**名单（target 同时在黑、白两张卡片里），只切 `mode` → 结果反转，
     专门锁「同一时刻只有一张生效」这条语义。

⚠️ 用 charmap 当「前台程序」的样本：**必须真的把它顶到前台**才能断言。
   用户正在用电脑时，Windows 前台锁会拒绝我们抢焦点 → 这时**报 SKIP**（不是 FAIL），
   避免把环境噪声当成代码回归。抢到前台的那次永远是严格断言。
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
SB = os.path.join(HERE, '_exempt')
EXE_SRC = os.path.join(HERE, os.environ.get('RL_EXE', 'Pigo.exe'))
TARGET = r'C:\Windows\System32\charmap.exe'
TARGET_NAME = 'charmap.exe'

WM_APP = 0x8000
WM_APP_EXEMSIM = WM_APP + 24

PASS = FAIL = SKIP = 0
u = ctypes.windll.user32
k = ctypes.windll.kernel32


def check(name, ok, extra=''):
    global PASS, FAIL
    if ok:
        PASS += 1
    else:
        FAIL += 1
    print('%s  %-46s %s' % ('PASS' if ok else 'FAIL', name, extra), flush=True)


def skip(name, extra=''):
    global SKIP
    SKIP += 1
    print('%s  %-46s %s' % ('SKIP', name, extra), flush=True)


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


def pids_of(name):
    out = subprocess.run(['tasklist', '/FI', 'IMAGENAME eq %s' % name, '/FO', 'CSV', '/NH'],
                         capture_output=True, text=True, errors='replace').stdout
    res = []
    for ln in out.splitlines():
        parts = [p.strip('"') for p in ln.split('","')]
        if len(parts) >= 2 and parts[0].lower() == name.lower():
            res.append(int(parts[1]))
    return res


def top_window_of(pid):
    for h in enum_windows():
        if win_pid(h) == pid and u.IsWindowVisible(h) and u.GetWindowTextLengthW(h) > 0:
            if not u.GetWindow(h, 4):   # GW_OWNER
                return h
    return None


def force_foreground(hwnd):
    """把窗口顶到最前（绕过前台锁的经典做法）。返回是否成功。"""
    try:
        u.AllowSetForegroundWindow(0xFFFFFFFF)   # ASFW_ANY
    except Exception:
        pass
    fg = u.GetForegroundWindow()
    ft = u.GetWindowThreadProcessId(fg, None)
    mt = k.GetCurrentThreadId()
    attached = ft and ft != mt and u.AttachThreadInput(mt, ft, True)
    try:
        u.ShowWindow(hwnd, 9)          # SW_RESTORE
        u.BringWindowToTop(hwnd)
        u.SetForegroundWindow(hwnd)
        if u.GetForegroundWindow() != hwnd:
            u.SwitchToThisWindow(hwnd, True)
    finally:
        if attached:
            u.AttachThreadInput(mt, ft, False)
    return u.GetForegroundWindow() == hwnd


def write_ini(mode, black, white, fs=0):
    """两张卡片都写进配置，但 `mode=` 决定**哪一张**生效（0=black / 1=white）。"""
    with open(os.path.join(SB, 'settings.ini'), 'w', encoding='utf-8') as f:
        f.write('[设置]\r\ndark=0\r\npersistent=1\r\nlistening=0\r\n'
                'key_middle=1\r\nkey_side=1\r\nkey_rhold=1\r\n\r\n[apps]\r\n终端=cmd.exe\r\n\r\n')
        f.write('[例外]\r\nmode=%s\r\nfullscreen=%d\r\n' % (mode, fs))
        for n in black:
            f.write('b=%s\r\n' % n)
        for n in white:
            f.write('w=%s\r\n' % n)


def run_case(tag, mode, black, white, fs, target_pid, expect_exempt):
    """把 charmap 顶到前台后驱动判定。抢不到前台 → SKIP（不计失败）。"""
    write_ini(mode, black, white, fs)
    env = dict(os.environ)
    env['RL_INSTANCE'] = 'exc_%s' % tag
    env['PIGO_CONFIG_DIR'] = SB
    p = subprocess.Popen([os.path.join(SB, 'Pigo.exe')], cwd=SB, env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    import threading
    out = []
    threading.Thread(target=lambda: [out.append(l.decode('utf-8', 'replace'))
                                     for l in p.stdout], daemon=True).start()
    time.sleep(2.5)
    u.FindWindowW.restype = w.HWND
    host = u.FindWindowW('RLHost', None)
    if not host:
        check('%s·拿到宿主窗口' % tag, False)
        p.terminate()
        return

    # 目标窗口可能被别的东西盖住/重开 → 每次重新取一次；取不到就 SKIP
    tgt = top_window_of(target_pid)
    focused = False
    if tgt:
        for _ in range(6):
            if force_foreground(tgt):
                focused = True
                break
            time.sleep(0.25)
    if not focused:
        skip('%s·前台=目标程序' % tag,
             '抢不到前台（你在用电脑时正常）→ 跳过断言 fg_pid=%s target=%s'
             % (win_pid(u.GetForegroundWindow()), win_pid(tgt) if tgt else 'None'))
        skip('%s·exempt 判定' % tag, '（前台不可控，无断言）')
        p.terminate()
        time.sleep(0.4)
        return

    time.sleep(0.9)                      # 等 UI 线程定时刷新 + 前台生效
    got_fg = win_pid(u.GetForegroundWindow())

    SM = u.SendMessageW
    SM.restype = ctypes.c_ssize_t
    SM.argtypes = [w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t]
    SM(host, WM_APP_EXEMSIM, 0, 0)
    time.sleep(0.4)
    line = next((l for l in reversed(out) if '[exemsim]' in l), '')
    m = re.search(r'exempt=(\d)', line)
    got = (m.group(1) == '1') if m else None
    check('%s·前台确为目标程序' % tag, got_fg == win_pid(tgt),
          'fg_pid=%s target_pid=%s' % (got_fg, win_pid(tgt)))
    check('%s·exempt 判定正确' % tag, got is expect_exempt,
          '%s  %s' % (line.strip()[:78], '(期望 %d)' % int(expect_exempt)))
    p.terminate()
    time.sleep(0.4)


def main():
    if os.path.isdir(SB):
        shutil.rmtree(SB, ignore_errors=True)
    os.makedirs(SB)
    shutil.copy(EXE_SRC, os.path.join(SB, 'Pigo.exe'))

    subprocess.run(['taskkill', '/F', '/IM', 'charmap.exe'], capture_output=True)
    time.sleep(0.5)
    subprocess.Popen([TARGET])
    time.sleep(3)
    pids = pids_of(TARGET_NAME)
    if not pids:
        print('起不来 charmap，跳过')
        return 0
    tpid = pids[0]
    hw = top_window_of(tpid)
    check('前置：拿到 charmap 窗口', hw is not None)
    if not hw:
        return 1
    print()

    run_case('a', 'black', [TARGET_NAME], [TARGET_NAME], 0, tpid, True)   # ① 同名单·黑名单模式 → 放行
    run_case('b', 'white', [TARGET_NAME], [TARGET_NAME], 0, tpid, False)  # ② 同名单·白名单模式 → 正常（结果反转＝只有一张生效）
    run_case('c', 'white', [TARGET_NAME], ['other.exe'], 0, tpid, True)   # ③ 白名单模式·不在名单 → 放行
    run_case('d', 'black', ['other.exe'], [], 0, tpid, False)             # ④ 黑名单模式·名单里是别人 → 正常
    run_case('e', 'white', [TARGET_NAME], [], 0, tpid, False)             # ⑤ 空白名单 → **不能**判成全例外

    # ⑤ 全屏自动放行：用 tkinter 开一个真全屏窗口（它自己的进程名就是 fg）
    full_ok = False
    try:
        code = ('import tkinter as tk\n'
                'r=tk.Tk()\nr.attributes("-fullscreen",True)\nr.title("RLFULLTEST")\n'
                'r.after(14000, r.destroy)\nr.mainloop()\n')
        fp = subprocess.Popen([sys.executable, '-c', code])
        time.sleep(3)
        fw = None
        for h in enum_windows():
            t = ctypes.create_unicode_buffer(256)
            u.GetWindowTextW(h, t, 256)
            if t.value == 'RLFULLTEST':
                fw = h
                break
        if fw:
            for _ in range(6):
                if force_foreground(fw):
                    break
                time.sleep(0.25)
            if u.GetForegroundWindow() == fw:
                run_case('f', 'black', [TARGET_NAME], [], 1, win_pid(fw), True)
                full_ok = True
            else:
                skip('e·全屏自动放行', '抢不到全屏窗口前台 → 跳过')
        else:
            print('SKIP  全屏场景（没拿到全屏窗口）')
        fp.terminate()
    except Exception as e:
        print('SKIP  全屏场景（%s）' % e)
    _ = full_ok

    subprocess.run(['taskkill', '/F', '/IM', 'charmap.exe'], capture_output=True)
    print('\n合计 %d 项，失败 %d 项，跳过 %d 项' % (PASS + FAIL, FAIL, SKIP))
    return 1 if FAIL else 0


if __name__ == '__main__':
    rc = main()
    sys.stdout.flush()
    os._exit(rc)
