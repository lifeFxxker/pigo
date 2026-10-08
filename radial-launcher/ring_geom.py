# -*- coding: utf-8 -*-
"""测试用的径向几何：镜像 src/main.rs 里的 ring_layout / chip_layout。

刻意做成「读 settings.ini 推导」，而不是硬编码 —— 应用数一变，
内环半径按 ring_layout 自适应、外环直径也变，硬编码的坐标会整体失效。

注意：这里必须和 Rust 侧保持同步。改 src/main.rs 的布局公式时，
同步改这里；不同步的话测试会以「hover 落空」的形式失败（这是有意的信号）。
"""
import io
import math
import os

# ── 与 src/main.rs 顶部常量一致 ──
K = 0.66
KI = 0.640  # 底板 35.8px / 图标 29.9px（用户反馈太小 → 放大；再大顶 DISC_R_MAX 会被压缩）
KC = 1.34

CANVAS = int(720.0 * K)          # 475
CC = CANVAS // 2                 # 237（测试用整数，避免 SendMessageW 参数类型报错）

DISC_R_MAX = 240.0 * K           # 盘面半径上限（与 Rust 同步：220 时 12 应用会把图标压小）
R_ICON_MIN = 132.0 * K           # 图标环半径下限
ICON_DRAW0 = 56.0 * KI           # 图标绘制边长上限
GAP_K = 0.45                     # 相邻图标的目标空隙 / 图标边长
TANG = 1.22                      # 圆角方块在切向的最坏投影系数
BAND_PAD = 28.0                  # 图标外沿 → 盘边留白（用户反馈贴边，18 → 24 → 28）
ICON_MIN = 30.0                  # 图标缩放下限
HUB_R = 66.0 * K                 # 中心头像球半径
RM_BTN_DY = HUB_R * 0.88 + 12.0  # 取消按钮中心（按钮出现时头像缩12%让位，镜像 main.rs）

CHIP_PAD_L = 6.0 * K * KC
CHIP_ICON = 22.0 * K * KC
CHIP_GAP = 8.0 * K * KC
CHIP_PAD_R = 14.0 * K * KC
CHIP_MIN_GAP = 12.0 * K * KC
CHIP_H = 34.0 * K * KC
CHIP_H2 = CHIP_H / 2.0
RING_NAME_MAX_W = 90.0      # 段里名称宽度上限（镜像 Rust 的 RING_NAME_MAX_W）

CJK_W = 13.0 * K * KC
LAT_W = 7.0 * K * KC


CHIP_TILT_K = 1.0    # 胶囊长轴严格平行于圆环（3/9 点就是竖的）
CHIP_TILT_MAX = 90.0
CHIP_FAN_MAX = 240.0


def read_settings(path):
    """返回 (apps:[名字], items:{应用名:[条目名]})"""
    apps, items, sec = [], {}, None
    for line in io.open(path, encoding='utf-8').read().splitlines():
        t = line.strip()
        if t.startswith('[') and t.endswith(']'):
            sec = t[1:-1]
            continue
        if not t or '=' not in t:
            continue
        k = t.split('=')[0]
        if sec == 'apps':
            apps.append(k)
        elif sec and sec.startswith('app.'):
            items.setdefault(sec[4:], []).append(k)
    return apps, items


def ring_layout(n):
    """镜像 Rust 的 ring_layout：返回 (内环半径, 图标绘制边长)

    目标 = 相邻两格之间留出 GAP_K×图标 的空隙；半径顶到 DISC_R_MAX 就反过来缩图标。
    """
    n = max(n, 1)
    hs = math.sin(math.pi / n)
    icon = ICON_DRAW0
    r = icon * (TANG + GAP_K) / (2.0 * hs)
    r_cap = DISC_R_MAX - icon * 0.5 - BAND_PAD
    if r > r_cap:
        r = r_cap
        icon = 2.0 * r * hs / (TANG + GAP_K)
    icon = min(max(icon, ICON_MIN), ICON_DRAW0)
    r = max(r, R_ICON_MIN, HUB_R + icon * 0.5 + 10.0)
    return r, icon


def disc_r(n):
    """镜像 Rust 的 disc_r()：盘面半径"""
    r, icon = ring_layout(n)
    return r + icon * 0.5 + BAND_PAD


def band_in(n):
    """镜像 Rust 的 band_in()：环带内沿**压到头像球底下**（头像周围不留白圈）"""
    r, icon = ring_layout(n)
    return max(min(HUB_R - 5.0, r - icon * 0.5 - 6.0), HUB_R * 0.5)


def est_text_w(text):
    return sum(CJK_W if ord(c) > 0x2E7F else LAT_W for c in text)


def chip_half_w(name):
    # ⚠️ 名称宽度**封顶** RING_NAME_MAX_W（镜像 Rust 的 chip_half_w）：
    # 段宽正比于名字宽度，一个 18 字符的 URL 会让单段吃掉 50°+ 的环。
    return (CHIP_PAD_L + CHIP_ICON + CHIP_GAP
            + min(est_text_w(name), RING_NAME_MAX_W) + CHIP_PAD_R) / 2.0


def norm_rot(a):
    """镜像 Rust 的 norm_rot：角度归一到 (-90°, 90°]（只为「文字从左向右读」）"""
    aa = ((a % 360.0) + 360.0) % 360.0
    if aa <= 90.0 or aa >= 270.0:
        return aa - 360.0 if aa > 180.0 else aa
    return aa - 180.0


def chip_segments(names, app_angle_deg, n_apps=12):
    """镜像 Rust 的 chip_layout（**甲方案 = 贴身扇面**）：返回 [(a1, a2, rot, r), ...]

    每段的角跨 ∝ 该条目的名字宽度，段间留 `CHIP_MIN_GAP` 的**弧长**缝；
    整体以 `app_angle_deg`（当前应用的方位角）为中心左右对称铺开。
    跨度 > `CHIP_FAN_MAX` 时才退回「整圈均布 + 首段锚定」。

    ⚠️ 半径**必须**是环带的中位半径（`disc_r + 3K + CHIP_H/2`）—— 段就画在那条带上。
    旧版这里用 `max(..., 盘沿)` 把半径外扩，那是给「贴弧胶囊」用的
    （胶囊可以浮在任意半径上）；梯形段固定在环带里，外扩会让段和内容对不上。
    `n_apps` 必须传 —— 盘面半径随应用数自适应，段所在环带跟着变。
    """
    m = len(names)
    if m == 0:
        return []
    halfs = [chip_half_w(x) for x in names]
    s = sum(2.0 * h for h in halfs)
    r = disc_r(n_apps) + 3.0 * K + CHIP_H2

    deg_per_px = 180.0 / (math.pi * r)
    gap_deg = CHIP_MIN_GAP * deg_per_px
    wdeg = [2.0 * h * deg_per_px for h in halfs]
    arc = s + CHIP_MIN_GAP * max(m - 1, 0)

    segs, acc = [], 0.0
    if arc * deg_per_px <= CHIP_FAN_MAX:
        # 贴身扇面：整体以图标方位角为中心
        acc = -arc * deg_per_px * 0.5
        for k in range(m):
            segs.append((app_angle_deg + acc, app_angle_deg + acc + wdeg[k]))
            acc += wdeg[k] + gap_deg
    else:
        # 整圈均布：间隙数 = m（闭合环首尾之间也要算一个），第 1 段锚定在图标方位角
        gap = max((2.0 * math.pi * r - s) / m, CHIP_MIN_GAP)
        gd = gap * deg_per_px
        for k in range(m):
            segs.append((app_angle_deg + acc, app_angle_deg + acc + wdeg[k]))
            acc += wdeg[k] + gd
    return [(a1, a2, norm_rot((a1 + a2) * 0.5), r) for (a1, a2) in segs]


def chip_layout(names, app_angle_deg, n_apps=12):
    """兼容旧调用：返回 [(中位角, 半径), ...]。真正的几何在 `chip_segments`。"""
    return [((a1 + a2) * 0.5, r)
            for (a1, a2, _rot, r) in chip_segments(names, app_angle_deg, n_apps)]


def app_angle(i, n_apps):
    """镜像 Rust 的 app_angle：**0° = 正上**、顺时针，第 i 个应用的角度（度）。

    ⚠️ 是 `i * 360/n`，**没有 -90 偏移** —— 分析脚本里手写 `-90 + i*360/n`
    会把每个托盘的位置算错 90°，量出来的「图标清晰度」全是别人的格子。
    """
    return i * (360.0 / max(n_apps, 1))


def polar(r, deg):
    """方位角（0=上，顺时针）→ 画布坐标"""
    a = math.radians(deg)
    return (CC + r * math.sin(a), CC - r * math.cos(a))


def settings_path(exe_dir):
    return os.path.join(exe_dir, 'settings.ini')
