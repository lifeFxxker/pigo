# -*- coding: utf-8 -*-
"""分析 radial_log.txt：钩子回调是否超时、菜单开关是否配对。

用户报「有时候鼠标失效，只有当前窗口能点，任务栏点不动」。
低级钩子（WH_MOUSE_LL）一旦超时，Windows 会**静默摘掉**它，
而摘掉的过程里输入会短暂失去响应；OLE 拖放卡在模态循环也会造成
「只有拖源窗口能响应」。这个脚本从日志里找证据。
"""
import io
import os
import re
import sys
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
PATH = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, 'radial_log.txt')

if not os.path.exists(PATH):
    print('找不到 %s' % PATH)
    sys.exit(0)

lines = io.open(PATH, encoding='utf-8', errors='replace').read().splitlines()
ev = []
for l in lines:
    if '启动' in l:
        ev.append(('BOOT', l[:23], 0))
        continue
    m = re.search(r'\]\s+[\d.]+\s+(HOOK_\w+|KEY_\w+|UI_\w+|WATCHDOG_\w+|MENU_\w+)(?: aux=(\d+)ms)?', l)
    if m:
        ev.append((m.group(1), l[:23], int(m.group(2) or 0)))

print('=== 事件统计 ===')
c = Counter(k for k, _, _ in ev)
for k, n in c.most_common():
    print('  %-14s %d' % (k, n))

print()
print('=== 关键判定 ===')
# ① 钩子回调耗时。系统的 LowLevelHooksTimeout 默认 300ms、用户可放宽到 1000ms；
#    超过它 Windows 会**静默摘掉钩子** → 输入链断（鼠标点不动、菜单弹不出）。
slow = [(k, a, t) for k, t, a in ev if k in ('HOOK_SLOW', 'KEY_SLOW', 'UI_SLOW') and a > 0]
if slow:
    print('  ✗ 有 %d 条慢回调记录（>30ms），最大 %dms'
          % (len(slow), max(a for _, a, _ in slow)))
    for k, a, t in slow[-8:]:
        print('      %s %s aux=%dms' % (t, k, a))
    # 慢回调的**典型成因是堆积**：前面的事件没被处理完，后面的一起涌进来。
    # 判据：慢回调密集出现在「长时间无事件」之后 = 事件在别处被堵了半天。
    print('  → 慢回调若成簇出现在长 HOOK_GAP 之后，说明事件被**别处**堵住后一次性涌入')
else:
    print('  ✓ 钩子/UI 回调没有任何一条超过 30ms —— 钩子本身不慢')

# ② 菜单开关配对：开了没收 = 卡在打开态
seg, cur = [], []
for k, t, _ in ev:
    if k == 'BOOT':
        if cur:
            seg.append(cur)
        cur = []
    else:
        cur.append((k, t, 0))
if cur:
    seg.append(cur)
print()
print('=== 逐次运行 ===')
bad = 0
for i, s in enumerate(seg):
    op = sum(1 for k, _, _ in s if k == 'MENU_OPEN')
    cl = sum(1 for k, _, _ in s if k == 'MENU_CLOSE')
    flag = ''
    if op > cl:
        flag = '  <<< 开了 %d 次只收了 %d 次（可能卡在打开态）' % (op, cl)
        bad += 1
    print('  run%-3d OPEN=%-3d CLOSE=%-3d%s' % (i + 1, op, cl, flag))
if bad:
    print()
    print('  → 有 %d 次运行菜单没配对收完' % bad)
else:
    print()
    print('  ✓ 每次运行的菜单开关都配对')

# ③ HOOK_GAP：两次鼠标事件的间隔。异常大的间隔说明期间输入根本没进到我们这
gaps = [a for k, _, a in ev if k == 'HOOK_GAP' and a > 0]
if gaps:
    print()
    print('=== 鼠标事件间隔（HOOK_GAP > 30s 才记）===')
    for g in gaps:
        h = g / 1000.0 / 3600.0
        print('  %8.1f s  (%.1f 小时)' % (g / 1000.0, h))
    print('  说明：间隔大 = 那段时间**没有鼠标移动**（人没动鼠标，或输入被别处吃了）')
