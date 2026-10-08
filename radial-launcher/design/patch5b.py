import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 4. 画圆环式扫掠显现（替换逐扇区错峰） ──
old = '''    let card = st.phase == 1;
    // 逐扇区浮现因子（入场/展开时错峰，静止时全 1）
    let mut appear_in = [1.0f32; 4];
    if st.entering {
        for i in 0..4 {
            appear_in[i] = ease_out(clamp01(((now as f64 - st.ent0 - 0.05 - i as f64 * 0.05) / 0.18) as f32));
        }
    }
    let mut appear_out = [1.0f32; 12];
    if (now as f64) - st.switch0 < 0.7 {
        for i in 0..12 {
            appear_out[i] = ease_out(clamp01(((now as f64 - st.switch0 - 0.06 - i as f64 * 0.045) / 0.16) as f32));
        }
    }
    let shape_r = st.gr;'''
new = '''    let card = st.phase == 1;
    // 画圆环式扫掠显现：环沿顺时针逐渐"画"出来（软边 24°）
    let sweep_in = if st.entering {
        ((now as f64 - st.ent0 - 0.08) / 0.5).clamp(0.0, 1.0) * 400.0
    } else {
        400.0
    };
    let sweep_out = if (now as f64) - st.switch0 < 0.8 {
        ((now as f64 - st.switch0) / 0.55).clamp(0.0, 1.0) * 400.0
    } else {
        400.0
    };
    // 某角度的可见度（rel: 0=顶部顺时针）
    let sweep_alpha = |rel: f32, sweep: f32| -> f32 {
        if sweep >= 360.0 {
            1.0
        } else {
            clamp01((sweep - rel) / 24.0)
        }
    };
    let shape_r = st.gr;'''
assert old in s, '4a'
s = s.replace(old, new, 1)

# 内环扇区应用扫掠
old = '''                    let o = (py * w + px) * 4;
                    let anim = st.cat_anim[i];
                    let (cr, cg2, cb, a) = if i == st.active_cat && st.expanded {
                        let (ar, ag, ab) = State::cat_accent(i);
                        (ar, ag, ab, (0.22 + anim * 0.08) * appear_in[i])
                    } else {
                        (255, 255, 255, (0.11 + anim * 0.06) * appear_in[i])
                    };
                    blend_over(buf, o, cr, cg2, cb, cov * a);'''
new = '''                    let o = (py * w + px) * 4;
                    let anim = st.cat_anim[i];
                    let (cr, cg2, cb, a) = if i == st.active_cat && st.expanded {
                        let (ar, ag, ab) = State::cat_accent(i);
                        (ar, ag, ab, (0.22 + anim * 0.08) * sweep_alpha(rel, sweep_in))
                    } else {
                        (255, 255, 255, (0.11 + anim * 0.06) * sweep_alpha(rel, sweep_in))
                    };
                    blend_over(buf, o, cr, cg2, cb, cov * a);'''
assert old in s, '4b'
s = s.replace(old, new, 1)

# 外环扇区应用扫掠
old = '''                        blend_over(buf, o, fr as u8, fg as u8, fb as u8, cov * (0.11 + 0.19 * anim) * appear_out[i]);'''
new = '''                        blend_over(buf, o, fr as u8, fg as u8, fb as u8, cov * (0.11 + 0.19 * anim) * sweep_alpha(rel, sweep_out));'''
assert old in s, '4c'
s = s.replace(old, new, 1)

# 外环图标/标签应用扫掠
old = '''                    let ta = ((170.0 + 85.0 * anim) * ap) as u8;'''
new = '''                    let rel_mid = (mid_angle(i, cnt) + 90.0).rem_euclid(360.0);
                    let ta = ((170.0 + 85.0 * anim) * ap * sweep_alpha(rel_mid, sweep_out)) as u8;'''
assert old in s, '4f'
s = s.replace(old, new, 2)

# 外环图标 alpha 乘扫掠
old = '''                    draw_bgra_icon(buf, ix, iy, size, data, ap * (0.85 + 0.15 * anim));'''
new = '''                    let rel_mid = (mid_angle(i, cnt) + 90.0).rem_euclid(360.0);
                    draw_bgra_icon(buf, ix, iy, size, data, ap * (0.85 + 0.15 * anim) * sweep_alpha(rel_mid, sweep_out));'''
assert old in s, '4g'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch5b ok')
