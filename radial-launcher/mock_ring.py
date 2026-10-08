# -*- coding: utf-8 -*-
"""用**真实图标**（_dicondump/*.rgba，带 alpha）离线渲染几种环布局方案，做设计对照稿。

为什么不用 HTML：环是像素级自绘的，CSS 圆角和真实渲染不是一回事。
这里用 PIL 在 SS 倍超采样下画完再降采样，和 Rust 那边的管线同思路，可逐像素对比。

用法：
    RL_ICON_DUMP=<abs dir> ./RadialLauncher.exe --icons     # 先导出图标（带 alpha）
    python mock_ring.py                                  # 生成 shots/ring_mock.png
"""
import glob
import math
import os
import re

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
DICON = os.path.join(HERE, '_dicondump')

SS = 3                      # 超采样倍率
CANVAS = 420                # 逻辑画布边长（所有方案共用，图标尺寸可直接对比）

# ── 配色（与 src/main.rs 的 Pal 对齐）──
PAPER = (255, 255, 255)
WALL = (238, 238, 240)
WALL2 = (227, 227, 230)
INK = (28, 28, 30)
EDGE = (150, 150, 154)


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


# ── 图标 ──
def load_icons():
    txt = open(os.path.join(HERE, 'settings.ini'), encoding='utf-8').read()
    body = txt.split('[apps]', 1)[1].split('\n[app.', 1)[0]
    pairs = [(l.split('=', 1)[0].strip(), l.split('=', 1)[1].strip())
             for l in body.splitlines() if '=' in l]
    icons = {}
    for f in glob.glob(os.path.join(DICON, 'norm_*.rgba')):
        # ⚠️ 纹理边长由 Rust 的 `ICON_PX = ICON_DRAW*2.05` 决定，**随 `KI` 变**。
        # 写死 82 会在改了 `KI`（现在是 0.640 → 73）之后报 "not enough image data"。
        # 从文件大小反推，永不失效。
        raw = open(f, 'rb').read()
        side = int((len(raw) // 4) ** 0.5)
        icons[os.path.basename(f)[5:-5]] = Image.frombytes(
            'RGBA', (side, side), raw, 'raw', 'BGRA')
    picks, used = [], set()
    for name, target in pairs:
        stem = re.sub(r'[^A-Za-z0-9]', '_', os.path.basename(target))
        hit = next((k for k in icons if k not in used and stem and stem in k), None)
        hit = hit or next((k for k in icons if k not in used), None)
        if hit:
            used.add(hit)
            picks.append((name, icons[hit]))
    return picks


def content_box(im, thr=8):
    px = im.load()
    xs = [x for y in range(im.height) for x in range(im.width) if px[x, y][3] > thr]
    ys = [y for y in range(im.height) for x in range(im.width) if px[x, y][3] > thr]
    if not xs:
        return None
    return (min(xs), min(ys), max(xs) + 1, max(ys) + 1)


def fit(im, box, size):
    """按内容外接框裁切后等比放进 size×size（居中）。box=None 时直接铺满（现状做法）。"""
    if box:
        im = im.crop(box)
    w, h = im.size
    k = size / max(w, h)
    return im.resize((max(1, round(w * k)), max(1, round(h * k))), Image.LANCZOS)


# ── 绘制基元（逻辑坐标 × SS）──
def rr(d, box, rad, fill=None, outline=None, width=0):
    d.rounded_rectangle([round(v * SS) for v in box], radius=max(1, round(rad * SS)),
                        fill=fill, outline=outline, width=max(0, round(width * SS)))


def hub(d, cx, cy, r, ball, eye):
    d.ellipse([(cx - r) * SS, (cy - r) * SS, (cx + r) * SS, (cy + r) * SS], fill=ball)
    for s in (-1, 1):
        ex, hw, hh = cx + s * 0.256 * r, 0.145 * r, 0.255 * r
        rr(d, (ex - hw, cy - hh, ex + hw, cy + hh), min(hw, hh), fill=eye)


def paste_icon(img, ic, cx, cy, plate=None):
    if plate:
        ps, prad, pfill, pedge = plate
        d = ImageDraw.Draw(img)
        rr(d, (cx - ps / 2 + 0.5, cy - ps / 2 + 3, cx + ps / 2 + 0.5, cy + ps / 2 + 3),
           prad, fill=mix(pfill, INK, 0.16))                      # 硬投影
        rr(d, (cx - ps / 2, cy - ps / 2, cx + ps / 2, cy + ps / 2),
           prad, fill=pfill, outline=pedge, width=1.5)            # 底板
    img.paste(ic, (round(cx * SS - ic.width / 2), round(cy * SS - ic.height / 2)), ic)


# ── 四个方案 ──
# ⚠️ 几何常量**从 ring_geom.py 读**（单一来源）。之前这里硬编码了一份，
#    改了 Rust 侧忘了同步，对照稿就画出一套不存在的「现状」，白比半天。
import ring_geom as G

K = G.K
HUB_R = G.HUB_R
TANG = G.TANG
GAP_K = G.GAP_K
BAND_PAD = G.BAND_PAD
DISC_R_MAX = G.DISC_R_MAX
ICON_MIN = G.ICON_MIN
ICON_DRAW = G.ICON_DRAW0


def ring_layout_new(n):
    return G.ring_layout(n)


def disc_r_new(n):
    return G.disc_r(n)


def band_in_new(n):
    return G.band_in(n)


def gap_of(n):
    r, icon = ring_layout_new(n)
    chord = 2.0 * r * math.sin(math.pi / max(n, 1))
    rad = 9.0 * (icon / 38.0)
    half = icon / 2.0
    tang = 2.0 * (((half - rad) * math.sqrt(2) + rad) if half > rad else half)
    return chord - tang, chord, icon


def mk_tile(tile_k, dark, donut=False, pad_in=None):
    """统一圆角方形图标：底板 = icon*tile_k，图标按内容裁切后 fit 到 icon。"""
    def fn(icons, n):
        r, icon = ring_layout_new(n)
        disc = r + icon / 2.0 + BAND_PAD
        r_in = band_in_new(n) if pad_in is None else max(HUB_R + pad_in, r - icon / 2.0 - pad_in)
        band = (r_in, disc, mix(PAPER, INK, dark))
        tile = icon * tile_k
        return _render(icons, n, r, disc, icon, HUB_R, trim=True,
                       band=band if donut else None,
                       track=None if donut else band,
                       plate=(tile, tile * 0.24, PAPER, mix(PAPER, INK, 0.34)))
    return fn


def mk_plain(dark, donut=False):
    def fn(icons, n):
        r, icon = ring_layout_new(n)
        disc = r + icon / 2.0 + BAND_PAD
        r_in = max(HUB_R + 8, r - icon / 2.0 - BAND_PAD)
        band = (r_in, disc, mix(PAPER, INK, dark))
        return _render(icons, n, r, disc, icon, HUB_R, trim=True,
                       band=band if donut else None,
                       track=None if donut else band)
    return fn


BAND_DARK = 0.28
v_tile_fill = mk_tile(1.0, BAND_DARK)       # 不留白：底板 = 图标
v_tile_pad = mk_tile(1.22, BAND_DARK)       # 留白：底板比图标大 22%
v_tile_hole = mk_tile(1.0, BAND_DARK, True) # 不留白 + 中心挖空
v_plain = mk_plain(BAND_DARK)               # 无底板对照
v_tile_tight = mk_tile(1.0, BAND_DARK, pad_in=14.0)  # 内沿贴近头像（对标参考图的比例）


def variant_baseline(icons, n):
    """现状：直接用 ring_geom（= 当前 Rust 实现）的几何"""
    r, icon = ring_layout_new(n)
    return _render(icons, n, r, disc_r_new(n), icon, HUB_R, disc=PAPER, trim=True)


def _render(icons, n, R_ICON, DISC, IDRAW, HUB_R, disc=PAPER, trim=True,
            plate=None, band=None, track=None):
    W = CANVAS * SS
    img = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    C = CANVAS / 2

    if disc is not None:
        d.ellipse([(C - DISC) * SS, (C - DISC) * SS + 6 * SS,
                   (C + DISC) * SS, (C + DISC) * SS + 6 * SS], fill=mix(PAPER, INK, 0.15))
        d.ellipse([(C - DISC) * SS, (C - DISC) * SS,
                   (C + DISC) * SS, (C + DISC) * SS], fill=disc,
                  outline=mix(PAPER, INK, 0.52), width=2 * SS)
    if band:
        r_in, r_out, col = band
        d.ellipse([(C - r_out) * SS, (C - r_out) * SS, (C + r_out) * SS, (C + r_out) * SS],
                  fill=col, outline=mix(PAPER, INK, 0.52), width=2 * SS)
        keep = Image.new('L', (W, W), 255)
        ImageDraw.Draw(keep).ellipse(
            [(C - r_in) * SS, (C - r_in) * SS, (C + r_in) * SS, (C + r_in) * SS], fill=0)
        img.putalpha(Image.composite(img.getchannel('A'), Image.new('L', (W, W), 0), keep))
        d = ImageDraw.Draw(img)
    if track:
        r_in, r_out, col = track
        d.ellipse([(C - r_out) * SS, (C - r_out) * SS, (C + r_out) * SS, (C + r_out) * SS], fill=col)
        d.ellipse([(C - r_in) * SS, (C - r_in) * SS, (C + r_in) * SS, (C + r_in) * SS], fill=disc)

    for i, (name, im) in enumerate(icons[:n]):
        a = -90 + i * 360.0 / n
        cx = C + R_ICON * math.cos(math.radians(a))
        cy = C + R_ICON * math.sin(math.radians(a))
        ic = fit(im, content_box(im) if trim else None, round(IDRAW * SS))
        paste_icon(img, ic, cx, cy, plate=plate)

    hub(d, C, C, HUB_R, INK, PAPER)
    return img


def main():
    icons = load_icons()
    n = len(icons)
    print('icons:', n)

    variants = [('现状：自适应环径 r94.4 / 图标 34.3 / 间隙 0.28（真实布局）', variant_baseline),
                ('甲 中环带 + 统一圆角方形（不留白，图标 33.2 ≈ 现状）', v_tile_fill),
                ('乙 留白版：底板比图标大 22%', v_tile_pad),
                ('丙 甲 + 中心挖空', v_tile_hole)]

    try:
        font = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', 16)
    except Exception:
        font = ImageFont.load_default()

    BG = (140, 140, 146)
    cells = []
    for title, fn in variants:
        im = fn(icons, n)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im)
        cells.append((title, flat.convert('RGB').resize((CANVAS, CANVAS), Image.LANCZOS)))

    # 2×2，1:1，尺寸可比
    pad, cap = 14, 28
    out = Image.new('RGB', (CANVAS * 2 + pad * 3, (CANVAS + cap) * 2 + pad * 3), (62, 62, 68))
    dd = ImageDraw.Draw(out)
    for k, (title, im) in enumerate(cells):
        x = pad + (k % 2) * (CANVAS + pad)
        y = pad + (k // 2) * (CANVAS + cap + pad)
        out.paste(im, (x, y + cap))
        dd.text((x + 2, y + 6), title, fill=(246, 246, 246), font=font)
    path = os.path.join(HERE, 'shots', 'ring_options.png')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    out.save(path)
    print('saved', path, out.size)

    # 局部放大：右下象限 2.4×，核对图标清晰度
    Z, SRC = 2.4, 176
    zw = round(SRC * Z)
    z = Image.new('RGB', (zw * 2 + pad * 3, (zw + cap) * 2 + pad * 3), (62, 62, 68))
    dz = ImageDraw.Draw(z)
    for k, (title, im) in enumerate(cells):
        x = pad + (k % 2) * (zw + pad)
        y = pad + (k // 2) * (zw + cap + pad)
        crop = im.crop((CANVAS // 2 - 24, CANVAS // 2 - 24, CANVAS // 2 - 24 + SRC,
                        CANVAS // 2 - 24 + SRC))
        z.paste(crop.resize((zw, zw), Image.LANCZOS), (x, y + cap))
        dz.text((x + 2, y + 6), title, fill=(246, 246, 246), font=font)
    path2 = os.path.join(HERE, 'shots', 'ring_options_zoom.png')
    z.save(path2)
    print('saved', path2, z.size)


if __name__ == '__main__':
    main()
