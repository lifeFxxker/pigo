# -*- coding: utf-8 -*-
"""验证「右键长按呼出」的两个 bug 都修好了 —— 顺带**测量 UI 线程有没有被卡住**。

用户报的现场：开机自启 → 呼出一次轮盘 → 关掉 → **右键点别处不生效** →
5~6 秒后全系统鼠标卡顿 → 再过 5~6 秒才恢复。

两个根因：
  A. **吞了 UP 却没吞配对的 DOWN**（或反之）→ 应用收到 RIGHTDOWN 收不到 RIGHTUP
     → 它以为右键一直按着 → 「右键点别处不生效」。不变式：**没吞过 DOWN 的 UP 必须放行**。
  B. **在鼠标钩子回调里 `mouse_event` 注入输入** → 注入事件要排进输入队列，
     而队列正卡在这个钩子回调上 → 自锁 → 钩子回调 1000ms、鼠标卡 5~6 秒。

**这个测试怎么做（踩过两个坑）**：
  · 不能注入真实鼠标事件来测 —— 测试进程注入 + 目标进程装着全局钩子，
    两边互相卡死（实测整个测试挂死，连 SIGTERM 都杀不掉）。
    改走 `WM_APP_RHOLDSIM` 直接喂事件码。
  · 不能用 `SendMessageW` —— 目标 UI 线程一旦被卡住，测试就永久挂起、看不到结果。
    改用 `SendMessageTimeoutW`：**它既是调用也是探针** ——
    一次调用耗时突然涨到几百毫秒，本身就是「UI 线程被卡」的实锤。
"""
import ctypes
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_rholdtest'))
# ⚠️ 用**新**构建产物：项目根的 `Pigo.exe` 可能被一个卡死的僵尸壳占着
# （钩子线程卡在 win32k → 文件锁住、也换不掉），拿它测会测到旧版本。
EXE_SRC = os.path.join(HERE, os.environ.get('RL_EXE', 'Pigo.exe'))
EXE = os.path.join(SANDBOX, 'Pigo.exe')

u = ctypes.windll.user32
u.FindWindowW.restype = w.HWND

WM_APP_SHOW = 0x8000 + 1
WM_APP_RHOLDSIM = 0x8000 + 19
RHOLD_MS = 380
SMTO_ABORTIFHUNG = 0x0002
STALL_MS = 250          # 单次调用超过这个值 = UI 线程被卡（正常应该 <20ms）

results = []
worst_ms = [0.0]


def check(name, ok, detail=''):
    results.append((name, bool(ok)))
    print('%-46s %s %s' % (name, 'PASS' if ok else 'FAIL', detail), flush=True)


def decode(v):
    return {
        'consumed': v & 1,
        'down_pending': (v >> 1) & 1,
        'in_menu': (v >> 2) & 1,
        'reinjects': (v >> 8) & 0xff,
        'downs_seen': (v >> 16) & 0xff,
        'magic_seen': (v >> 24) & 0x7f,   # 钩子见到几个「带我们标记」的事件
        'inject_done': (v >> 31) & 0x7f,  # 补发实际执行完的次数
        'menu_open': (v >> 5) & 1,        # MENU_OPEN 当前值
    }


def main():
    if os.path.isdir(SANDBOX):
        shutil.rmtree(SANDBOX, ignore_errors=True)
    os.makedirs(SANDBOX, exist_ok=True)
    shutil.copy(EXE_SRC, EXE)
    NL = '\r\n'
    with open(os.path.join(SANDBOX, 'settings.ini'), 'w', encoding='utf-8') as f:
        f.write('[设置]' + NL + 'dark=0' + NL + 'persistent=0' + NL
                + 'listening=1' + NL + 'key_middle=1' + NL + 'key_side=1' + NL
                + 'key_rhold=1' + NL + NL)
        f.write('[apps]' + NL + '记事本=' + os.path.join(
            os.environ.get('SystemRoot', r'C:\Windows'), 'System32', 'notepad.exe') + NL)

    env = dict(os.environ)
    env['RL_INSTANCE'] = 'rhold' + os.environ.get('RL_TAG', '1')
    env['PIGO_CONFIG_DIR'] = SANDBOX
    proc = subprocess.Popen([os.path.abspath(EXE)], cwd=SANDBOX, env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(3.0)
    host = u.FindWindowW('RLHost', None)
    if not host:
        print('FAIL: 找不到 RLHost，程序没起来')
        proc.kill()
        return 1

    u.SendMessageTimeoutW.restype = ctypes.c_ssize_t
    u.SendMessageTimeoutW.argtypes = [
        w.HWND, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t,
        ctypes.c_uint, ctypes.c_uint, ctypes.POINTER(ctypes.c_ssize_t)]

    def sim(ev, ms=4000):
        """喂一个事件码（0=DOWN 1=UP 2=查询），返回 (状态, 是否响应, 耗时秒)。"""
        res = ctypes.c_ssize_t(0)
        t0 = time.time()
        ok = u.SendMessageTimeoutW(host, WM_APP_RHOLDSIM, ev, 0,
                                   SMTO_ABORTIFHUNG, ms, ctypes.byref(res))
        dt = time.time() - t0
        worst_ms[0] = max(worst_ms[0], dt * 1000)
        return decode(res.value if ok else 0), bool(ok), dt

    def ring_visible():
        h = u.FindWindowW('RLRing', None)
        return bool(h) and bool(u.IsWindowVisible(h))

    # ── ① 没有配对的 DOWN 时，UP 必须放行（不能吞）──
    s, ok, dt = sim(1)
    check('① 孤立 UP 不被吞（吞了应用会以为右键永远按着）',
          ok and s['consumed'] == 0 and s['down_pending'] == 0,
          'consumed=%d %.0fms' % (s['consumed'], dt * 1000))

    # ── ② 菜单没开时短按右键：**两个都不吞**（= 一次完全正常的右键）──
    base = sim(2)[0]
    d, _, _ = sim(0)
    up, ok, dt = sim(1)                 # 立刻 UP → 短按
    check('②a 短按：DOWN 放行（应用正常收到右键按下）', d['consumed'] == 0,
          'consumed=%d' % d['consumed'])
    check('②b 短按：UP 放行（配对完整，不会被卡住）', up['consumed'] == 0,
          'consumed=%d' % up['consumed'])
    check('②c 短按：DOWN 记账已结清（R_DOWN_AT 归零）', up['down_pending'] == 0)
    check('②d 短按：没有注入任何鼠标事件（inject_done 仍为 0）',
          up['inject_done'] == 0, 'inject_done=%d' % up['inject_done'])
    check('②e 短按后 UI 线程没被卡（Bug B：注入输入）',
          ok and dt * 1000 < STALL_MS, '后续调用 %.0fms' % (dt * 1000))
    check('②f 全程没有鼠标注入（magic_seen 仍为 0）—— 这是本次修复的核心',
          up['magic_seen'] == 0, 'magic_seen=%d' % up['magic_seen'])

    # ── ③ 长按右键（≥RHOLD_MS）→ 呼出菜单；DOWN/UP 同样放行 ──
    sim(0)
    time.sleep((RHOLD_MS + 150) / 1000.0)
    up, ok, dt = sim(1)
    time.sleep(0.5)
    check('③a 长按：菜单被呼出', ring_visible())
    check('③b 长按：UP 仍是放行（不吞，不留孤儿）', up['consumed'] == 0,
          'consumed=%d' % up['consumed'])
    check('③c 长按：依然没有注入鼠标事件', up['inject_done'] == 0,
          'inject_done=%d' % up['inject_done'])

    # ── ④ 菜单开着时右键（= 关菜单）：这一对 DOWN/UP **都吞**（成对，不留孤儿）──
    if not ring_visible():
        u.SendMessageTimeoutW(host, WM_APP_SHOW, 0, 0, SMTO_ABORTIFHUNG, 2000,
                              ctypes.byref(ctypes.c_ssize_t(0)))
        time.sleep(0.7)
    d, _, _ = sim(0)                      # 菜单开着 → 这条 down 触发收起
    check('④a 关菜单：DOWN 被吞', d['consumed'] == 1)
    # 等 MENU_OPEN 真的归零，并记下时间线 —— 如果它卡在 1 就是真 bug
    tl = []
    for _ in range(20):
        time.sleep(0.2)
        v = sim(2)[0]['menu_open']
        tl.append(v)
        if v == 0:
            break
    check('④b MENU_OPEN 归零（关菜单真的生效）', tl[-1] == 0, '时间线 %s' % tl)
    up, ok, dt = sim(1)
    check('④c 关菜单：UP 被吞（配对完整）', up['consumed'] == 1)
    check('④d 关菜单：DOWN 欠账已结清', up['down_pending'] == 0)
    check('④e 窗口也收起了', not ring_visible())

    # ── ⑤ 关完之后右键必须恢复正常（用户报的就是这个）──
    # 先确认菜单确实收干净了（MENU_OPEN=0），再验右键行为 ——
    # 否则测的是「菜单还开着」那条分支，不是「关完之后」。
    for _ in range(20):
        if sim(2)[0]['menu_open'] == 0:
            break
        time.sleep(0.2)
    sim(0)
    up, ok, dt = sim(1)
    check('⑤ 关完菜单后右键恢复完全正常（放行 + 记账结清 + 无注入）',
          up['consumed'] == 0 and up['down_pending'] == 0
          and up['inject_done'] == 0,
          'consumed=%d pending=%d inject=%d menu_open=%d'
          % (up['consumed'], up['down_pending'], up['inject_done'], up['menu_open']))

    # ── ⑥ 全程没有钩子回调慢记录 ──
    proc.terminate()
    time.sleep(0.7)
    try:
        txt = open(os.path.join(SANDBOX, 'radial_log.txt'),
                   encoding='utf-8', errors='replace').read()
    except OSError:
        txt = ''
    # ⚠️ 不能只 grep 关键词 —— 日志头部自带一行「HOOK_SLOW/KEY_SLOW = …」的说明，
    # 会被误当成故障记录（第一版就误报过）。用行首的时间戳来区分。
    slow = [l for l in txt.splitlines()
            if ('HOOK_SLOW' in l or 'KEY_SLOW' in l or 'INJECT_SLOW' in l)
            and l.startswith('[')
            and ' = ' not in l]     # 头部说明行形如「…HOOK_SLOW/KEY_SLOW = 回调慢」，不�要算进去
    check('⑥ 日志无 HOOK_SLOW/KEY_SLOW（钩子回调不慢）', not slow,
          ('%d 条' % len(slow)) if slow else '')

    print()
    print('最慢的一次调用: %.0fms（正常应 <20ms；>250ms 即 UI 线程被卡）' % worst_ms[0])
    check('⑦ 全程最慢调用 < %dms（没有卡顿）' % STALL_MS,
          worst_ms[0] < STALL_MS, '%.0fms' % worst_ms[0])

    npass = sum(1 for _, ok in results if ok)
    print()
    print('合计 %d 项，失败 %d 项' % (len(results), len(results) - npass))
    return 0 if npass == len(results) else 1


if __name__ == '__main__':
    sys.exit(main())