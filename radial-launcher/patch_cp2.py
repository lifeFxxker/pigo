import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8', errors='replace').read()

old = '''    load_ini(&mut st);
    init_surfaces(&mut st);
    init_fonts(&mut st);
    synth_bg(&mut st, false);
    st.ent0 = -10.0; // 动画已收敛
    st.switch0 = -10.0;

    // 1) 静置态（无悬停）
    st.entering = false;
    st.active_app = None;
    st.hover_chip = None;
    let buf = draw_to_buf(&mut st);
    save_bmp(&format!("{}\\dump_disc.bmp", dir), &buf, CANVAS, CANVAS);'''
new = '''    load_ini(&mut st);
    eprintln!("[cp2] load_ini done, apps={}", st.apps.len());
    init_surfaces(&mut st);
    init_fonts(&mut st);
    synth_bg(&mut st, false);
    eprintln!("[cp3] synth_bg done");
    st.ent0 = -10.0; // 动画已收敛
    st.switch0 = -10.0;

    // 1) 静置态（无悬停）
    st.entering = false;
    st.active_app = None;
    st.hover_chip = None;
    eprintln!("[cp4] drawing disc");
    let buf = draw_to_buf(&mut st);
    eprintln!("[cp5] disc drawn");
    save_bmp(&format!("{}\\dump_disc.bmp", dir), &buf, CANVAS, CANVAS);'''
assert old in s, 'cpA'
s = s.replace(old, new, 1)

old = '''    // 2) 悬停态（第一个有内容的应用展开 chips + 选中环 + 压暗其他）
    if let Some(hi) = st.apps.iter().position(|a| !a.items.is_empty()) {
        st.active_app = Some(hi);
        st.hover_chip = Some(0);'''
new = '''    // 2) 悬停态（第一个有内容的应用展开 chips + 选中环 + 压暗其他）
    if let Some(hi) = st.apps.iter().position(|a| !a.items.is_empty()) {
        eprintln!("[cp6] hover app {} items {}", hi, st.apps[hi].items.len());
        st.active_app = Some(hi);
        st.hover_chip = Some(0);'''
assert old in s, 'cpB'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8').write(s)
print('ok')
