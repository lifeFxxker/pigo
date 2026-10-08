# -*- coding: utf-8 -*-
"""外环「梯形环」对照稿 —— 模仿 StarPie 的「经典紧凑扇区」形态。

参考（StarPie v1.6.8 设置面板的实时预览，`--dump` 拿到的 GIF 抽帧）：
  外环 = **环形扇区**（两条直的径向边 + 内弧 + 外弧），**四角大圆角**；
  段与段之间留一条窄缝；白底、无重描边、柔和投影；选中段是白底 + 粗强调色描边。
  中心是核圆。

用户要求：把外环的**胶囊**换成这种梯形环，**颜色与内环保持一致**。
内环的颜色语言 = 白底（`PAPER`）+ `mix(ink 34%)` 描边 + `0 3K` 硬投影（见 §2「统一圆角方块」）。
所以梯形段就用同一套：白底 + 34% 灰描边 + 硬投影，落在同一条环带上。

**尺寸与排布是两件事，分开对照**：
  · 排布：① 保持现有「贴身扇面」（用户之前明确要求「优先在内环对应图标的位置开始排列」）
          ② 改成「整圈均布」（StarPie 的做法，段更宽更好看）
  · 内容：横向（图标在左 + 名称在右，沿用现在的排布）/ 居中（图标居中 + 名称在下，同 StarPie）
  · 厚度：30px（现状）/ 46px（加厚，才放得下「图标 + 下置名称」）
"""
import math
import os

from PIL import Image, ImageChops, ImageDraw, ImageFont

import ring_geom as G
import mock_ring as MR

HERE = os.path.dirname(os.path.abspath(__file__))
SS = MR.SS
CANVAS = MR.CANVAS
CC = CANVAS / 2.0
K = G.K
KC = G.KC

# 与 Rust 同步的 chip 常量
CHIP_H = 34.0 * K * KC            # 30.07
CHIP_MIN_GAP = 12.0 * K * KC      # 10.6
CHIP_FAN_MAX = 240.0
CHIP_ICON = 22.0 * K * KC
CHIP_PAD_L = 6.0 * K * KC
CHIP_GAP = 8.0 * K * KC
CHIP_PAD_R = 14.0 * K * KC

WHITE = MR.PAPER
INK = MR.INK
MIX = MR.mix

# 演示用的外环条目（真实的 资源管理器/设置 只有 3~4 条，段太窄看不出形态；
# 用 6 条中等长度名字，形态最清楚）
NAMES = ['下载', '桌面', '项目', '图片', '影音', '回收站']


# ───────────────────────── 几何 ─────────────────────────

def pt(r, adeg):
    """方位角（0=上，顺时针）+ 半径 → 画布坐标（与 ring_geom.polar 同一约定）"""
    a = math.radians(adeg)
    return (CC + r * math.sin(a), CC - r * math.cos(a))


def qbez(p0, p1, p2, n=10):
    """二次贝塞尔采样 —— 用来倒四角（比真圆弧简单，视觉几乎一样）"""
    out = []
    for i in range(n + 1):
        t = i / n
        u = 1.0 - t
        out.append((u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0],
                    u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]))
    return out


def sector_path(r_in, r_out, a1, a2, cr, steps=28):
    """圆角环形扇区（梯形环的一段）。

    路径 = 外弧 → 右下角 → 右边 → 下内角 → 内弧（往回）→ 左下角 → 左边 → 左上角。
    `cr` = 四角圆角半径（像素）。弧上按**弧长**裁 `cr` → 角上按**径向**裁 `cr`。
    """
    d_out = math.degrees(cr / max(r_out, 1e-3))     # 外弧上裁掉的角
    d_in = math.degrees(cr / max(r_in, 1e-3))
    pts = []

    def arc(r, frm, to, n):
        if n <= 1:
            pts.append(pt(r, to))
            return
        for i in range(n + 1):
            pts.append(pt(r, frm + (to - frm) * i / n))

    # 1) 外弧 a1..a2（两端各裁 d_out）
    arc(r_out, a1 + d_out, a2 - d_out, steps)
    # 2) 右下角
    pts += qbez(pt(r_out, a2 - d_out), pt(r_out, a2), pt(r_out - cr, a2))
    # 3) 右边（径向，向下到内弧）
    pts.append(pt(r_in + cr, a2))
    # 4) 下内角
    pts += qbez(pt(r_in + cr, a2), pt(r_in, a2), pt(r_in, a2 - d_in))
    # 5) 内弧 a2..a1（往回）
    arc(r_in, a2 - d_in, a1 + d_in, steps)
    # 6) 左下角
    pts += qbez(pt(r_in, a1 + d_in), pt(r_in, a1), pt(r_in + cr, a1))
    # 7) 左边（径向，向上回外弧）
    pts.append(pt(r_out - cr, a1))
    # 8) 左上角
    pts += qbez(pt(r_out - cr, a1), pt(r_out, a1), pt(r_out, a1 + d_out))
    return [(x * SS, y * SS) for (x, y) in pts]


def div_full(m):
    """整圈均布：m 段等分 360°，段间留缝（StarPie 的做法）"""
    gap = 5.0                                   # 段间角度缝
    step = 360.0 / m
    out = []
    for i in range(m):
        c = -90.0 + i * step
        out.append((c - step / 2 + gap / 2, c + step / 2 - gap / 2))
    return out


def div_fan(halfs, r, a0):
    """贴身扇面：保持现有 chip_layout 的角跨（每段角宽 ∝ 名字宽度），以图标方位为中心"""
    widths = [2.0 * h for h in halfs]
    arc = sum(widths) + CHIP_MIN_GAP * (len(widths) - 1)
    span = math.degrees(arc / r)
    out = []
    acc = -span / 2.0
    for wd in widths:
        wdeg = math.degrees(wd / r)
        out.append((a0 + acc, a0 + acc + wdeg))
        acc += wdeg + math.degrees(CHIP_MIN_GAP / r)
    return out


# ───────────────────────── 绘制 ─────────────────────────

def nrm(a):
    """角度归一化到 (-90, 90] —— 只为「文字永远从左向右读」（与 Rust 的 rot 一致）"""
    a = ((a + 180.0) % 360.0) - 180.0
    if a <= -90.0:
        a += 180.0
    elif a > 90.0:
        a -= 180.0
    return a


def draw_sector(img, r_in, r_out, a1, a2, cr, fill, edge, edge_w,
                sh_dy=3.0 * K, sh_alpha=0.13):
    """画一段圆角环形扇区：先硬投影（同内环底板：`0 3K` 偏移、不模糊），再填充 + 描边"""
    poly = sector_path(r_in, r_out, a1, a2, cr)
    if sh_alpha > 0:
        m = Image.new('L', img.size, 0)
        ImageDraw.Draw(m).polygon([(x, y + sh_dy * SS) for (x, y) in poly], fill=255)
        sh = Image.new('RGBA', img.size, MIX(WHITE, INK, sh_alpha) + (0,))
        sh.putalpha(m.point(lambda v: int(v * 1.0)))
        img.alpha_composite(sh)
    d = ImageDraw.Draw(img)
    d.polygon(poly, fill=fill)
    if edge_w > 0:
        d.line(poly + [poly[0]], fill=edge, width=max(1, int(round(edge_w * SS))),
               joint='curve')
    return poly


def content_layer(text, icon, box_w, box_h, mode):
    """在**局部坐标**里画内容（+x 向右＝文字方向，−y 向外）。

    mode='row'   ：图标在左 + 名称在右（沿用现在的胶囊内部排布）
    mode='stack' ：图标居中 + 名称在下（StarPie 那套，需要环够厚）
    """
    pad = 6 * SS
    W = int(box_w + pad * 2)
    H = int(box_h + pad * 2)
    lay = Image.new('RGBA', (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(lay)
    f = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', int(11.5 * SS))
    try:
        f = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', int(11.5 * SS))
    except Exception:
        f = ImageFont.load_default()

    if mode == 'row':
        iw = CHIP_ICON * SS
        ic = icon.resize((int(iw), int(iw)), Image.LANCZOS) if icon else None
        tx = pad + iw + CHIP_GAP * SS
        if ic:
            lay.alpha_composite(ic, (int(pad), int((H - iw) / 2)))
        tw = d.textlength(text, font=f)
        d.text((tx, H / 2), text, font=f, fill=INK, anchor='lm')
    else:
        # 图标居中偏上、名称居中偏下（StarPie 那套）。
        # ⚠️ 必须**紧凑**：环厚只有 46px，内容再大就会被扇形裁掉
        # （第一版图标 21px + 字号 10.5 + 上下留白 → 名称整条被裁掉）。
        lay = Image.new('RGBA', (W, H), (0, 0, 0, 0))
        d = ImageDraw.Draw(lay)
        f2 = ImageFont.truetype('C:/Windows/Fonts/msyh.ttc', int(9.6 * SS))
        iw = 17.0 * SS
        ic = icon.resize((int(iw), int(iw)), Image.LANCZOS) if icon else None
        gapy = 1.2 * SS
        th = d.textlength(text, font=f2)
        total = iw + gapy + 10.0 * SS
        top = (H - total) / 2.0
        if ic:
            lay.alpha_composite(ic, (int((W - iw) / 2), int(top)))
        d.text((W / 2, top + iw + gapy), text, font=f2, fill=INK, anchor='ma')
    return lay


def paste_rot(img, layer, cx, cy, rot_deg, clip=None):
    """把局部坐标的内容旋转后贴到 (cx, cy)。

    局部 → 屏幕：`sx = lx·cos r − ly·sin r`、`sy = lx·sin r + ly·cos r`（y 向下的顺时针）。
    PIL 的 `rotate(+a)` 在这个坐标系里是逆时针 → 要传 `-rot_deg`。

    `clip` = 该段的扇形多边形（画布坐标）：**用它裁一下**。不裁的话长名字（如「回收站」）
    在侧面的段上会横向/纵向溢出到段外面去 —— 那不是「设计效果」，是假的。
    """
    r = layer.rotate(-rot_deg, resample=Image.BICUBIC, expand=True)
    px = int(cx * SS - r.width / 2)
    py = int(cy * SS - r.height / 2)
    if clip is None:
        img.alpha_composite(r, (px, py))
        return
    tmp = Image.new('RGBA', img.size, (0, 0, 0, 0))
    tmp.alpha_composite(r, (px, py))
    m = Image.new('L', img.size, 0)
    ImageDraw.Draw(m).polygon(clip, fill=255)
    tmp.putalpha(ImageChops.multiply(tmp.getchannel('A'), m))
    img.alpha_composite(tmp)


def icon_for(name, icons, idx):
    """外环条目没有真图标，借用应用图标示意（形态对照够用）"""
    if idx < len(icons):
        return icons[idx][1]
    return icons[0][1] if icons else None


# ───────────────────────── 四个方案 ─────────────────────────

def build(icons, n, variant):
    r_icon, tile = G.ring_layout(n)
    disc = G.disc_r(n)
    bin_ = G.band_in(n)
    bg_img = MR._render(icons, n, r_icon, disc, tile, 66.0 * K,
                        disc=WHITE, band=(bin_, disc, MIX(WHITE, INK, 0.28)))
    img = bg_img

    m = len(NAMES)
    halfs = [G.chip_half_w(t) for t in NAMES]
    a0 = -90.0                                     # 第 0 个应用的方位角
    r = max((sum(2 * h for h in halfs) + CHIP_MIN_GAP * m) / (2 * math.pi),
            disc + 3.0 * K + CHIP_H / 2.0)

    if variant == 'capsule':
        # 现状：贴弧胶囊（沿用 rot_rrect 的观感）
        for j, nm in enumerate(NAMES):
            c = -90.0 + (j - (m - 1) / 2.0) * 12.0
            cx, cy = pt(r, c)
            rot = nrm(c)
            hw, hh = G.chip_half_w(nm), CHIP_H / 2.0
            lay = Image.new('RGBA', (int((2 * hw + 8) * SS), int((2 * hh + 8) * SS)), (0, 0, 0, 0))
            dd = ImageDraw.Draw(lay)
            rr = 999
            dd.rounded_rectangle([4 * SS, 4 * SS, lay.width - 4 * SS, lay.height - 4 * SS],
                                 radius=lay.height / 2 - 4 * SS,
                                 fill=MIX(WHITE, INK, 0.02),
                                 outline=MIX(WHITE, INK, 0.45), width=max(1, int(2 * K * SS)))
            inner = content_layer(nm, icon_for(nm, icons, j), 2 * hw, CHIP_H, 'row')
            lay.alpha_composite(inner, (int(lay.width / 2 - inner.width / 2),
                                        int(lay.height / 2 - inner.height / 2)))
            paste_rot(img, lay, cx, cy, rot)
        return img

    thick = 46.0 if variant in ('stack_full', 'stack_upright') else CHIP_H
    r_in = disc + 3.0 * K
    r_out = r_in + thick
    cr = thick * (0.24 if thick > 40 else 0.30)   # 四角大圆角（参考图就是这样）

    if variant in ('fan', 'fan_thick'):
        segs = div_fan(halfs, r, a0)               # 贴身扇面（保持现有排布）
    else:                                          # full / stack_full / stack_upright
        segs = div_full(m)                         # 整圈均布（StarPie 做法）

    for j, nm in enumerate(NAMES):
        a1, a2 = segs[j]
        mid = (a1 + a2) / 2.0
        poly = draw_sector(img, r_in, r_out, a1, a2, cr,
                           fill=WHITE, edge=MIX(WHITE, INK, 0.34), edge_w=2.0 * K)
        bx, by = pt((r_in + r_out) / 2.0, mid)
        # 局部盒宽 = 这段在**中间半径**处的弧长（用来排版内容）
        arc_len = math.radians(a2 - a1) * (r_in + r_out) / 2.0
        stacky = variant in ('stack_full', 'stack_upright')
        mode = 'stack' if stacky else 'row'
        # stack 用满环厚（内容自己居中），row 留一点边距
        box_h = (thick - 2.0 * K) if stacky else (thick - 6 * K)
        lay = content_layer(nm, icon_for(nm, icons, j), arc_len, box_h, mode)
        # stack_upright：内容**不随弧转**（图标居中、名称在其下方，永远是水平的）
        # —— StarPie 的参考图就是这样（左侧段的「学习资料」也是水平的）。
        rot_deg = 0.0 if variant == 'stack_upright' else nrm(mid)
        paste_rot(img, lay, bx, by, rot_deg, clip=poly)
    return img


def main():
    icons = MR.load_icons()
    n = len(icons)
    cases = [('现状：贴弧胶囊（对照）', 'capsule'),
             ('甲 梯形环 · 贴身扇面 + 内容横向（厚 30，改动最小）', 'fan'),
             ('乙 梯形环 · 整圈均布 + 内容横向（厚 30）', 'full'),
             ('丙 梯形环 · 整圈均布 + 加厚 46 + 图标居中、名称水平（最像 StarPie）',
              'stack_upright')]

    font = None
    for fp in ('C:/Windows/Fonts/msyh.ttc', 'C:/Windows/Fonts/simhei.ttf'):
        try:
            font = ImageFont.truetype(fp, 15)
            break
        except Exception:
            pass
    if font is None:
        font = ImageFont.load_default()

    BG = (140, 140, 146)
    cells = []
    for title, v in cases:
        im = build(icons, n, v)
        flat = Image.alpha_composite(Image.new('RGBA', im.size, BG + (255,)), im)
        cells.append((title, flat.convert('RGB').resize((CANVAS, CANVAS), Image.LANCZOS)))

    pad, cap = 14, 30
    out = Image.new('RGB', (CANVAS * 2 + pad * 3, (CANVAS + cap) * 2 + pad * 3), (58, 58, 64))
    dd = ImageDraw.Draw(out)
    for k, (title, im) in enumerate(cells):
        x = pad + (k % 2) * (CANVAS + pad)
        y = pad + (k // 2) * (CANVAS + cap + pad)
        out.paste(im, (x, y + cap))
        dd.text((x + 2, y + 7), title, fill=(246, 246, 246), font=font)
    p1 = os.path.join(HERE, 'shots', 'trap_options.png')
    os.makedirs(os.path.dirname(p1), exist_ok=True)
    out.save(p1)
    print('saved', p1, out.size)

    # 放大：只看右上象限，核对梯形段的细节与图标清晰度
    Z, SRC = 2.2, 210
    zw = int(SRC * Z)
    z = Image.new('RGB', (zw * 2 + pad * 3, (zw + cap) * 2 + pad * 3), (58, 58, 64))
    dz = ImageDraw.Draw(z)
    for k, (title, im) in enumerate(cells):
        x = pad + (k % 2) * (zw + pad)
        y = pad + (k // 2) * (zw + cap + pad)
        crop = im.crop((CANVAS // 2 - 20, CANVAS // 2 - 150, CANVAS // 2 - 20 + SRC,
                        CANVAS // 2 - 150 + SRC))
        z.paste(crop.resize((zw, zw), Image.LANCZOS), (x, y + cap))
        dz.text((x + 2, y + 7), title, fill=(246, 246, 246), font=font)
    p2 = os.path.join(HERE, 'shots', 'trap_options_zoom.png')
    z.save(p2)
    print('saved', p2, z.size)


if __name__ == '__main__':
    main()