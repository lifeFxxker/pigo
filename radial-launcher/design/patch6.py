import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 1. 材质升级：提升黑位（均匀浅灰材质，压缩背景明暗差） ──
old = '''            let (lift, sat, k, bri) = if st.glass_liquid {
                (4.0, 1.8, 10.0 / 255.0, 1.04)
            } else {
                (12.0, 1.2, 26.0 / 255.0, 1.12)
            };
            let mut r = br * (1.0 - k) + 255.0 * k;
            let mut g = bg * (1.0 - k) + 255.0 * k;
            let mut b = bb * (1.0 - k) + 255.0 * k;
            r = r * bri + lift;
            g = g * bri + lift;
            b = b * bri + lift + 2.0;
            let l = 0.299 * r + 0.587 * g + 0.114 * b;
            r = (l + (r - l) * sat).clamp(0.0, 255.0);
            g = (l + (g - l) * sat).clamp(0.0, 255.0);
            b = (l + (b - l) * sat).clamp(0.0, 255.0);'''
new = '''            // 提升黑位：模糊背景压缩到均匀的中灰材质（macOS 深色 vibrancy 的关键）
            let (black_lift, sat, k) = if st.glass_liquid {
                (74.0, 1.35, 8.0 / 255.0)
            } else {
                (88.0, 1.2, 26.0 / 255.0)
            };
            let mut r = br * (1.0 - k) + 255.0 * k;
            let mut g = bg * (1.0 - k) + 255.0 * k;
            let mut b = bb * (1.0 - k) + 255.0 * k;
            r = r * 0.42 + black_lift;
            g = g * 0.45 + black_lift + 2.0;
            b = b * 0.45 + black_lift + 6.0;
            let l = 0.299 * r + 0.587 * g + 0.114 * b;
            r = (l + (r - l) * sat).clamp(0.0, 255.0);
            g = (l + (g - l) * sat).clamp(0.0, 255.0);
            b = (l + (b - l) * sat).clamp(0.0, 255.0);'''
assert old in s, '1'
s = s.replace(old, new, 1)

# ── 2. 移除玻璃 pass 内的色散采样（位移移到 blur 阶段做） ──
old = '''            let (br, bg, bb) = if st.glass_liquid {
                let nd = ((dx * dx + dy * dy).sqrt() / shape_r).clamp(0.0, 1.0);
                let kk = (1.0 - nd) * (1.0 - nd) * 0.10;
                let wx = (dx * 0.055).sin() * 2.5 + (dy * 0.041).sin() * 1.5;
                let wy = (dy * 0.06).sin() * 2.5 + (dx * 0.038).sin() * 1.5;
                let cl = |v: f32| (v.clamp(0.0, CANVAS as f32 - 1.0)) as usize;
                let sx = cl(CC + dx * (1.0 - kk) + wx);
                let sy = cl(CC + dy * (1.0 - kk) + wy);
                let srx = cl(CC + dx * (1.0 - kk * 1.3) + wx + 3.0);
                let sry = cl(CC + dy * (1.0 - kk * 1.3) + wy);
                let sbx = cl(CC + dx * (1.0 - kk * 0.75) + wx - 3.0);
                let sby = cl(CC + dy * (1.0 - kk * 0.75) + wy);
                let bo = (sy * w + sx) * 4;
                let bo_r = (sry * w + srx) * 4;
                let bo_b = (sby * w + sbx) * 4;
                (
                    *st.bg_bits.add(bo_r + 2) as f32,
                    *st.bg_bits.add(bo + 1) as f32,
                    *st.bg_bits.add(bo_b) as f32,
                )
            } else {
                let bo = (py * w + px) * 4;
                (
                    *st.bg_bits.add(bo + 2) as f32,
                    *st.bg_bits.add(bo + 1) as f32,
                    *st.bg_bits.add(bo) as f32,
                )
            };'''
new = '''            let bo = (py * w + px) * 4;
            (
                *st.bg_bits.add(bo + 2) as f32,
                *st.bg_bits.add(bo + 1) as f32,
                *st.bg_bits.add(bo) as f32,
            )'''
assert old in s, '2'
s = s.replace(old, new, 1)

# ── 3. blur_bg：液态 = 平滑噪声位移 + 轻模糊；毛玻璃 = 重模糊 ──
old = '''    let n = (CANVAS * CANVAS * 4) as usize;
    let w = CANVAS as usize;
    let r = if st.glass_liquid { 10usize } else { 24usize };
    let a = std::slice::from_raw_parts(st.bg_bits, n);
    let mut buf_a = a.to_vec();
    let mut buf_b = vec![0u8; n];
    for _ in 0..3 {
        box_h(&buf_a, &mut buf_b, w, r);
        box_v(&buf_b, &mut buf_a, w, r);
    }
    std::slice::from_raw_parts_mut(st.bg_bits, n).copy_from_slice(&buf_a);
}'''
new = '''    let n = (CANVAS * CANVAS * 4) as usize;
    let w = CANVAS as usize;
    let a = std::slice::from_raw_parts(st.bg_bits, n);
    let mut buf_a = a.to_vec();
    let mut buf_b = vec![0u8; n];

    if st.glass_liquid {
        // 液态玻璃：平滑噪声位移（对齐 chrome_starter 的 feTurbulence→blur→displacement）
        let (nf_x, nf_y) = noise_field();
        let mut out = vec![0u8; n];
        for y in 0..w {
            for x in 0..w {
                let ii = y * w + x;
                let off_x = nf_x[ii] * 11.0;
                let off_y = nf_y[ii] * 11.0;
                let sx = (x as f32 + off_x).clamp(0.0, (w - 1) as f32);
                let sy = (y as f32 + off_y).clamp(0.0, (w - 1) as f32);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(w - 1);
                let y1 = (y0 + 1).min(w - 1);
                let tu = sx - x0 as f32;
                let tv = sy - y0 as f32;
                let di = ii * 4;
                for c in 0..3 {
                    let p00 = buf_a[(y0 * w + x0) * 4 + c] as f32;
                    let p10 = buf_a[(y0 * w + x1) * 4 + c] as f32;
                    let p01 = buf_a[(y1 * w + x0) * 4 + c] as f32;
                    let p11 = buf_a[(y1 * w + x1) * 4 + c] as f32;
                    let val = p00 * (1.0 - tu) * (1.0 - tv)
                        + p10 * tu * (1.0 - tv)
                        + p01 * (1.0 - tu) * tv
                        + p11 * tu * tv;
                    out[di + c] = val as u8;
                }
                out[di + 3] = 255;
            }
        }
        buf_a = out;
        for _ in 0..2 {
            box_h(&buf_a, &mut buf_b, w, 6);
            box_v(&buf_b, &mut buf_a, w, 6);
        }
    } else {
        for _ in 0..3 {
            box_h(&buf_a, &mut buf_b, w, 24);
            box_v(&buf_b, &mut buf_a, w, 24);
        }
    }
    std::slice::from_raw_parts_mut(st.bg_bits, n).copy_from_slice(&buf_a);
}

/// 平滑噪声位移场（-1..1），启动后首次使用时生成一次
fn noise_field() -> &'static (Vec<f32>, Vec<f32>) {
    static N: std::sync::OnceLock<(Vec<f32>, Vec<f32>)> = std::sync::OnceLock::new();
    N.get_or_init(|| {
        let w = CANVAS as usize;
        let gs = 40usize;
        let gw = w / gs + 2;
        let mut rnd = 0x1234_5678u32;
        let mut next = || {
            rnd = rnd.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((rnd >> 16) as f32 / 32_767.0 - 1.0) as f32
        };
        let (mut gx, mut gy) = (vec![0f32; gw * gw], vec![0f32; gw * gw]);
        for v in gx.iter_mut() {
            *v = next();
        }
        for v in gy.iter_mut() {
            *v = next();
        }
        let (mut nx, mut ny) = (vec![0f32; w * w], vec![0f32; w * w]);
        let sm = |t: f32| t * t * (3.0 - 2.0 * t);
        for y in 0..w {
            for x in 0..w {
                let u = x as f32 / gs as f32;
                let v = y as f32 / gs as f32;
                let x0 = (u as usize).min(gw - 2);
                let y0 = (v as usize).min(gw - 2);
                let tu = sm(u - x0 as f32);
                let tv = sm(v - y0 as f32);
                let i00 = y0 * gw + x0;
                let i10 = i00 + 1;
                let i01 = i00 + gw;
                let i11 = i00 + gw + 1;
                let bl = |a: f32, b: f32, c: f32, d: f32| {
                    a * (1.0 - tu) * (1.0 - tv) + b * tu * (1.0 - tv) + c * (1.0 - tu) * tv + d * tu * tv
                };
                nx[y * w + x] = bl(gx[i00], gx[i10], gx[i01], gx[i11]);
                ny[y * w + x] = bl(gy[i00], gy[i10], gy[i01], gy[i11]);
            }
        }
        for _ in 0..2 {
            let t = nx.clone();
            for y in 0..w {
                for x in 1..w - 1 {
                    let i = y * w + x;
                    nx[i] = (t[i - 1] + t[i] * 2.0 + t[i + 1]) / 4.0;
                }
            }
            let t = ny.clone();
            for y in 1..w - 1 {
                for x in 0..w {
                    let i = y * w + x;
                    ny[i] = (t[i - w] + t[i] * 2.0 + t[i + w]) / 4.0;
                }
            }
        }
        (nx, ny)
    })
}'''
assert old in s, '3'
s = s.replace(old, new, 1)

# ── 4. 扫掠渲染窗口对齐 ──
old = '''    // 无动画时跳过重绘
    let animating = st.hiding || st.entering || st.expanding || st.morphing || moving;
    if animating {
        frame(st, alpha);
    }
}'''
new = '''    // 扫掠画完才允许停帧（否则最后一帧冻结在未画完状态）
    let sweep_running = (now - st.ent0) < 0.62 || (now - st.switch0) < 0.75;
    let animating = st.hiding || st.entering || st.expanding || st.morphing || moving || sweep_running;
    if animating {
        frame(st, alpha);
    }
}'''
assert old in s, '4'
s = s.replace(old, new, 1)

# ── 5. 边缘高光顶部加重 ──
old = '''                if d.abs() <= edge {
                    let a = (1.0 - d.abs() / edge) * 0.10;
                    let o = (py * w + px) * 4;
                    if buf[o + 3] > 0 {
                        blend_over(buf, o, 255, 255, 255, a);
                    }
                }'''
new = '''                if d.abs() <= edge {
                    let top_w = 0.35 + 0.65 * (((CC - dy) / shape_r).clamp(0.0, 1.0));
                    let a = (1.0 - d.abs() / edge) * 0.14 * top_w;
                    let o = (py * w + px) * 4;
                    if buf[o + 3] > 0 {
                        blend_over(buf, o, 255, 255, 255, a);
                    }
                }'''
assert old in s, '5'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch6 ok')
