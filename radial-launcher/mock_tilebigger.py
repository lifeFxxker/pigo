# -*- coding: utf-8 -*-
"""底板放大 + 图标同步放大（留白比例 TILE_PAD_K=1.20 不变）的对照稿。

用户反馈：「有点太小了，边框可以大一些，图标也大一点点」

**关键约束**（不是随便放大就行的）：
盘径上限 `DISC_R_MAX = 220×K = 145.2px`。7 个应用时盘径本来是 130.6，
放大底板会顶到这个上限；12 个应用时更早顶到 → 布局会**反过来缩图标**，
所以「放大」到某个点就反向变小了。表里列了实测值。

变量只有一个 `KI`（= 56×KI 得出底板边长）；图标 = 底板 / 1.20 同步放大，
所以**留白观感完全不变**，只是整体变大。
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

import ring_geom as G
import mock_tilepad as MT

HERE = os.path.dirname(os.path.abspath(__file__))
PAD, HEAD, FOOT = 16, 46, 92
TILE_PAD_K = 1.20

# 与 ring_layout 同构的复算（要能覆盖 KI，所以不能直接用 ring_geom 的常量）
DISC_R_MAX = 220.0 * G.K
R_ICON_MIN = 132.0 * G.K


def layout(ki, n):
    icon0 = 56.0 * ki
    hs = math.sin(math.pi / n)
    icon = icon0
    r = icon * (G.TANG + G.GAP_K) / (2.0 * hs)
    cap = DISC_R_MAX - icon * 0.5 - G.BAND_PAD
    if r > cap:
        r = cap
        icon = 2.0 * r * hs / (G.TANG + G.GAP_K)
    icon = min(max(icon, G.ICON_MIN), icon0)
    r = max(r, R_ICON_MIN, G.HUB_R + icon * 0.5 + 10.0)
    return icon, r


def main():
    icons = MT.load_icons()
    n = len(icons)
    variants = [
        ('现状 31', 0.554),
        ('34', 0.600),
        ('36 ★', 0.640),
        ('38', 0.680),
    ]
    f = MT.font(15)
    fs = MT.font(12)

    # ① 整盘三档并排
    out = Image.new('RGB', (G.CANVAS * 4 + PAD * 5, G.CANVAS + HEAD + FOOT + PAD * 2), (58, 58, 64))
    dd = ImageDraw.Draw(out)
    for k, (title, ki) in enumerate(variants):
        tile, _ = layout(ki, n)
        im, geo = MT.render(icons, n, tile, TILE_PAD_K)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, MT.BG + (255,)), im)
        flat = flat.convert('RGB').resize((G.CANVAS, G.CANVAS), Image.LANCZOS)
        x = PAD + k * (G.CANVAS + PAD)
        out.paste(flat, (x, PAD + HEAD))
        dd.text((x + 2, PAD + 2), title, fill=(246, 246, 246), font=f)
        i7, _ = layout(ki, 7)
        i12, _ = layout(ki, 12)
        d7 = layout(ki, 7)[1] + i7 * 0.5 + G.BAND_PAD
        d12 = layout(ki, 12)[1] + i12 * 0.5 + G.BAND_PAD
        dd.multiline_text(
            (x + 2, PAD + HEAD + G.CANVAS + 12),
            '底板 %.1f\n图标 %.1f\n盘径 %s/%s' % (tile, tile / TILE_PAD_K,
                                              ('%.0f' % d7), ('%.0f' % d12)),
            fill=(158, 158, 164), font=fs, spacing=7)
    p1 = os.path.join(HERE, 'shots', 'tile_bigger_options.png')
    out.save(p1)
    print('saved', p1, out.size)

    # ② 局部放大：一颗图标 + 盘沿
    Z, SRCW = 2.6, 100
    zw = int(SRCW * Z)
    z = Image.new('RGB', (zw * 4 + PAD * 5, zw + HEAD + FOOT + PAD * 2), (58, 58, 64))
    dz = ImageDraw.Draw(z)
    a = math.radians(2 * 360.0 / n)
    for k, (title, ki) in enumerate(variants):
        tile, r = layout(ki, n)
        disc = r + tile * 0.5 + G.BAND_PAD
        im, geo = MT.render(icons, n, tile, TILE_PAD_K)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, MT.BG + (255,)), im)
        flat = flat.convert('RGB')
        C = G.CANVAS / 2.0
        ix, iy = C + r * math.sin(a), C - r * math.cos(a)
        mx, my = C + disc * math.sin(a), C - disc * math.cos(a)
        midx, midy = (ix + mx) / 2.0, (iy + my) / 2.0
        crop = flat.crop((int((midx - SRCW / 2) * MT.SS), int((midy - SRCW / 2) * MT.SS),
                          int((midx - SRCW / 2) * MT.SS) + int(SRCW * MT.SS),
                          int((midy - SRCW / 2) * MT.SS) + int(SRCW * MT.SS)))
        x = PAD + k * (zw + PAD)
        z.paste(crop.resize((zw, zw), Image.LANCZOS), (x, PAD + HEAD))
        dz.text((x + 2, PAD + 2), title, fill=(246, 246, 246), font=f)
        dz.multiline_text(
            (x + 2, PAD + HEAD + zw + 12),
            '底板 %.1f\n图标 %.1f\n白边 %.1f' % (tile, tile / TILE_PAD_K,
                                            (tile - tile / TILE_PAD_K) / 2.0),
            fill=(158, 158, 164), font=fs, spacing=6)
    p2 = os.path.join(HERE, 'shots', 'tile_bigger_zoom.png')
    z.save(p2)
    print('saved', p2, z.size)

    print()
    print('%-10s %-8s %-8s %-10s %-10s' % ('KI', '底板', '图标', '7个盘径', '12个盘径'))
    for ki in (0.554, 0.600, 0.640, 0.680, 0.720):
        i7, r7 = layout(ki, 7)
        i12, r12 = layout(ki, 12)
        d7 = r7 + i7 * 0.5 + G.BAND_PAD
        d12 = r12 + i12 * 0.5 + G.BAND_PAD
        print('%-10.3f %-8.1f %-8.1f %-10.1f %-10.1f' % (ki, i7, i7 / TILE_PAD_K, d7, d12))
    print('\n盘径上限 DISC_R_MAX = %.1f' % DISC_R_MAX)


if __name__ == '__main__':
    main()