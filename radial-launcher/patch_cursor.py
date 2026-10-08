import io
p = 'src/main.rs'
s = io.open(p, encoding='utf-8', errors='replace').read()

# ═══ 1. 修复滞留的忙碌指针：ring 窗口自己管理光标 ═══
old = '''        WM_NCHITTEST => {'''
new = '''        WM_SETCURSOR => {
            // 窗口接收鼠标后必须自设光标：拖放期间系统可能留下忙碌指针，
            // 类光标为空时 Windows 不会自动改回，于是"一直转圈"
            unsafe {
                let _ = SetCursor(LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(ptr::null_mut())));
            }
            LRESULT(1)
        }
        WM_NCHITTEST => {'''
assert old in s, 'setcursor'
s = s.replace(old, new, 1)

# ═══ 2. 落点归属：图标磁贴 + 当前应用的外环胶囊都算 ═══
old = '''                // 进入 / 移动：高亮落点应用
                0 | 1 => {
                    st.drop_mode = true;
                    DROP_MODE_ARMED.store(true, Ordering::SeqCst);
                    let px = sx - st.win_x;
                    let py = sy - st.win_y;
                    st.drop_target_app = nearest_app(st, px as f32, py as f32, 6.0);
                }'''
new = '''                // 进入 / 移动：高亮落点应用（图标磁贴，或当前应用的外环胶囊区）
                0 | 1 => {
                    st.drop_mode = true;
                    DROP_MODE_ARMED.store(true, Ordering::SeqCst);
                    let px = sx - st.win_x;
                    let py = sy - st.win_y;
                    let mut force = nearest_app(st, px as f32, py as f32, 8.0);
                    if force.is_none() {
                        if let Some(ai) = st.active_app {
                            if hit_chip_at(st, ai, px, py).is_some() {
                                force = Some(ai);
                            }
                        }
                    }
                    st.drop_target_app = force;
                }'''
assert old in s, 'drop force'
s = s.replace(old, new, 1)

# ═══ 3. 命中胶囊辅助函数（update_hover 与拖放共用）═══
old = '''/// 画布坐标 → 最近的应用磁贴（拖放落点识别）'''
new = '''/// 画布坐标 → 命中 ai 应用的外环胶囊（返回 chip 序号）
fn hit_chip_at(st: &State, ai: usize, px: f32, py: f32) -> Option<usize> {
    if ai >= st.apps.len() {
        return None;
    }
    let pts = chip_layout(st, ai);
    for (j, (cx, cy)) in pts.iter().enumerate() {
        let dx = px - cx;
        let dy = py - cy;
        let hw = chip_half_w(&st.apps[ai].items[j].name) + 4.0;
        if dx.abs() <= hw && dy.abs() <= CHIP_H2 + 4.0 {
            return Some(j);
        }
    }
    None
}

/// 画布坐标 → 最近的应用磁贴（拖放落点识别）'''
assert old in s, 'hit_chip'
s = s.replace(old, new, 1)

# ═══ 4. process_drop 落盘日志（每次入库/跳过原因可见）═══
old = '''fn process_drop(st: &mut State, items: &[DropItem], force_app: Option<usize>) -> usize {
    let mut added = 0usize;
    for it in items {'''
new = '''fn process_drop(st: &mut State, items: &[DropItem], force_app: Option<usize>) -> usize {
    let mut added = 0usize;
    for it in items {
        drag_log(&format!(
            "item [{}] target={} force={:?}",
            it.name, it.target, force_app
        ));'''
assert old in s, 'drop log head'
s = s.replace(old, new, 1)

old = '''            Some(ai) => {
                let app = &st.apps[ai];
                if app.items.iter().any(|x| x.target.eq_ignore_ascii_case(&it.target)) {
                    continue; // 已存在
                }
                if app.items.len() >= MAX_ITEMS {
                    continue; // 外环满了
                }'''
new = '''            Some(ai) => {
                let app = &st.apps[ai];
                if app.items.iter().any(|x| x.target.eq_ignore_ascii_case(&it.target)) {
                    drag_log(&format!("  skip dup -> [{}]", app.name));
                    continue; // 已存在
                }
                if app.items.len() >= MAX_ITEMS {
                    drag_log(&format!(
                        "  skip FULL ({}/{} items) -> [{}]",
                        app.items.len(),
                        MAX_ITEMS,
                        app.name
                    ));
                    continue; // 外环满了
                }'''
assert old in s, 'skip logs'
s = s.replace(old, new, 1)

old = '''            None => {
                if st.apps.iter().any(|a| a.target.eq_ignore_ascii_case(&it.target)) {
                    continue;
                }
                if st.apps.len() >= MAX_APPS {
                    continue;
                }'''
new = '''            None => {
                if st.apps.iter().any(|a| a.target.eq_ignore_ascii_case(&it.target)) {
                    drag_log("  skip dup (top-level)");
                    continue;
                }
                if st.apps.len() >= MAX_APPS {
                    drag_log("  skip FULL apps");
                    continue;
                }'''
assert old in s, 'skip logs2'
s = s.replace(old, new, 1)

old = '''    if added > 0 {
        // ini 已更新，全量热重载（重新解析 + 重新解码图标）
        unsafe { load_ini(st) };
    }
    added
}'''
new = '''    drag_log(&format!("  added={}", added));
    if added > 0 {
        // ini 已更新，全量热重载（重新解析 + 重新解码图标）
        unsafe { load_ini(st) };
    }
    added
}'''
assert old in s, 'added log'
s = s.replace(old, new, 1)

# ═══ 5. 落下处理：无新增也要日志（方便对照）═══
old = '''                    if !items.is_empty() {
                        let n = process_drop(st, &items, force_app);
                        println!("[drop] 入环 {} 项", n);'''
new = '''                    if !items.is_empty() {
                        let n = process_drop(st, &items, force_app);
                        drag_log(&format!("Drop processed, added={}", n));
                        println!("[drop] 入环 {} 项", n);'''
assert old in s, 'drop processed'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8').write(s)
print('OK')
