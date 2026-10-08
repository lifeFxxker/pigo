# -*- coding: utf-8 -*-
"""拖拽移除交互的设计稿：紧张 / 生气 两种表情 + 取消按钮的倒计时边框。

表情参数**逐条抄自** `D:/工作/IDEA/Pisland/my-pet/common/ui/avatar/avatar_data.py`
的 `EMOTION_SEED`（上游 zhulin025/LaoA-GrokBot），单位已从 my-pet 的脸半径
114.27px 归一到本项目的球半径：

  紧张 = 慌张 (id 17)  「圆睁眼环高频轮换，目光乱晃，整球细颤」
        eyes.lookX  jitter amp 6    speed 10
        eyes.lookY  jitter amp 3    speed 11
        body.x      jitter amp 1.9464 speed 7
        body.breathe 0.0057
  生气 (id 21)「怒目圆睁，脸色在 0.25s 内迅速涨红并保持，身体细微发抖」
        body.x      jitter amp 1.0467 speed 6
        body.breathe 0.0032
        body.color  #F3F0EA → #E4574A（257ms 内涨红）

jitter 算法同样抄自 `common/ui/avatar/avatar_engine.py::_anim_jitter`：
    s = t/1000 * speed
    v = (sin(s*3.1+seed) + sin(s*5.7+seed*2.3) + sin(s*9.3+seed*4.1)) / 3 * amp

倒计时边框抄自 `objects/dialogs/dynamic_island.py::_paint_event_countdown`：
把边框的底边「点亮」为高亮色，白 = 剩余，随倒计时从起点回退；
线宽比原描边略粗（那边是手绘抖动边框），尾部 < 3px 不画。

用法：python mock_mood.py   →  shots/mood_draft.png
"""
import math
import os

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 3

# ── 与 src/main.rs 一致的常量 ──
K, KI, KC = 0.66, 0.72, 1.34
CANVAS = int(720.0 * K)          # 475
CC = CANVAS / 2
HUB_R = 66.0 * K                 # 43.6 球半径
GAP_K, TANG, BAND_PAD, BAND_DARK = 0.45, 1.22, 18.0, 0.28
ICON_MIN = 30.0
ICON_DRAW0 = 56.0 * KI
DISC_R_MAX = 220.0 * K
PAPER, INK = (255, 255, 255), (28, 28, 30)
BAND = tuple(round(PAPER[i] + (INK[i] - PAPER[i]) * BAND_DARK) for i in range(3))
EDGE = tuple(round(PAPER[i] + (INK[i] - PAPER[i]) * 0.34) for i in range(3))
HAIR = tuple(round(PAPER[i] + (INK[i] - PAPER[i]) * 0.16) for i in range(3))
ANGRY_RED = (228, 87, 74)        # #E4574A，my-pet「生气」的脸色

# 眼睛几何（抄自 src/main.rs draw_scene）
R_ORB, EYE_DX, EYE_W, EYE_H, EYE_DY = 0.92, 0.266, 0.156, 0.276, -0.055
LON0 = math.asin(EYE_DX / R_ORB)

# 取消按钮
BTN_W, BTN_H, BTN_R = 78.0, 30.0, 15.0
BTN_DY = HUB_R + 24.0            # 按钮中心离球心的距离（落在球与图标环之间的空档里）


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def ring_layout(n):
    hs = math.sin(math.pi / max(n, 1))
    icon = ICON_DRAW0
    r = icon * (TANG + GAP_K) / (2 * hs)
    cap = DISC_R_MAX - icon * 0.5 - BAND_PAD
    if r > cap:
        r, icon = cap, 2 * r * hs / (TANG + GAP_K)
    icon = min(max(icon, ICON_MIN), ICON_DRAW0)
    return max(r, 132.0 * K, HUB_R + icon * 0.5 + 10), icon


def disc_r(n):
    r, icon = ring_layout(n)
    return r + icon * 0.5 + BAND_PAD


def jitter(t_ms, speed, amp, seed=0.37):
    s = t_ms / 1000.0 * speed
    return (math.sin(s * 3.1 + seed) + math.sin(s * 5.7 + seed * 2.3)
            + math.sin(s * 9.3 + seed * 4.1)) / 3.0 * amp


# ── 绘制（全部在 SS 倍画布上，最后降采样）──
def rr(d, cx, cy, hw, hh, rad, fill=None, outline=None, w=0.0):
    d.rounded_rectangle([(cx - hw) * SS, (cy - hh) * SS, (cx + hw) * SS, (cy + hh) * SS],
                        radius=max(1, rad * SS), fill=fill, outline=outline,
                        width=max(0, round(w * SS)))


def draw_avatar(d, cx, cy, r, gx=0.0, gy=0.0, open_=1.0, esc=1.0, rot=0.0, t=0.0,
                face=INK, eye=PAPER):
    d.ellipse([(cx - r) * SS, (cy - r) * SS, (cx + r) * SS, (cy + r) * SS], fill=face)
    turn = gx * 0.30
    for side in (-1.0, 1.0):
        lon = side * LON0 + turn
        persp = min(max(math.cos(lon) / math.cos(LON0), 0.55), 1.50)
        ex = cx + R_ORB * math.sin(lon) * r
        ey = cy + (EYE_DY + gy * 0.20) * r
        ehw, ehh = EYE_W * r * persp * esc, EYE_H * r * open_ * esc
        if ehh <= 0.7:
            continue
        # 药丸：圆角 = 半宽；两只眼平行同向倾
        box = Image.new('RGBA', (max(1, round(ehw * 2 * SS)), max(1, round(ehh * 2 * SS))), (0, 0, 0, 0))
        bd = ImageDraw.Draw(box)
        bd.rounded_rectangle([0, 0, box.width - 1, box.height - 1],
                             radius=min(box.width, box.height) / 2, fill=eye + (255,))
        box = box.rotate(-math.degrees(turn), resample=Image.BICUBIC, expand=False)
        img = getattr(d, '_image', None)
        if img is not None:
            img.alpha_composite(box, (round(ex * SS - box.width / 2), round(ey * SS - box.height / 2)))


def draw_disc(d, n, dim_icons=False):
    R, icon = ring_layout(n)
    disc = disc_r(n)
    d.ellipse([(CC - disc) * SS, (CC - disc) * SS, (CC + disc) * SS, (CC + disc) * SS], fill=BAND)
    return R, icon


def draw_icon_tiles(d, n, R, icon, dim=False):
    col = mix(BAND, PAPER, 0.35) if dim else PAPER
    for i in range(n):
        a = math.radians(180 + i * 360.0 / n)      # 只画下半圈，作为间距参照
        cx = CC + R * math.sin(a)
        cy = CC - R * math.cos(a)
        rr(d, cx, cy, icon / 2, icon / 2, icon * 0.24, fill=col)


def draw_cancel(d, cx, cy, frac):
    """取消按钮 + 倒计时边框（my-pet 灵动岛同款：底边「点亮」后随剩余回退）"""
    hw, hh, rad = BTN_W / 2, BTN_H / 2, BTN_R
    rr(d, cx, cy + 2.5, hw, hh, rad, fill=mix(PAPER, INK, 0.16))      # 硬投影
    rr(d, cx, cy, hw, hh, rad, fill=PAPER, outline=mix(PAPER, INK, 0.34), w=1.6)
    # 倒计时：沿底边直线段（避开两角圆角）画一条墨色圆头线，长度 ∝ 剩余
    x0, x1 = cx - hw + rad, cx + hw - rad
    span = (x1 - x0) * max(0.0, min(1.0, frac))
    if span >= 3.0:
        y = cy + hh - 0.8
        d.line([(x0 * SS, y * SS), ((x0 + span) * SS, y * SS)],
               fill=INK, width=max(1, round(2.6 * SS)))
        r = 2.6 * SS / 2
        for xx in (x0, x0 + span):
            d.ellipse([xx * SS - r, y * SS - r, xx * SS + r, y * SS + r], fill=INK)
    # 文字
    img = d._image
    try:
        f = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', int(13 * SS))
    except Exception:
        f = ImageFont.load_default()
    txt = '取消'
    bb = d.textbbox((0, 0), txt, font=f)
    d.text((cx * SS - (bb[2] - bb[0]) / 2 - bb[0], cy * SS - (bb[3] - bb[1]) / 2 - bb[1]),
           txt, font=f, fill=INK)


def panel(title, sub, n=12, mood=None, frac=None, red=False, dim_icons=False):
    W = CANVAS * SS
    img = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d._image = img
    R, icon = draw_disc(d, n)
    draw_icon_tiles(d, n, R, icon, dim=dim_icons)

    t = 137.0                                   # 取抖动序列的一个中间时刻
    gx = gy = 0.0
    esc, open_, face = 1.0, 1.0, INK
    # ⚠️ 抖动幅度**放大了 2.5 倍**：my-pet 的脸直径 228px，本项目的球只有 87px，
    #    同样的相对位移在小球上只有 0.7px，肉眼读不出来。放大后「颤动感」才成立。
    AMP = 2.5
    if mood == 'nervous':                        # 慌张 id 17
        gx = jitter(t, 10, 0.0525 * AMP) / 0.0525   # 归一化成 [-1,1] 的注视量
        gy = jitter(t + 91, 11, 0.0263 * AMP) / 0.0263
        bx = jitter(t + 37, 7, 0.0170 * AMP)
        esc = 1.22                              # 「圆睁」：my-pet 靠眼环，这里用眼睛放大近似
                                             # （静态可读性优先：球只有 87px，眼放大是最强的静态信号）
        face = INK
    elif mood == 'angry':                        # 生气 id 21
        gx, gy = 0.0, 0.30                        # 怒目：目光下压
        bx = jitter(t + 53, 6, 0.00916 * AMP)
        esc = 1.18                              # 眼睛放大（同上，my-pet 用眼环 16/7）
        open_ = 1.0
        face = ANGRY_RED if red else INK
    else:
        bx = 0.0
    # 呼吸（my-pet 的 body.breathe 是叠加在基础呼吸上的比例，这里直接用）
    br = math.sin(t / 1000 * 2 * math.pi / 3.2) * (0.0057 if mood == 'nervous'
                                                  else 0.0032 if mood == 'angry' else 0.024)
    r = HUB_R * (1 + br)
    draw_avatar(d, CC + bx, CC, r, gx=gx, gy=gy, open_=open_, esc=esc, face=face)

    if frac is not None:
        draw_cancel(d, CC, CC + BTN_DY, frac)

    out = Image.alpha_composite(Image.new('RGBA', (W, W), (150, 150, 156, 255)), img)
    return out.convert('RGB').resize((CANVAS, CANVAS), Image.LANCZOS)


def main():
    n = 12
    cells = [
        ('① 常态', '拖动之前', panel('a', '', n)),
        ('② 紧张（拖到盘外）', '慌张 id17：目光乱晃 + 整球细颤', panel('b', '', n, mood='nervous', dim_icons=True)),
        ('③ 生气 + 取消按钮', '涨红版（my-pet 原样）· 倒计时 100%', panel('c', '', n, mood='angry', frac=1.0, red=True, dim_icons=True)),
        ('④ 生气 + 取消按钮', '保持灰阶版 · 倒计时 100%', panel('d', '', n, mood='angry', frac=1.0, red=False, dim_icons=True)),
    ]
    pad, cap, strip_h = 14, 40, 150
    out = Image.new('RGB', (CANVAS * 4 + pad * 5,
                            pad * 3 + cap + CANVAS + strip_h), (58, 58, 64))
    try:
        f = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', 16)
    except Exception:
        f = ImageFont.load_default()
    dd = ImageDraw.Draw(out)
    for k, (t1, t2, im) in enumerate(cells):
        x = pad + k * (CANVAS + pad)
        out.paste(im, (x, pad + cap))
        dd.text((x + 2, pad + 4), t1, fill=(246, 246, 246), font=f)
        dd.text((x + 2, pad + 22), t2, fill=(170, 170, 174), font=f)

    # 倒计时边框的 4 个进度
    strip = Image.new('RGBA', (CANVAS * SS, 150 * SS), (0, 0, 0, 0))
    sd = ImageDraw.Draw(strip)
    sd._image = strip
    rr(sd, 120, 60, BTN_W / 2, BTN_H / 2, BTN_R, fill=PAPER, outline=EDGE, w=1.6)
    for i, frac in enumerate((1.0, 0.66, 0.33, 0.0)):
        cx = 120 + i * 105
        draw_cancel(sd, cx, 60, frac)
    sflat = Image.alpha_composite(Image.new('RGBA', strip.size, (150, 150, 156, 255)), strip)
    sflat = sflat.convert('RGB').resize((CANVAS, 150), Image.LANCZOS)
    sy = pad * 2 + cap + CANVAS
    out.paste(sflat, (pad, sy))
    dd.text((pad + 2, sy + 6), '倒计时边框：剩余 100% / 66% / 33% / 0%（墨色＝剩余，从左端回退；my-pet 灵动岛同款）',
            fill=(200, 200, 204), font=f)
    _pre = out.copy()

    # 2× 头像放大对比（静态只能靠眼睛形状/位置区分，抖动是动效，静帧看不出来）
    Z = 2.2
    zs = round(HUB_R * 2.6 * Z)
    zy = out.size[1]
    out = Image.new('RGB', (out.size[0], zy + zs + cap + pad), (58, 58, 64))
    out.paste(_pre, (0, 0))
    dd3 = ImageDraw.Draw(out)
    for k, (t1, _t2, im) in enumerate(cells):
        half_src = round(zs / 2 / SS)
        sub = im.crop((round(CC) - half_src, round(CC) - half_src,
                       round(CC) - half_src + half_src * 2, round(CC) - half_src + half_src * 2))
        sub = sub.resize((zs, zs), Image.LANCZOS)
        x = pad + k * (CANVAS + pad)
        out.paste(sub, (x, zy + cap))
        dd3.text((x + 2, zy + 4), t1, fill=(246, 246, 246), font=f)
    dd3.text((pad + 2, zy + 22), '头像 2.2× 放大：能看出的是眼睛的睁大与位置；抖动是动效，静帧看不出来',
             fill=(170, 170, 174), font=f)

    path = os.path.join(HERE, 'shots', 'mood_draft.png')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    out.save(path)
    print('saved', path, out.size)


if __name__ == '__main__':
    main()
