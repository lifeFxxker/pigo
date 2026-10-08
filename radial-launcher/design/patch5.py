import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 1. 紧凑几何：中心圈紧贴内环，去掉多余空隙 ──
old = '''const GEO_CTR: [f32; 2] = [22.0, 26.0];
const GEO_IN0: [f32; 2] = [58.0, 67.0];
const GEO_IN1: [f32; 2] = [98.0, 108.0];
const GEO_INMID: [f32; 2] = [78.0, 87.0];
const GEO_OUT0: [f32; 2] = [114.0, 119.0];
const GEO_OUT1: [f32; 2] = [119.0, 175.0];
const GEO_OUTMID: [f32; 2] = [117.0, 147.0];'''
new = '''const GEO_CTR: [f32; 2] = [22.0, 26.0];
const GEO_IN0: [f32; 2] = [30.0, 34.0];
const GEO_IN1: [f32; 2] = [70.0, 78.0];
const GEO_INMID: [f32; 2] = [50.0, 56.0];
const GEO_OUT0: [f32; 2] = [84.0, 88.0];
const GEO_OUT1: [f32; 2] = [118.0, 172.0];
const GEO_OUTMID: [f32; 2] = [116.0, 130.0];'''
assert old in s, '1'
s = s.replace(old, new, 1)

# ── 2. 玻璃均匀性：降低饱和/提亮，色块消失 ──
old = '''            let (lift, sat, k) = if st.glass_liquid {
                (10.0, 1.9, 14.0 / 255.0)
            } else {
                (16.0, 1.5, 20.0 / 255.0)
            };
            let mut r = br * (1.0 - k) + 255.0 * k;
            let mut g = bg * (1.0 - k) + 255.0 * k;
            let mut b = bb * (1.0 - k) + 255.0 * k;
            r = r * 1.22 + lift;
            g = g * 1.22 + lift;
            b = b * 1.22 + lift + 4.0;'''
new = '''            let (lift, sat, k, bri) = if st.glass_liquid {
                (4.0, 1.8, 10.0 / 255.0, 1.04)
            } else {
                (12.0, 1.2, 26.0 / 255.0, 1.12)
            };
            let mut r = br * (1.0 - k) + 255.0 * k;
            let mut g = bg * (1.0 - k) + 255.0 * k;
            let mut b = bb * (1.0 - k) + 255.0 * k;
            r = r * bri + lift;
            g = g * bri + lift;
            b = b * bri + lift + 2.0;'''
assert old in s, '2'
s = s.replace(old, new, 1)

# ── 3. 图标路径解析 + 双线性采样 + 24px ──
old = '''    if r == 0 {
        return None;
    }
    let hicon = sfi.hIcon;'''
new = '''    if r == 0 {
        return None;
    }
    let hicon = sfi.hIcon;
    let _ = hicon;'''
# 占位无用，直接改目标解析：在 decode_icon 开头加 SearchPathW 解析
old2 = '''    if target.contains("://") || target.starts_with("ms-") {
        return None;
    }
    let wt = wide(target);'''
new2 = '''    if target.contains("://") || target.starts_with("ms-") {
        return None;
    }
    // 裸 exe 名（notepad.exe）→ SearchPathW 解析完整路径，否则拿到的是通用图标
    let mut full: Vec<u16> = Vec::new();
    if !target.contains('\\\\') {
        let name = wide(target);
        let mut buf = [0u16; 520];
        let mut part = windows::core::PWSTR::null();
        let n = windows::Win32::Storage::FileSystem::SearchPathW(
            None,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut buf),
            Some(&mut part),
        );
        if n > 0 && (n as usize) <= buf.len() {
            full = buf[..n as usize].to_vec();
        }
    }
    let wt = if full.is_empty() { wide(target) } else { full };'''
assert old2 in s, '3a'
s = s.replace(old2, new2, 1)

old = '''fn draw_bgra_icon(buf: &mut [u8], cx: i32, cy: i32, size: f32, data: &[u8], alpha: f32) {
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
}'''
new = '''fn draw_bgra_icon(buf: &mut [u8], cx: i32, cy: i32, size: f32, data: &[u8], alpha: f32) {
    if alpha <= 0.01 || data.len() < 32 * 32 * 4 {
        return;
    }
    let half = size / 2.0;
    let ext = (half).ceil() as i32 + 1;
    for py in (cy - ext).max(0)..(cy + ext).min(CANVAS) {
        for px in (cx - ext).max(0)..(cx + ext).min(CANVAS) {
            // 双线性采样（32x32 源缩放到显示尺寸时平滑）
            let fx = ((px as f32 + 0.5 - cx as f32) / size * 32.0 - 0.5).clamp(0.0, 31.0);
            let fy = ((py as f32 + 0.5 - cy as f32) / size * 32.0 - 0.5).clamp(0.0, 31.0);
            let u0 = fx.floor() as usize;
            let v0 = fy.floor() as usize;
            let u1 = (u0 + 1).min(31);
            let v1 = (v0 + 1).min(31);
            let tu = fx - u0 as f32;
            let tv = fy - v0 as f32;
            let mut col = [0.0f32; 4];
            for (i, (uu, vv)) in [(u0, v0), (u1, v0), (u0, v1), (u1, v1)].iter().enumerate() {
                let so = (vv * 32 + uu) * 4;
                let w = if i == 0 { (1.0-tu)*(1.0-tv) } else if i == 1 { tu*(1.0-tv) } else if i == 2 { (1.0-tu)*tv } else { tu*tv };
                col[0] += data[so] as f32 * w;
                col[1] += data[so + 1] as f32 * w;
                col[2] += data[so + 2] as f32 * w;
                col[3] += data[so + 3] as f32 * w;
            }
            let a = (col[3] / 255.0) * alpha;
            if a <= 0.01 {
                continue;
            }
            let o = (py as usize * CANVAS as usize + px as usize) * 4;
            blend_over(buf, o, col[2] as u8, col[1] as u8, col[0] as u8, a);
        }
    }
}'''
assert old in s, '3b'
s = s.replace(old, new, 1)

old = '''                if let Some(data) = icon {
                    // 图标在上（22px，悬停放大 10%），标签在下
                    let size = 22.0 * (1.0 + 0.10 * anim);'''
new = '''                if let Some(data) = icon {
                    // 图标在上（24px，悬停放大 10%），标签在下
                    let size = 24.0 * (1.0 + 0.10 * anim);'''
assert old in s, '3c'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch5a ok')
