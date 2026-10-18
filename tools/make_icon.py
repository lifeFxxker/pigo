# -*- coding: utf-8 -*-
"""用中心头像球的**真实几何参数**生成 app.ico。

参数逐条抄自 src/main.rs 的「中心头像：GrokBot 式表情球」：
    球 = ink（近黑实心，无渐变无描边），眼睛 = paper（白色药丸）
    R_ORB 0.92 / EYE_DX 0.266 / EYE_W 0.156 / EYE_H 0.276 / EYE_DY -0.055
    lon0 = asin(EYE_DX / R_ORB)；眼位 x = R_ORB·sin(lon)·R，透视 = cos(lon)/cos(lon0)
静止常态（calm）：gaze=0、open=1、esc=1 → persp = 1。

⚠️ 小尺寸必须**补偿眼睛比例**：16px 时按原始比例眼睛只有 1.1×1.9px，
   缩完就糊成一团灰点。这里按尺寸放大 eye_scale（并在最后保证最小 2px 宽）。
⚠️ 每个尺寸都用 4× 超采样绘制再 LANCZOS 缩回 —— 直接画会出锯齿台阶。
"""
import math
import os

from PIL import Image, ImageDraw

# ── 与 Rust 一致的调色（亮色主题）──
INK = (28, 28, 30)      # #1C1C1E  pal.ink
PAPER = (255, 255, 255)  # pal.paper

# ── 与 Rust 一致的几何（相对球半径 R）──
R_ORB = 0.92
EYE_DX = 0.266
EYE_W = 0.156
EYE_H = 0.276
EYE_DY = -0.055
LON0 = math.asin(EYE_DX / R_ORB)

BALL_K = 0.44           # 球半径 = 画布边长 × 这个系数（直径占 88%，留边）

# 小尺寸的眼睛补偿。
# ⚠️ 别补偿过头：眼睛变宽的同时**间距是固定的**（0.266R），放大太多两只眼会粘在一起
#    （第一版 16px 用 1.55 → 间隙只剩 0.35px，缩完就是一团白块）。
def eye_scale(s):
    if s >= 64:
        return 1.00
    if s >= 48:
        return 1.02
    if s >= 40:
        return 1.06
    if s >= 32:
        return 1.10
    if s >= 24:
        return 1.16
    if s >= 20:
        return 1.22
    return 1.28          # 16px


def draw(size, ss=4, ring=None, eye_k=None):
    """画一张 size×size 的头像。

    ring  = (r,g,b,a) 描边色；None = 不加描边。
            ⚠️ 用**极淡的白色描边**（而不是灰）：浅色底上白边与背景融合 → 看不见；
               深色任务栏底上白边刚好勾出轮廓 → 球还在。纯黑球在深色任务栏上会消失。
    eye_k = 手动指定 eye_scale（None = 按尺寸自动）。
    """
    S = size * ss
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    R = S * BALL_K
    cx = cy = S / 2.0

    if ring:
        # 描边宽度固定 ~1 物理像素（不能随尺寸缩成看不见）
        w = max(S * 0.010, ss * 0.9)
        d.ellipse([cx - R - w, cy - R - w, cx + R + w, cy + R + w], fill=ring)

    d.ellipse([cx - R, cy - R, cx + R, cy + R], fill=INK + (255,))

    es = eye_scale(size) if eye_k is None else eye_k
    ehw = EYE_W * R * es
    ehh = EYE_H * R * es
    ehw = max(ehw, S * 0.050)           # 最小宽度（16px 下约 0.8px → 提到 ~3.2px）
    for side in (-1.0, 1.0):
        lon = side * LON0
        persp = math.cos(lon) / math.cos(LON0)
        ex = cx + R_ORB * math.sin(lon) * R
        ey = cy + EYE_DY * R
        w = ehw * persp
        h = ehh
        rad = min(w, h)                  # 圆角 = 半宽 → 胶囊
        d.rounded_rectangle([ex - w, ey - h, ex + w, ey + h],
                            radius=rad, fill=PAPER + (255,))

    return img.resize((size, size), Image.LANCZOS)


def pack_ico(images):
    """把若干 RGBA 图打成标准 .ico（每个尺寸**独立编码**，不互相缩放）。

    ⚠️ 不能用 `im.save(f, 'ICO', sizes=[...], append_images=[...])`：
       PIL 会**忽略 append_images**，只从主图缩放出各尺寸 —— 于是小尺寸
       丢掉了各自的眼形补偿（实测：存完只剩一帧 16×16）。
       这里手写 ICONDIR + ICONDIRENTRY + PNG 数据，最可控。
    """
    import io
    import struct

    blobs = []
    for im in images:
        b = io.BytesIO()
        im.save(b, 'PNG')
        blobs.append((im.width, im.height, b.getvalue()))

    out = bytearray()
    out += struct.pack('<HHH', 0, 1, len(blobs))        # reserved, type=icon, count
    off = 6 + 16 * len(blobs)
    for w, h, data in blobs:
        out += struct.pack(
            '<BBBBHHII',
            w if w < 256 else 0,                        # 256 要写成 0
            h if h < 256 else 0,
            0, 0,                                       # colorCount, reserved
            1, 32,                                      # planes, bitCount
            len(data), off)
        off += len(data)
    for _, _, data in blobs:
        out += data
    return bytes(out)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    # 极淡的白色描边：浅色底上与背景融合（看不见），深色任务栏底上勾出轮廓（球还在）
    RING = (255, 255, 255, 64)

    # ── 默认（带极淡白描边）→ exe 图标 + 托盘图标 ──
    frames = [draw(s, ring=RING) for s in sizes]
    ico = os.path.join(here, 'app.ico')
    open(ico, 'wb').write(pack_ico(frames))
    print('已写出 %s  （%d 个尺寸：%s，%.1f KB）'
          % (ico, len(sizes), sizes, os.path.getsize(ico) / 1024))

    # ── 纯版（无描边）—— 在深色任务栏上会看不见，需要时手动换 ──
    plain = [draw(s) for s in sizes]
    ico2 = os.path.join(here, 'app_plain.ico')
    open(ico2, 'wb').write(pack_ico(plain))
    print('已写出 %s（纯版，无描边）' % ico2)

    # ── 预览：左半 = 浅色底，右半 = 深色底；每半上排带描边、下排纯版 ──
    prev_sizes = [16, 20, 24, 32, 48, 64, 128]
    Z = 5
    pad = 16
    head = 24
    cell_h = max(s * Z for s in prev_sizes)
    half_w = sum(s * Z for s in prev_sizes) + pad * (len(prev_sizes) + 1)
    W = half_w * 2
    H = head + (cell_h + pad) * 2 + pad
    sheet = Image.new('RGB', (W, H), (58, 58, 66))
    sd = ImageDraw.Draw(sheet)
    sd.text((pad, 6), '左：浅色底（资源管理器 / 浅色任务栏）', fill=(235, 235, 240))
    sd.text((half_w + pad, 6), '右：深色底（深色任务栏）', fill=(235, 235, 240))
    for half, bg in ((0, (238, 238, 242)), (1, (32, 32, 36))):
        x0 = half * half_w
        for row, ring in ((0, RING), (1, None)):
            x = x0 + pad
            y = head + row * (cell_h + pad)
            for s in prev_sizes:
                tile = Image.new('RGB', (s * Z, s * Z), bg)
                im = draw(s, ring=ring).resize((s * Z, s * Z), Image.NEAREST)
                tile.paste(im, (0, 0), im)
                sheet.paste(tile, (x, y + (cell_h - s * Z) // 2))
                x += s * Z + pad
    sheet.save(os.path.join(here, '_icon_preview.png'))
    print('预览 → _icon_preview.png', sheet.size)
    print('（每半上排 = 带描边，下排 = 纯版）')


if __name__ == '__main__':
    main()
