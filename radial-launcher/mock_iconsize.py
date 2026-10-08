# -*- coding: utf-8 -*-
"""内环图标缩小的三档对照稿（用户反馈「图标太大，完全贴着边框」）。

用**真实图标**（RL_ICON_DUMP 导出的 .rgba，带 alpha）离线渲染，
几何全部来自 ring_geom（= src/main.rs 的镜像），所以看到的就是真会跑出来的效果。

三档的差别只在 ICON_DRAW0（图标边长上限）与 BAND_PAD（盘沿留白）：

| 档 | ICON_DRAW0 | BAND_PAD | 12 应用实际 icon | 盘沿余量 | 间隙/图标 |
|----|-----------|----------|----------------|----------|----------|
| 现状 | 56×0.62=34.7 | 24 | 32.2 | 24 | 0.45 |
| 收一档 | 50×0.62=31.0 | 28 | 28.8 | 28 | 0.45 |
| 收两档 | 46×0.62=28.5 | 32 | 26.5 | 32 | 0.45 |

注意：**盘径会跟着缩**（DISC = r + icon/2 + BAND_PAD），
所以「图标变小」不等于「图标相对盘面变小」—— 相对关系由 GAP_K 决定，那个不动。
真正让观感变松的是 BAND_PAD（图标外沿到盘边那一圈）。
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

import ring_geom as G

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 3
CANVAS = int(720.0 * G.K)   # 475，和真机画布一致
ICON_PX = 82                # Rust 的 ICON_PX（2 倍超采样）

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


def load_icons():
    d = os.path.join(HERE, '_dicondump')
    order = []
    names = []
    for sec in G.read_settings(G.settings_path(HERE))[0]:
        for fn in sorted(os.listdir(d)):
            if not fn.endswith('.rgba'):
                continue
            key = fn[5:-5]
            # 用「显示名」反推比较难，直接按 ini 里的 target 路径尾部匹配
            tgt = ''
            for line in open(os.path.join(HERE, 'settings.ini'), encoding='utf-8'):
                pass
            del tgt
        break
    # 简化：ini 的 [apps] 顺序 == --icons 的导出顺序（同一份 load_ini）
    for line in open(os.path.join(HERE, 'settings.ini'), encoding='utf-8').read().splitlines():
        pass
    del order, names
    return _load_by_ini_order(d)


def _load_by_ini_order(d):
    """按 settings.ini 的 [apps] 顺序装载图标（与 Rust 的 load_ini 同一顺序）。"""
    apps, items, sec = [], {}, None
    for line in open(os.path.join(HERE, 'settings.ini'), encoding='utf-8').read().splitlines():
        t = line.strip()
        if t.startswith('[') and t.endswith(']'):
            sec = t[1:-1]
            continue
        if not t or '=' not in t:
            continue
        k, v = t.split('=', 1)
        if sec == 'apps':
            apps.append((k, v))
        elif sec and sec.startswith('app.'):
            items.setdefault(sec[4:], []).append((k, v))

    files = [f for f in sorted(os.listdir(d))
             if f.startswith('norm_') and f.endswith('.rgba')]
    files_by_stem = {f[5:-5]: f for f in files}
    out, missing = [], []
    # Rust 侧 decode_icon 对「已知系统目标」会先替换成**绝对系统路径**再提取，
    # 所以导出名里是 C__WINDOWS_SYSTEM32_cmd_exe 这种，不是 settings.ini 里的相对写法。
    # 这里补上映射，让对照稿能用上全部 7 个真实图标。
    SYS = {
        'cmd.exe': r'C:\WINDOWS\SYSTEM32\cmd.exe',
        'explorer.exe': r'C:\WINDOWS\explorer.exe',
        'ms-settings:': r'C:\Windows\ImmersiveControlPanel\SystemSettings.exe',
    }
    for name, target in apps:
        full = SYS.get(target.strip(), target)
        if '\\' not in full and ':' not in full[:3]:
            full = HERE.replace('/', '\\') + '\\' + full
        # Rust 侧规则（src/main.rs 的 decode_icon 导出分支）：
        # `c.is_ascii_alphanumeric()` 之外全换 '_' —— 中文/全角也被换掉。
        stem = ''.join(ch if (ch.isascii() and ch.isalnum()) else '_' for ch in full)
        hit = files_by_stem.get(stem)
        if hit is None:
            missing.append('%s -> %s' % (name, stem))
            continue
        # 纹理边长由 Rust 的 ICON_PX 决定（随 KI 变），**从文件大小推断**，
        # 写死 ICON_PX 会在改了 KI 之后报 "not enough image data"。
        raw = open(os.path.join(d, hit), 'rb').read()
        side = int(len(raw) // 4) ** 0.5
        im = Image.frombytes('RGBA', (int(side), int(side)), raw, 'raw', 'BGRA')
        out.append((name, im))
    if missing:
        print('!! 没匹配到图标的条目：')
        for m in missing:
            print('   ', m)
    return out


def alpha_bbox(im, thr=6):
    a = im.getchannel('A')
    return a.point(lambda v: 255 if v > thr else 0).getbbox()


def fit(im, draw_px, crop=True):
    """按 alpha 外接框裁掉透明留白，再等比 fit 到 draw_px（只缩不放）。"""
    src = im
    if crop:
        bb = alpha_bbox(im)
        if bb:
            src = im.crop(bb)
    k = draw_px / max(src.width, src.height)
    if k < 1.0:
        src = src.resize((max(1, int(src.width * k)), max(1, int(src.height * k))),
                         Image.LANCZOS)
    return src


def render(icons, n, icon_max, band_pad):
    """按给定参数渲染一帧（结构与 mock_ring._render 一致）。"""
    W = CANVAS * SS
    img = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    C = CANVAS / 2.0
    hub = G.HUB_R

    hs = math.sin(math.pi / max(n, 1))
    icon = icon_max
    r = icon * (G.TANG + G.GAP_K) / (2.0 * hs)
    r_cap = G.DISC_R_MAX - icon * 0.5 - band_pad
    if r > r_cap:
        r = r_cap
        icon = 2.0 * r * hs / (G.TANG + G.GAP_K)
    icon = min(max(icon, G.ICON_MIN), icon_max)
    r = max(r, G.R_ICON_MIN, hub + icon * 0.5 + 10.0)
    disc = r + icon * 0.5 + band_pad
    r_in = max(min(hub - 5.0, r - icon * 0.5 - 6.0), hub * 0.5)

    def rr(box, rad, fill=None, outline=None, w=0):
        d.rounded_rectangle([v * SS for v in box], radius=rad * SS, fill=fill,
                            outline=outline, width=max(1, int(w * SS)) if outline else 0)

    # 盘面（白）
    d.ellipse([(C - disc) * SS, (C - disc) * SS, (C + disc) * SS, (C + disc) * SS],
              fill=PAPER + (255,), outline=mix(PAPER, INK, 0.52) + (255,), width=2 * SS)
    # 环带
    d.ellipse([(C - disc) * SS, (C - disc) * SS, (C + disc) * SS, (C + disc) * SS],
              fill=mix(PAPER, INK, BAND_DARK) + (255,))
    keep = Image.new('L', (W, W), 255)
    ImageDraw.Draw(keep).ellipse(
        [(C - r_in) * SS, (C - r_in) * SS, (C + r_in) * SS, (C + r_in) * SS], fill=0)
    img.putalpha(Image.composite(img.getchannel('A'), Image.new('L', (W, W), 0), keep))
    d = ImageDraw.Draw(img)
    # 盘沿描边压在环带之上
    d.ellipse([(C - disc) * SS, (C - disc) * SS, (C + disc) * SS, (C + disc) * SS],
              outline=mix(PAPER, INK, 0.52) + (255,), width=2 * SS)

    # 图标：每格一块圆角方形底板（白底 + 描边 + 硬投影）
    for i in range(min(n, len(icons))):
        a = math.radians(i * 360.0 / n)
        cx = C + r * math.sin(a)
        cy = C - r * math.cos(a)
        half = icon / 2.0
        rad = icon * TILE_RAD_K
        rr((cx - half, cy - half + 2.5 * G.K, cx + half, cy + half + 2.5 * G.K),
           rad, fill=mix(PAPER, INK, 0.13) + (255,))
        rr((cx - half, cy - half, cx + half, cy + half), rad, fill=PAPER + (255,),
           outline=mix(PAPER, INK, 0.34) + (255,), w=1.4 * G.K)
        ic = fit(icons[i][1], int(icon * SS))
        img.alpha_composite(ic, (int(cx * SS - ic.width / 2), int(cy * SS - ic.height / 2)))

    # 中心头像球
    d = ImageDraw.Draw(img)
    d.ellipse([(C - hub - 1.5) * SS, (C - hub + 2.5) * SS,
               (C + hub - 1.5) * SS, (C + hub + 2.5) * SS],
              fill=mix(PAPER, INK, 0.13) + (255,))
    d.ellipse([(C - hub) * SS, (C - hub) * SS, (C + hub) * SS, (C + hub) * SS],
              fill=INK + (255,))
    for sx in (-1, 1):
        ex = C + sx * hub * 0.30
        ew, eh = hub * 0.115, hub * 0.175
        d.rounded_rectangle([(ex - ew) * SS, (C - eh) * SS, (ex + ew) * SS, (C + eh) * SS],
                            radius=ew * SS, fill=PAPER + (255,))
    return img, (r, icon, disc, band_pad)


def main():
    icons = _load_by_ini_order(os.path.join(HERE, '_dicondump'))
    n = len(icons)
    print('图标 %d 个，CANVAS=%d' % (n, CANVAS))
    variants = [
        ('现状', 56.0 * 0.62, 24.0),
        ('收一档 ★', 50.0 * 0.62, 28.0),
        ('收两档', 46.0 * 0.62, 32.0),
    ]
    PAD, HEAD, FOOT = 16, 46, 92   # PAD=栏间距 HEAD=标题区高 FOOT=数据区高
    f = font(15)
    fs = font(12)

    def layout(sz, extra=0):
        return Image.new('RGB', (sz[0] * 3 + PAD * 4, sz[1] + HEAD + FOOT + PAD * 2),
                         (58, 58, 64))

    # ── ① 整盘三档并排 ──
    out = layout((CANVAS, CANVAS))
    dd = ImageDraw.Draw(out)
    for k, (title, imax, bpad) in enumerate(variants):
        im, geo = render(icons, n, imax, bpad)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im)
        flat = flat.convert('RGB').resize((CANVAS, CANVAS), Image.LANCZOS)
        x = PAD + k * (CANVAS + PAD)
        out.paste(flat, (x, PAD + HEAD))
        r, icon, disc, bp = geo
        dd.text((x + 2, PAD + 4), title, fill=(246, 246, 246), font=f)
        # ⚠️ 用 multiline_text 一次画完：逐行 dd.text 的 y 是「文字顶」，
        # 12px msyh 的 bbox 从 y=3 起，三次调用很容易叠在一起。
        dd.multiline_text(
            (x + 2, PAD + HEAD + CANVAS + 14),
            '图标 %.1fpx · 盘径 %.1fpx\n'
            '盘沿留白 %.0fpx（现状 24）\n'
            '盘比现状小 %.0fpx · 间隙/图标恒为 %.2f'
            % (icon, disc, bp, 143.9 - disc, G.GAP_K),
            fill=(158, 158, 164), font=fs, spacing=7)
    p1 = os.path.join(HERE, 'shots', 'icon_size_options.png')
    out.save(p1)
    print('saved', p1, out.size)

    # ── ② 局部 2.4× 放大：取**右下方那颗图标**（连同盘沿一起），最能看出留白变化 ──
    # ⚠️ render() 返回的是 SS 倍画布，所有逻辑坐标都要先 ×SS 再裁。
    Z = 2.4
    SRCW = 108          # 逻辑像素见方
    a = math.radians(2 * 360.0 / n)

    def crop_around(flat, cx, cy):
        x0 = int((cx - SRCW / 2.0) * SS)
        y0 = int((cy - SRCW / 2.0) * SS)
        w = int(SRCW * SS)
        return flat.crop((x0, y0, x0 + w, y0 + w))

    zw = int(SRCW * Z)
    z = Image.new('RGB', (zw * 3 + PAD * 4, zw + HEAD + FOOT + PAD * 2), (58, 58, 64))
    dz = ImageDraw.Draw(z)
    for k, (title, imax, bpad) in enumerate(variants):
        im, geo = render(icons, n, imax, bpad)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im)
        flat = flat.convert('RGB')
        r_, ic_, d_, _ = geo
        C = CANVAS / 2.0
        # 窗口中心 = 图标中心与盘沿的中点 → 图标和盘沿同框
        ix, iy = C + r_ * math.sin(a), C - r_ * math.cos(a)
        mx, my = C + d_ * math.sin(a), C - d_ * math.cos(a)
        crop = crop_around(flat, (ix + mx) / 2.0, (iy + my) / 2.0)
        x = PAD + k * (zw + PAD)
        z.paste(crop.resize((zw, zw), Image.LANCZOS), (x, PAD + HEAD))
        dz.text((x + 2, PAD + 6), title, fill=(246, 246, 246), font=f)
        dz.multiline_text(
            (x + 2, PAD + HEAD + zw + 12),
            '图标 %.1fpx\n盘沿留白 %.0fpx\n盘径 %.1f' % (ic_, bpad, d_),
            fill=(158, 158, 164), font=fs, spacing=7)
    p2 = os.path.join(HERE, 'shots', 'icon_size_zoom.png')
    z.save(p2)
    print('saved', p2, z.size)


if __name__ == '__main__':
    main()
