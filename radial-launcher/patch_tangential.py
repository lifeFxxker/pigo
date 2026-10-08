import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8', errors='replace').read()

# ═══ 恢复贴弧旋转绘制（与圆环平行 = 胶囊沿切向倾斜）═══
old = '''                let hot_k = st.chip_hot[j];
                let a = angs[j];
                let (bx, by) = polar(rads[j] * lerp(pop, 1.0, anim), a);
                let lift = 3.4 * K * hot_k;
                let cx = bx;
                let cy = by - lift; // hover 上浮：屏幕向上（胶囊水平，无角度）
                let hw = chip_half_w(&st.apps[ai].items[j].name) * (1.0 + 0.028 * hot_k);
                let hh = CHIP_H2 * (1.0 + 0.028 * hot_k);
                let am = anim;

                // 胶囊本体：水平放置（零旋转）+ 描边 + 向下偏移的柔和投影
                rot_rrect_ex(
                    buf,
                    cx,
                    cy,
                    hw,
                    hh,
                    hh,
                    0.0,
                    mixc(pal.paper, pal.ink, 0.08 * hot_k),
                    2.0 * K,
                    mixc(mixc(pal.paper, pal.ink, 0.45), pal.ink, hot_k),
                    1.0,
                    3.0 * K,
                    0.13,
                    7.0 * K,
                    am,
                );

                // 左端图标方块：实色 accent 底 + 白色字形（水平）
                let acc = GRADS[grad_for(&st.apps[ai].items[j].name)].0;
                let isq = CHIP_ICON / 2.0;
                let icx = cx - hw + CHIP_PAD_L + isq;
                rot_rrect(buf, icx, cy, isq, isq, 6.5 * K, 0.0,
                    mixc(pal.paper, acc, 0.90), am);
                if let Some(ch) = st.apps[ai].items[j].name.chars().next() {
                    let cht = ch.to_uppercase().to_string();
                    draw_text(st, buf, icx as i32, cy as i32 - 1, &cht, 14, 600,
                        255, 255, 255, (255.0 * am) as u8, 0, 0);
                }
                // 标签：GDI 原生水平渲染
                let textw =
                    est_text_w(&st.apps[ai].items[j].name, 13.0 * K * KC, 7.0 * K * KC);
                let tx = cx - hw + CHIP_PAD_L + CHIP_ICON + CHIP_GAP + textw / 2.0;
                draw_text(st, buf, tx as i32, cy as i32, &st.apps[ai].items[j].name, 13, 500,
                    ink.0, ink.1, ink.2, (255.0 * am) as u8, 0, 0);

                if st.commit_pulse > 0.0 && st.commit_chip == j {
                    rot_rrect(buf, cx, cy, hw, hh, hh, 0.0, (255, 255, 255),
                        0.38 * st.commit_pulse);
                }'''
new = '''                let hot_k = st.chip_hot[j];
                let a = angs[j];
                let rot = rots[j].to_radians();
                let (sn, cs) = rot.sin_cos();
                let (bx, by) = polar(rads[j] * lerp(pop, 1.0, anim), a);
                // hover 上浮：沿**径向向外**抬 —— chip 贴在弧上，
                // 抬升方向必须背离圆心，否则侧面的 chip 会「横着蹭出去」。
                let lift = 3.4 * K * hot_k;
                let cx = bx + a.to_radians().sin() * lift;
                let cy = by - a.to_radians().cos() * lift;
                let hw = chip_half_w(&st.apps[ai].items[j].name) * (1.0 + 0.028 * hot_k);
                let hh = CHIP_H2 * (1.0 + 0.028 * hot_k);
                let am = anim;
                // 局部坐标 → 屏幕（+x 沿切线，−y 朝外）
                let loc = |lx: f32, ly: f32| (cx + lx * cs - ly * sn, cy + lx * sn + ly * cs);

                // 胶囊本体：贴弧旋转（与圆环平行）+ 描边 + 向下偏移的柔和投影
                rot_rrect_ex(
                    buf,
                    cx,
                    cy,
                    hw,
                    hh,
                    hh,
                    rot,
                    mixc(pal.paper, pal.ink, 0.08 * hot_k),
                    2.0 * K,
                    mixc(mixc(pal.paper, pal.ink, 0.45), pal.ink, hot_k),
                    1.0,
                    3.0 * K,
                    0.13,
                    7.0 * K,
                    am,
                );

                // 左端图标方块：实色 accent 底 + 白色字形，随胶囊一起转
                let acc = GRADS[grad_for(&st.apps[ai].items[j].name)].0;
                let isq = CHIP_ICON / 2.0;
                let (icx, icy) = loc(-hw + CHIP_PAD_L + isq, 0.0);
                rot_rrect(buf, icx, icy, isq, isq, 6.5 * K, rot,
                    mixc(pal.paper, acc, 0.90), am);
                if let Some(ch) = st.apps[ai].items[j].name.chars().next() {
                    let cht = ch.to_uppercase().to_string();
                    draw_text_rot(st, buf, icx, icy - 1.0, &cht, 14, rot, 255, 255, 255,
                        (255.0 * am) as u8);
                }
                // 标签（3 倍超采样遮罩保证旋转文字锐利）
                let textw =
                    est_text_w(&st.apps[ai].items[j].name, 13.0 * K * KC, 7.0 * K * KC);
                let (tx, ty) = loc(-hw + CHIP_PAD_L + CHIP_ICON + CHIP_GAP + textw / 2.0, 0.0);
                draw_text_rot(st, buf, tx, ty, &st.apps[ai].items[j].name, 13, rot,
                    ink.0, ink.1, ink.2, (255.0 * am) as u8);

                if st.commit_pulse > 0.0 && st.commit_chip == j {
                    rot_rrect(buf, cx, cy, hw, hh, hh, rot, (255, 255, 255),
                        0.38 * st.commit_pulse);
                }'''
assert old in s, 'draw revert'
s = s.replace(old, new, 1)
print('draw reverted to tangential')

# ═══ 恢复反旋命中判定 ═══
old = '''    // 外环 chip：与绘制一致的轴对齐盒子（胶囊水平，位置仍在弧线上）
    st.hover_chip = None;
    if let Some(ai) = st.active_app {
        if ai < n {
            let (angs, _rots, rads) = chip_layout(st, ai);
            for (j, a) in angs.iter().enumerate() {
                let (cx, cy) = polar(rads[j], *a);
                let dx = px as f32 - cx;
                let dy = py as f32 - cy;
                let hw = chip_half_w(&st.apps[ai].items[j].name) + 4.0 * K;
                if dx.abs() <= hw && dy.abs() <= CHIP_H2 + 4.0 * K {
                    st.hover_chip = Some(j);
                    break;
                }
            }
        }
    }
}'''
new = '''    // 外环 chip：反旋到 chip 局部坐标再判盒子 —— 与绘制共用 chip_layout，
    // 保证「画在哪就能点到哪」
    st.hover_chip = None;
    if let Some(ai) = st.active_app {
        if ai < n {
            let (angs, rots, rads) = chip_layout(st, ai);
            for (j, a) in angs.iter().enumerate() {
                let (cx, cy) = polar(rads[j], *a);
                let rot = rots[j].to_radians();
                let (sn, cs) = rot.sin_cos();
                let dx = px as f32 - cx;
                let dy = py as f32 - cy;
                let lx = dx * cs + dy * sn;
                let ly = -dx * sn + dy * cs;
                let hw = chip_half_w(&st.apps[ai].items[j].name) + 4.0 * K;
                if lx.abs() <= hw && ly.abs() <= CHIP_H2 + 4.0 * K {
                    st.hover_chip = Some(j);
                    break;
                }
            }
        }
    }
}'''
assert old in s, 'hit revert'
s = s.replace(old, new, 1)
print('hit reverted')

old = '''    let (angs, _rots, rads) = chip_layout(st, ai);
    for (j, a) in angs.iter().enumerate() {
        let (cx, cy) = polar(rads[j], *a);
        let dx = px - cx;
        let dy = py - cy;
        let hw = chip_half_w(&st.apps[ai].items[j].name) + 4.0;
        if dx.abs() <= hw && dy.abs() <= CHIP_H2 + 4.0 {
            return Some(j);
        }
    }
    None
}'''
new = '''    let (angs, rots, rads) = chip_layout(st, ai);
    for (j, a) in angs.iter().enumerate() {
        let (cx, cy) = polar(rads[j], *a);
        let rot = rots[j].to_radians();
        let (sn, cs) = rot.sin_cos();
        let dx = px - cx;
        let dy = py - cy;
        let lx = dx * cs + dy * sn;
        let ly = -dx * sn + dy * cs;
        let hw = chip_half_w(&st.apps[ai].items[j].name) + 4.0;
        if lx.abs() <= hw && ly.abs() <= CHIP_H2 + 4.0 {
            return Some(j);
        }
    }
    None
}'''
assert old in s, 'hit_chip_at revert'
s = s.replace(old, new, 1)
print('hit_chip_at reverted')

io.open(p, 'w', encoding='utf-8').write(s)
print('OK')
