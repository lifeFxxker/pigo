import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 9. 中心文字精简：去掉 idle 文字/dot，仅悬停时显示当前项名称 ──
old = '''        let (dr, dg, db) = State::cat_accent(st.active_cat);
        dot(buf, CC as i32, CC as i32 - 22, 3, dr, dg, db, 230);
        let name = if st.hover_cat.is_none() && !st.expanded {
            "径向启动器".to_string()
        } else {
            State::cat_name(st.active_cat).to_string()
        };
        draw_text(st, buf, CC as i32, CC as i32 - 4, &name, 15, 600, 255, 255, 255, 240, 0, 0);
        let hint = if st.expanded {
            format!("{} 个项目 · 移入外环", st.cat_count(st.active_cat))
        } else if st.hover_cat.is_some() {
            "展开外环…".to_string()
        } else {
            "悬停分类展开外环".to_string()
        };
        draw_text(st, buf, CC as i32, CC as i32 + 16, &hint, 10, 400, 255, 255, 255, 105, 0, 0);
'''
new = '''        // 中心：仅悬停时显示当前项名称（中心圈已缩小一半）
        if let Some(hc) = st.hover_cat {
            let name = if st.expanded {
                st.hover_item
                    .map(|i| st.item_label(hc, i))
                    .unwrap_or_else(|| State::cat_name(hc).to_string())
            } else {
                State::cat_name(hc).to_string()
            };
            let (ar, ag, ab) = State::cat_accent(hc);
            dot(buf, CC as i32, CC as i32 - 12, 2, ar, ag, ab, 220);
            draw_text(st, buf, CC as i32, CC as i32 + 6, &name, 10, 600, 255, 255, 255, 235, 0, 0);
        }
'''
assert old in s, '9'
s = s.replace(old, new, 1)

# ── 10. 外环条目：应用项绘制真实图标（图标在上、标签在下）；无图标居中纯文字 ──
old = '''            for i in 0..cnt {
                let a = mid_angle(i, cnt).to_radians();
                let (lx, ly) = ((CC + st.goutmid * a.cos()) as i32, (CC + st.goutmid * a.sin()) as i32);
                let label = st.item_label(cat, i);
                let anim = st.item_anim[i.min(12)];
                let ta = ((170.0 + 85.0 * anim) * appear_out[i]) as u8;
                draw_text(st, buf, lx, ly, &label, 11, 500, 255, 255, 255, ta, 0, 0);
            }'''
new = '''            for i in 0..cnt {
                let a = mid_angle(i, cnt).to_radians();
                let (px, py) = (CC + st.goutmid * a.cos(), CC + st.goutmid * a.sin());
                let label = st.item_label(cat, i);
                let anim = st.item_anim[i.min(12)];
                let ap = appear_out[i];
                let icon = st.item_icon(cat, i);
                if let Some(data) = icon {
                    // 图标在上（22px，悬停放大 10%），标签在下
                    let size = 22.0 * (1.0 + 0.10 * anim);
                    let (ix, iy) = (px as i32, py as i32 - 12);
                    draw_bgra_icon(buf, ix, iy, size, data, ap * (0.85 + 0.15 * anim));
                    let ly = py as i32 + 13;
                    let ta = ((170.0 + 85.0 * anim) * ap) as u8;
                    draw_text(st, buf, lx, ly, &label, 11, 500, 255, 255, 255, ta, 0, 0);
                } else {
                    let ta = ((170.0 + 85.0 * anim) * ap) as u8;
                    draw_text(st, buf, px as i32, py as i32, &label, 11, 500, 255, 255, 255, ta, 0, 0);
                }
            }'''
assert old in s, '10'
s = s.replace(old, new, 1)

# ── 11. draw_bgra_icon（32x32 BGRA → 任意尺寸最近邻，带透明度） ──
old = '''fn dot(buf: &mut [u8], cx: i32, cy: i32, r: i32, cr: u8, cg: u8, cb: u8, a: u8) {'''
new = '''fn draw_bgra_icon(buf: &mut [u8], cx: i32, cy: i32, size: f32, data: &[u8], alpha: f32) {
    if alpha <= 0.01 || data.len() < 32 * 32 * 4 {
        return;
    }
    let half = size / 2.0;
    let ext = (half).ceil() as i32 + 1;
    for py in (cy - ext).max(0)..(cy + ext).min(CANVAS) {
        for px in (cx - ext).max(0)..(cx + ext).min(CANVAS) {
            let fx = (px as f32 + 0.5 - cx as f32) / size * 32.0;
            let fy = (py as f32 + 0.5 - cy as f32) / size * 32.0;
            if fx < 0.0 || fy < 0.0 || fx >= 32.0 || fy >= 32.0 {
                continue;
            }
            let u = (fx as usize).min(31);
            let v = (fy as usize).min(31);
            let so = (v * 32 + u) * 4;
            let a = (data[so + 3] as f32 / 255.0) * alpha;
            if a <= 0.01 {
                continue;
            }
            let o = (py as usize * CANVAS as usize + px as usize) * 4;
            blend_over(buf, o, data[so + 2], data[so + 1], data[so], a);
        }
    }
}

fn dot(buf: &mut [u8], cx: i32, cy: i32, r: i32, cr: u8, cg: u8, cb: u8, a: u8) {'''
assert old in s, '11'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch4 part3 ok')
