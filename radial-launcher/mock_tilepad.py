# -*- coding: utf-8 -*-
"""「底板不动、只缩小里面的图标」——四档留白对照稿。

用户反馈：「不是让你缩小图标吗，我只让你缩小**图标框**，**只缩小里面的图标**，
这样框与图标就能有点间距了，好看」

**关键区别**（上一轮做错的就在这）：
- 上一轮：底板边长 == 图标边长，**两者一起缩小** → 间距恒为 0 → 白改
- 这一轮：**底板边长固定**（由 BAND_PAD/TILE_RAD 决定），只把图标按 `fit_k` 缩进去
  → 底板露出一圈白边 = 统一的内边距

变量只有一个：`TILE_PAD_K` = 底板边长 / 图标边长。
`TILE_PAD_K = 1.0` 就是现在（无留白），`1.18` = 底板比图标大 18%。
"""
import io
import math
import os

from PIL import Image, ImageDraw, ImageFont

import ring_geom as G

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 3
CANVAS = int(720.0 * G.K)
ICON_PX = 82

PAPER = (255, 255, 255)
INK = (28, 28, 30)
EDGE = (200, 200, 205)
BAND_DARK = 0.28
TILE_RAD_K = 0.24
BG = (150, 150, 156)


def mix(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def font(sz, bold=False):
    name = 'C:/Windows/Fonts/msyhbd.ttc' if bold else 'C:/Windows/Fonts/msyh.ttc'
    try:
        return ImageFont.truetype(name, int(sz * SS))
    except Exception:
        return ImageFont.load_default()


def text_c(d, s, cx, cy, f, col):
    tw = d.textlength(s, font=f) / SS
    asc, desc = f.getmetrics()
    th = (asc + desc) / SS
    d.text(((cx - tw / 2.0) * SS, (cy - th / 2.0) * SS), s, fill=col + (255,), font=f)


def load_icons():
    return __import__('mock_iconsize')._load_by_ini_order(os.path.join(HERE, '_dicondump'))


def alpha_bbox(im, thr=6):
    return im.getchannel('A').point(lambda v: 255 if v > thr else 0).getbbox()


def fit(im, draw_px):
    """按 alpha 外接框裁透明留白，再等比 fit 到 draw_px（只缩不放）。"""
    src = im
    bb = alpha_bbox(im)
    if bb:
        src = im.crop(bb)
    k = draw_px / max(src.width, src.height)
    if k < 1.0:
        src = src.resize((max(1, int(src.width * k)), max(1, int(src.height * k))), Image.LANCZOS)
    return src


def render(icons, n, tile_side, pad_k=1.20):
    """tile_side = 底板（圆角方块）边长，**不随 pad 变**；图标按 pad 缩进去。

    几何（r / disc / band）全部只由 tile_side 决定 —— 这正是「底板不动」的意义：
    留白变化**不会**让盘面变大变小，图标之间的间隙也不变。
    """
    W = CANVAS * SS
    img = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    C = CANVAS / 2.0
    hub = G.HUB_R

    # 与 ring_layout 同构，但输入是「底板边长」
    hs = math.sin(math.pi / max(n, 1))
    r = tile_side * (G.TANG + G.GAP_K) / (2.0 * hs)
    r_cap = G.DISC_R_MAX - tile_side * 0.5 - G.BAND_PAD
    if r > r_cap:
        r = r_cap
    r = max(r, G.R_ICON_MIN, hub + tile_side * 0.5 + 10.0)
    disc = r + tile_side * 0.5 + G.BAND_PAD
    r_in = max(min(hub - 5.0, r - tile_side * 0.5 - 6.0), hub * 0.5)

    def rr(box, rad, fill=None, outline=None, w=0):
        d.rounded_rectangle([v * SS for v in box], radius=rad * SS, fill=fill,
                            outline=outline, width=max(1, int(w * SS)) if outline else 0)

    d.ellipse([(C - disc) * SS] * 2 + [(C + disc) * SS] * 2, fill=PAPER + (255,))
    # 环带
    d.ellipse([(C - disc) * SS] * 2 + [(C + disc) * SS] * 2,
              fill=mix(PAPER, INK, BAND_DARK) + (255,))
    keep = Image.new('L', (W, W), 255)
    ImageDraw.Draw(keep).ellipse(
        [(C - r_in) * SS] * 2 + [(C + r_in) * SS] * 2, fill=0)
    img.putalpha(Image.composite(img.getchannel('A'), Image.new('L', (W, W), 0), keep))
    d = ImageDraw.Draw(img)
    d.ellipse([(C - disc) * SS] * 2 + [(C + disc) * SS] * 2,
              outline=mix(PAPER, INK, 0.52) + (255,), width=2 * SS)

    for i in range(min(n, len(icons))):
        a = math.radians(i * 360.0 / n)
        cx = C + r * math.sin(a)
        cy = C - r * math.cos(a)
        half = tile_side / 2.0
        rad = tile_side * TILE_RAD_K
        rr((cx - half, cy - half + 2.5 * G.K, cx + half, cy + half + 2.5 * G.K),
           rad, fill=mix(PAPER, INK, 0.13) + (255,))
        rr((cx - half, cy - half, cx + half, cy + half), rad, fill=PAPER + (255,),
           outline=mix(PAPER, INK, 0.34) + (255,), w=1.4 * G.K)
        ic = fit(icons[i][1], int(tile_side / pad_k * SS))
        img.alpha_composite(ic, (int(cx * SS - ic.width / 2), int(cy * SS - ic.height / 2)))

    d = ImageDraw.Draw(img)
    d.ellipse([(C - hub - 1.5) * SS, (C - hub + 2.5) * SS,
               (C + hub - 1.5) * SS, (C + hub + 2.5) * SS],
              fill=mix(PAPER, INK, 0.13) + (255,))
    d.ellipse([(C - hub) * SS] * 2 + [(C + hub) * SS] * 2, fill=INK + (255,))
    for sx in (-1, 1):
        ex = C + sx * hub * 0.30
        ew, eh = hub * 0.115, hub * 0.175
        d.rounded_rectangle([(ex - ew) * SS, (C - eh) * SS, (ex + ew) * SS, (C + eh) * SS],
                            radius=ew * SS, fill=PAPER + (255,))
    return img, (r, disc, r_in)


def main():
    icons = load_icons()
    n = len(icons)
    # 底板边长取当前实现的值（31.0），四档只改图标缩放比例
    TILE = 31.0
    PAD, HEAD, FOOT = 16, 46, 92
    variants = [
        ('现状 1.00', 1.00),
        ('1.12', 1.12),
        ('1.20 ★', 1.20),
        ('1.30', 1.30),
    ]
    out = Image.new('RGB', (CANVAS * 4 + PAD * 5, CANVAS + HEAD + FOOT + PAD * 2), (58, 58, 64))
    dd = ImageDraw.Draw(out)
    f = font(15)
    fs = font(12)
    for k, (title, pk) in enumerate(variants):
        im, geo = render(icons, n, TILE)
        # 图标按 pk 缩进去：重画一次，底板不变
        W = im.size[0]
        d2 = ImageDraw.Draw(im)
        r, disc, r_in = geo
        C = CANVAS / 2.0
        hs = math.sin(math.pi / max(n, 1))
        rr_ = max(r, G.R_ICON_MIN, G.HUB_R + TILE * 0.5 + 10.0)
        for i in range(min(n, len(icons))):
            a = math.radians(i * 360.0 / n)
            cx = C + rr_ * math.sin(a)
            cy = C - rr_ * math.cos(a)
            ic = fit(icons[i][1], int(TILE / pk * SS))
            im.alpha_composite(ic, (int(cx * SS - ic.width / 2), int(cy * SS - ic.height / 2)))
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im)
        flat = flat.convert('RGB').resize((CANVAS, CANVAS), Image.LANCZOS)
        x = PAD + k * (CANVAS + PAD)
        out.paste(flat, (x, PAD + HEAD))
        pad_px = (TILE - TILE / pk) / 2.0
        dd.text((x + 2, PAD + 2), title, fill=(246, 246, 246), font=f)
        dd.multiline_text(
            (x + 2, PAD + HEAD + CANVAS + 12),
            '底板 %.1f\n图标 %.1f\n每边 %.1f' % (TILE, TILE / pk, pad_px),
            fill=(158, 158, 164), font=fs, spacing=7)
    p1 = os.path.join(HERE, 'shots', 'tile_pad_options.png')
    out.save(p1)
    print('saved', p1, out.size)

    # 局部放大：取一颗图标 + 盘沿，看留白的实际观感
    Z, SRCW = 3.0, 92
    zw = int(SRCW * Z)
    z = Image.new('RGB', (zw * 4 + PAD * 5, zw + HEAD + FOOT + PAD * 2), (58, 58, 64))
    dz = ImageDraw.Draw(z)
    a = math.radians(2 * 360.0 / n)
    for k, (title, pk) in enumerate(variants):
        im, geo = render(icons, n, TILE)
        W = im.size[0]
        r, disc, r_in = geo
        C = CANVAS / 2.0
        rr_ = max(r, G.R_ICON_MIN, G.HUB_R + TILE * 0.5 + 10.0)
        im2 = ImageDraw.Draw(im)
        for i in range(min(n, len(icons))):
            aa = math.radians(i * 360.0 / n)
            cx = C + rr_ * math.sin(aa)
            cy = C - rr_ * math.cos(aa)
            ic = fit(icons[i][1], int(TILE / pk * SS))
            im.alpha_composite(ic, (int(cx * SS - ic.width / 2), int(cy * SS - ic.height / 2)))
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im).convert('RGB')
        ix, iy = C + rr_ * math.sin(a), C - rr_ * math.cos(a)
        mx, my = C + disc * math.sin(a), C - disc * math.cos(a)
        midx, midy = (ix + mx) / 2.0, (iy + my) / 2.0
        crop = flat.crop((int((midx - SRCW / 2) * SS), int((midy - SRCW / 2) * SS),
                          int((midx - SRCW / 2) * SS) + int(SRCW * SS),
                          int((midy - SRCW / 2) * SS) + int(SRCW * SS)))
        x = PAD + k * (zw + PAD)
        z.paste(crop.resize((zw, zw), Image.LANCZOS), (x, PAD + HEAD))
        dz.text((x + 2, PAD + 2), title, fill=(246, 246, 246), font=f)
        dz.multiline_text(
            (x + 2, PAD + HEAD + zw + 12),
            '每边 %.1fpx 白边' % ((TILE - TILE / pk) / 2.0),
            fill=(158, 158, 164), font=fs, spacing=6)
    p2 = os.path.join(HERE, 'shots', 'tile_pad_zoom.png')
    z.save(p2)
    print('saved', p2, z.size)


if __name__ == '__main__':
    main()