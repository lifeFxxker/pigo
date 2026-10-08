# -*- coding: utf-8 -*-
"""外环胶囊右键菜单的设计稿：胶囊的几种配色 + 菜单的三个交互态。

用户需求：「给外环加右键菜单，右键后可以给外环胶囊改名、换色」。
按项目铁律，先出设计稿验收再写实现。

**为什么单独一个脚本而不是并进 mock_ring.py**：
mock_ring.py 管内环几何（盘面/图标/环带），这个管外环胶囊的配色与菜单 ——
设计变量完全不同，混在一起以后改菜单样式会顺手改坏内环布局。

⚠️ 在 SS 超采样画布里画字，**字号必须乘 SS**，否则缩回去就是一团糊。
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

import ring_geom as G

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 3  # 超采样倍数，和 Rust 管线同思路

# ── 项目色板（与 src/main.rs 一致，全中性灰阶）──
PAPER = (255, 255, 255)
WALL = (238, 238, 240)
INK = (28, 28, 30)
EDGE = (200, 200, 205)
DANGER = (176, 52, 48)


def mix(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def font(sz, bold=False):
    name = 'C:/Windows/Fonts/msyhbd.ttc' if bold else 'C:/Windows/Fonts/msyh.ttc'
    try:
        return ImageFont.truetype(name, int(sz * SS))
    except Exception:
        return ImageFont.load_default()


def text_c(d, s, cx, cy, f, col):
    """在 SS 画布里按**视觉中心**对齐地画一行字（cx/cy 是逻辑坐标）。"""
    tw = d.textlength(s, font=f) / SS
    asc, desc = f.getmetrics()
    th = (asc + desc) / SS
    d.text(((cx - tw / 2.0) * SS, (cy - th / 2.0) * SS), s, fill=col + (255,), font=f)
    return tw, th


def rr(d, box, rad, fill=None, outline=None, width=0):
    d.rounded_rectangle([v * SS for v in box], radius=rad * SS,
                        fill=fill, outline=outline,
                        width=max(1, int(width * SS)) if outline else 0)


# ── 胶囊换色的候选色板 ──
# 铁律：全中性灰阶，**唯一允许彩色的只有真实 app 图标**。
# 所以「换色」给的是一组灰阶深浅；要做彩色得用户明确点头。
CHIP_TINTS = [
    ('默认（白底）', PAPER, EDGE, INK),
    ('一级灰', (243, 243, 245), (196, 196, 202), INK),
    ('二级灰', (227, 227, 231), (176, 176, 183), INK),
    ('深灰反白', (108, 108, 114), (84, 84, 90), PAPER),
    ('墨底反白', INK, INK, PAPER),
]


def chip(d, cx, cy, name, w, h, fill, edge, txt, hot=False, shadow=True):
    """横放胶囊。和 Rust 的 rot_rrect_ex 同构（圆角 = 高/2 = 胶囊）。"""
    box = (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
    if shadow:
        rr(d, (box[0], box[1] + 2.5, box[2], box[3] + 2.5), h / 2.0,
           fill=mix(fill, INK, 0.13) + (255,))
    rr(d, box, h / 2.0, fill=fill + (255,), outline=edge + (255,), width=2)
    text_c(d, name, cx, cy, font(12), txt)
    if hot:  # 右上角小圆点 = 这条被自定义过
        r, ax, ay = 2.4, box[2] - 8.0, box[1] + 5.0
        d.ellipse([(ax - r) * SS, (ay - r) * SS, (ax + r) * SS, (ay + r) * SS],
                  fill=(INK if txt == PAPER else PAPER) + (255,))


def check_mark(d, cx, cy, col):
    """✓ 用矢量折线画 —— msyh / msyhbd 都没有这个字形，会渲染成方框。"""
    d.line([((cx - 3.6) * SS, cy * SS),
            ((cx - 1.0) * SS, (cy + 2.6) * SS),
            ((cx + 3.8) * SS, (cy - 2.8) * SS)],
           fill=col + (255,), width=max(1, int(1.6 * SS)), joint='curve')


def menu(d, x, y, items, w=134, row_h=26, title=None, hover=None):
    """胶囊右键菜单：白圆角卡 + 2px 实色描边 + 硬投影，扁平无渐变。"""
    head = 24 if title else 0
    h = head + row_h * len(items) + 8
    rr(d, (x, y + 3, x + w, y + h + 3), 10, fill=mix(PAPER, INK, 0.13) + (255,))
    rr(d, (x, y, x + w, y + h), 10, fill=PAPER + (255,), outline=INK + (255,), width=2)
    cy = y + 4
    if title:
        text_c(d, title, x + 10 + d.textlength(title, font=font(11)) / SS / 2, cy + head / 2,
               font(11), mix(PAPER, INK, 0.5))
        cy += head
        rr(d, (x + 8, cy, x + w - 8, cy + 1), 0, fill=mix(PAPER, INK, 0.14) + (255,))
        cy += 1
    f = font(12)
    for idx, (label, kind) in enumerate(items):
        if kind == 'sep':
            rr(d, (x + 8, cy + row_h / 2, x + w - 8, cy + row_h / 2 + 1), 0,
               fill=mix(PAPER, INK, 0.12) + (255,))
            cy += row_h
            continue
        if hover is not None and idx == hover:
            rr(d, (x + 4, cy + 1, x + w - 4, cy + row_h - 1), 7,
               fill=mix(PAPER, INK, 0.08) + (255,))
        col = DANGER if kind == 'danger' else INK
        tw = d.textlength(label, font=f) / SS
        d.text(((x + 13) * SS, (cy + row_h / 2 - (f.getmetrics()[0] + f.getmetrics()[1]) / SS / 2) * SS),
               label, fill=col + (255,), font=f)
        if kind == 'check':
            check_mark(d, x + w - 17, cy + row_h / 2, mix(PAPER, INK, 0.55))
        cy += row_h
    return h


def main():
    names, items_map = G.read_settings(G.settings_path(HERE))
    # 挑条目最多的应用做演示（胶囊最多，最能看出菜单该贴哪）
    ai, best = 0, -1
    for i, nm in enumerate(names):
        c = len(items_map.get(nm, []))
        if c > best:
            best, ai = c, i
    its = items_map[names[ai]][:5] or ['条目']
    W1, W2 = 1160, 1160

    # ── ① 胶囊换色的几档 ──
    row = Image.new('RGBA', (W1 * SS, 124 * SS), WALL + (255,))
    d = ImageDraw.Draw(row)
    d.text((14 * SS, 10 * SS),
           '外环胶囊「换色」候选 —— 全部落在中性灰阶内（铁律：唯一彩色是 app 图标）',
           fill=INK + (255,), font=font(17))
    for k, (label, fill, edge, txt) in enumerate(CHIP_TINTS):
        cw = G.chip_half_w(its[0]) * 2
        x = 24 + k * 228
        d.text((x * SS, 48 * SS), label, fill=mix(PAPER, INK, 0.55) + (255,), font=font(13))
        chip(d, x + cw / 2, 84, its[0], cw, 30, fill, edge, txt, hot=(k >= 1))
    row = row.resize((W1, 124), Image.LANCZOS)

    # ── ② 右键菜单三态 ──
    m2 = Image.new('RGBA', (W2 * SS, 300 * SS), WALL + (255,))
    d = ImageDraw.Draw(m2)
    d.text((14 * SS, 10 * SS), '右键菜单三态：入口态 / 悬停项 / 换色子面板',
           fill=INK + (255,), font=font(17))
    fsm = font(13)
    MENU = [('改名…', 'item'), ('换色', 'item'), ('恢复默认', 'item'),
            ('—', 'sep'), ('删除该条目', 'danger')]
    TINTS = [('默认（白底）', 'check'), ('一级灰', 'item'), ('二级灰', 'item'),
             ('深灰反白', 'item'), ('墨底反白', 'item'), ('—', 'sep'), ('自定义…', 'item')]

    for k, (nm, hov, cap) in enumerate([
            (its[0], None, '① 在胶囊上右键'),
            (its[min(1, len(its) - 1)], 1, '② 悬停「换色」'),
            (its[min(2, len(its) - 1)], 0, '③ 换色子面板（默认项打勾）')]):
        left = 24 + k * 380
        cw = G.chip_half_w(nm) * 2
        chip(d, left + cw / 2, 72, nm, cw, 30, PAPER, EDGE, INK, shadow=False)
        d.text((left * SS, 96 * SS), cap, fill=mix(PAPER, INK, 0.5) + (255,), font=fsm)
        if k < 2:
            menu(d, left + cw + 14, 50, MENU, title=nm, hover=hov)
        else:
            menu(d, left + cw + 14, 50, TINTS, title='换色', hover=hov)
    m2 = m2.resize((W2, 300), Image.LANCZOS)

    GAP, FOOT = 26, 104
    out = Image.new('RGB', (W1, 124 + GAP + 300 + FOOT), (58, 58, 64))
    out.paste(row.convert('RGB'), (0, 0))
    out.paste(m2.convert('RGB'), (0, 124 + GAP))
    dd = ImageDraw.Draw(out)
    fsm2 = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', 14)
    fy = 124 + GAP + 300 + 12
    for i, (t, c) in enumerate([
        ('胶囊长轴严格平行圆环（上一轮已修）；菜单是独立浮层，不随环旋转 —— 悬停判据走屏幕坐标', (198, 198, 204)),
        ('右键菜单支持键盘 ↑↓ + Enter（自绘浮层也得能只用键盘操作）', (198, 198, 204)),
        ('配色全灰阶 —— 想做彩色请明说，那会破「唯一彩色 = app 图标」的铁律', (198, 198, 204)),
        ('实现要点：胶囊配置存 settings.ini 的 [app.X] 段（新增 tint= 键）；改名 = 改键名后整段重写，写前先备份', (150, 150, 156)),
    ]):
        dd.text((14, fy + i * 20), t, fill=c, font=fsm2)

    path = os.path.join(HERE, 'shots', 'chip_menu_draft.png')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    out.save(path)
    print('saved', path, out.size)
    print('演示应用 =', names[ai], '条目', its)


if __name__ == '__main__':
    main()
