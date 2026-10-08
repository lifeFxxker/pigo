import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 5a. 托盘菜单文字拆分 ──
old = '''    let t4: Vec<u16> = wide("退出");
    let mut flag = MF_STRING.0;'''
new = '''    let t4: Vec<u16> = wide("液态玻璃模式");
    let t5: Vec<u16> = wide("退出");
    let mut flag = MF_STRING.0;'''
assert old in s, '5a'
s = s.replace(old, new, 1)

# ── 5b. 插入液态玻璃开关项 ──
old = '''    AppendMenuW(menu, mif(MF_STRING.0), MENU_RELOAD as usize, PCWSTR(t3.as_ptr()));
    AppendMenuW(menu, mif(MF_SEPARATOR.0), 0, PCWSTR::null());
    AppendMenuW(menu, mif(MF_STRING.0), MENU_QUIT as usize, PCWSTR(t4.as_ptr()));'''
new = '''    AppendMenuW(menu, mif(MF_STRING.0), MENU_RELOAD as usize, PCWSTR(t3.as_ptr()));
    let mut lflag = MF_STRING.0;
    if st.glass_liquid {
        lflag |= MF_CHECKED.0;
    }
    AppendMenuW(menu, mif(lflag), MENU_LIQUID as usize, PCWSTR(t4.as_ptr()));
    AppendMenuW(menu, mif(MF_SEPARATOR.0), 0, PCWSTR::null());
    AppendMenuW(menu, mif(MF_STRING.0), MENU_QUIT as usize, PCWSTR(t5.as_ptr()));'''
assert old in s, '5b'
s = s.replace(old, new, 1)

# ── 5c. WM_COMMAND 分支（当前已是 unsafe 调用形式） ──
old = '''                MENU_RELOAD => unsafe { load_ini(st) },'''
new = '''                MENU_RELOAD => unsafe { load_ini(st) },
                MENU_LIQUID => {
                    st.glass_liquid = !st.glass_liquid;
                    println!("[*] 液态玻璃：{}", if st.glass_liquid { "开" } else { "关" });
                }'''
assert old in s, '5c'
s = s.replace(old, new, 1)

# ── 6. 模糊半径按模式 ──
old = '''    let n = (CANVAS * CANVAS * 4) as usize;
    let w = CANVAS as usize;
    let r = 24usize;'''
new = '''    let n = (CANVAS * CANVAS * 4) as usize;
    let w = CANVAS as usize;
    let r = if st.glass_liquid { 10usize } else { 24usize };'''
assert old in s, '6'
s = s.replace(old, new, 1)

# ── 8. 入场呼吸：整环透明度+缩放涌现 ──
old = '''    if st.entering {
        let k = clamp01(((now - st.ent0) / 0.34) as f32);
        st.gs = lerp(0.8, 1.0, ease_out(k));
        set_geo(st, 0.0);
        if k >= 1.0 {
            st.entering = false;
            st.gs = 1.0;
        }
    }'''
new = '''    if st.entering {
        let k = clamp01(((now - st.ent0) / 0.34) as f32);
        st.gs = lerp(0.8, 1.0, ease_out(k));
        alpha = (alpha as f32 * (0.25 + 0.75 * ease_out(k))) as u8; // 整环呼吸式涌现
        set_geo(st, 0.0);
        if k >= 1.0 {
            st.entering = false;
            st.gs = 1.0;
        }
    }'''
new = '''    if st.entering {
        let k = clamp01(((now - st.ent0) / 0.34) as f32);
        st.gs = lerp(0.8, 1.0, ease_out(k));
        alpha = (alpha as f32 * (0.25 + 0.75 * ease_out(k))) as u8; // 整环呼吸式涌现
        if k < 1.0 {
            more = true;
        } else {
            st.entering = false;
            st.gs = 1.0;
        }
    }'''
assert old in s, '8'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch4b ok')
