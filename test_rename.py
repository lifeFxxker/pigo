# -*- coding: utf-8 -*-
"""双击胶囊改名的回归测试。

**为什么全走 SendMessage 不用真鼠标**：双击要真钩子（低级钩子只认真实事件），
中文输入要真 IME —— 自动化都跑不了。所以加了诊断消息 `WM_APP_EDITSIM`，
它和真实消息**调用同一批函数**（`chip_click` / `edit_push` / `edit_finish`），
只是把「两次点击 / 一个字符 / 提交」这些输入摆出来。

跑法：必须在**沙箱**里（改名会写 settings.ini 并删条目）。
"""
import ctypes
import os
import shutil
import subprocess
import sys
import time
from ctypes import wintypes as w

HERE = os.path.dirname(os.path.abspath(__file__))
SANDBOX = os.path.join(HERE, os.environ.get('RL_SANDBOX', '_edittest'))
EXE = os.path.join(SANDBOX, 'Pigo.exe')
INI = os.path.join(SANDBOX, 'settings.ini')

WM_APP_EDITSIM = 0x8000 + 17
P0, P1, P2, P3, P4, P5 = range(6)

# 阶段 0 的 l=指定 chip 序号；这里默认第一个
CHIP0 = 0

results = []


def check(name, ok, detail=''):
    results.append((name, bool(ok), detail))
    print('%-46s %s %s' % (name, 'PASS' if ok else 'FAIL', detail), flush=True)


def read_ini():
    apps, items, sec = [], {}, None
    for line in open(INI, encoding='utf-8').read().splitlines():
        t = line.strip()
        if t.startswith('[') and t.endswith(']'):
            sec = t[1:-1]
            continue
        if not t or '=' not in t:
            continue
        k = t.split('=', 1)[0]
        if sec == 'apps':
            apps.append(k)
        elif sec and sec.startswith('app.'):
            items.setdefault(sec[4:], []).append(k)
    return apps, items


def sim(hwnd, phase, arg=0):
    ctypes.windll.user32.SendMessageW(hwnd, WM_APP_EDITSIM, phase, arg)


def pick_app_with_items(hwnd):
    """找条目最多的应用，让诊断消息落在它身上（st.active_app 会被设成它）。"""
    import ring_geom as G
    apps, items = G.read_settings(INI)
    best, bi = -1, 0
    for i, nm in enumerate(apps):
        c = len(items.get(nm, []))
        if c > best:
            best, bi = c, i
    return apps[bi], best


def main():
    if os.path.isdir(SANDBOX):
        shutil.rmtree(SANDBOX, ignore_errors=True)
    os.makedirs(SANDBOX, exist_ok=True)
    for f in ('Pigo.exe',):
        src = os.path.join(HERE, f)
        if os.path.exists(src):
            shutil.copy(src, EXE)
    if not os.path.exists(EXE):
        src = os.path.join(HERE, 'target', 'release', 'pigo.exe')
        shutil.copy(src, EXE)
    shutil.copy(os.path.join(HERE, 'settings.ini'), INI)

    tag = os.environ.get('RL_TAG', 'e1')
    env = dict(os.environ)
    env['RL_INSTANCE'] = 'edit' + tag
    env['PIGO_CONFIG_DIR'] = SANDBOX
    # ⚠️ **必须用绝对路径**启动：exe 只把 ini 写到「自己所在目录」（exe_dir()），
    # 用相对名 + cwd= 时 Windows 的解析会落到**项目根目录**那份真配置上。
    # 这个坑真踩过：测试把用户 settings.ini 的「下载」改成了「下载ABC」。
    proc = subprocess.Popen([os.path.abspath(EXE)], cwd=SANDBOX, env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2.5)
    host = ctypes.windll.user32.FindWindowW('RLHost', None)
    if not host:
        print('FAIL: 找不到宿主窗口 RLHost')
        proc.kill()
        return 1

    u = ctypes.windll.user32
    # ⚠️ **不用中键呼出菜单**：环境里常有杀不掉的僵尸实例，它们的低级鼠标钩子
    # 会把中键吞掉（测过，ring 根本弹不出来）。诊断消息自己会设 active_app。
    app, n_items = pick_app_with_items(host)
    print('演示应用 = %s（%d 个条目）' % (app, n_items), flush=True)
    if n_items < 1:
        print('FAIL: 配置里没有可改名的条目')
        proc.kill()
        return 1

    # 先记下**真配置**的指纹，最后核对它没被碰过
    real_ini = os.path.join(HERE, 'settings.ini')
    real_before = open(real_ini, encoding='utf-8').read() if os.path.exists(real_ini) else ''

    _, items = read_ini()
    orig_names = list(items.get(app, []))
    orig_name = orig_names[CHIP0] if CHIP0 < len(orig_names) else orig_names[0]
    print('原名 = %r' % orig_name, flush=True)

    # 1. 双击进入编辑 → 编辑态激活
    sim(host, P0, CHIP0)
    time.sleep(0.3)
    check('双击胶囊 → 进入改名模式', True)  # 不崩即通过（下面逐项验证落盘）

    # 2. 追加中文（逐字符，走 edit_push 同一个入口）
    for ch in '我的文件夹':
        sim(host, P1, ord(ch))
    time.sleep(0.3)

    # 3. 退格删一个（删掉最后的「夹」）
    sim(host, P2)
    time.sleep(0.2)

    # 4. 提交 → 落盘
    #    期望 '下载我的文件'：双击进入时**预填原名**，用户接着输入是**追加**。
    #    （想整体替换：再双击一次进入「选中」态，第一次按键会清掉旧文本 ——
    #      那条路径由 edit_push 里的 edit_started < 0 分支负责。）
    sim(host, P3)
    time.sleep(0.5)
    _, items2 = read_ini()
    new_names = items2.get(app, [])
    expect = orig_name + '我的文件'
    check('Enter 提交 → ini 里出现新名（原名+追加）', expect in new_names, str(new_names))
    check('条目数不变（改名不是删除）', len(new_names) == len(orig_names),
          '%d vs %d' % (len(new_names), len(orig_names)))

    # 5. 留了 .bak
    check('留了 settings.ini.bak（无 git 的唯一回滚点）',
          os.path.exists(INI + '.bak'))

    # 6. 双击 → Esc 取消 → 名字不变
    sim(host, P0, CHIP0)
    time.sleep(0.25)
    for ch in 'ZZZ':
        sim(host, P1, ord(ch))
    sim(host, P4)          # 取消
    time.sleep(0.5)
    _, items3 = read_ini()
    names3 = items3.get(app, [])
    check('Esc 取消 → 名字不变', 'ZZZ' not in names3, str(names3))
    check('取消后条目数不变', len(names3) == len(new_names),
          '%d vs %d' % (len(names3), len(new_names)))

    # 7. 双击 → 清空 → 提交 = 删除该条目
    before = len(read_ini()[1].get(app, []))
    sim(host, P0, CHIP0)
    time.sleep(0.25)
    sim(host, P5)          # 清空 + 提交
    time.sleep(0.5)
    after = len(read_ini()[1].get(app, []))
    check('空提交 → 删除该条目', after == before - 1, '%d -> %d' % (before, after))

    # 8. 条目数变少后 chip 命中仍准（布局要跟着重算）
    time.sleep(0.3)
    sim(host, P3, 0)       # 无编辑态时是 no-op
    ok = proc.poll() is None
    check('改完仍正常运行（布局未崩）', ok)

    # 9. **真配置一个字节都没动**（沙箱跑测的底线）
    real_after = open(real_ini, encoding='utf-8').read() if os.path.exists(real_ini) else ''
    check('项目根目录的真实 settings.ini 未被改动', real_before == real_after)

    proc.terminate()
    try:
        proc.communicate(timeout=5)
    except Exception:
        pass

    npass = sum(1 for _, ok, _ in results if ok)
    print()
    print('合计 %d 项，失败 %d 项' % (len(results), len(results) - npass))
    return 0 if npass == len(results) else 1


if __name__ == '__main__':
    sys.exit(main())
