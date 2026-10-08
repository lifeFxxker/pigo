import io

p = 'src/main.rs'
s = io.open(p, encoding='utf-8').read()

# ── 1. 中心圈缩半 + 液态玻璃常量 ──
old = 'const GEO_CTR: [f32; 2] = [45.0, 52.0];'
new = 'const GEO_CTR: [f32; 2] = [22.0, 26.0];'
assert old in s, '1a'
s = s.replace(old, new, 1)

# ── 2. imports ──
old = 'use windows::Win32::UI::Shell::{ShellExecuteW, Shell_NotifyIconW, NOTIFYICONDATAW, NIF_MESSAGE, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE};'
new = '''use windows::Win32::UI::Shell::{ShellExecuteW, Shell_NotifyIconW, NOTIFYICONDATAW, NIF_MESSAGE, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;'''
assert old in s, '2a'
s = s.replace(old, new, 1)

old = 'const MENU_QUIT: u16 = 103;'
new = '''const MENU_QUIT: u16 = 103;
const MENU_LIQUID: u16 = 104;'''
assert old in s, '2b'
s = s.replace(old, new, 1)

# ── 3. AppEntry + State 字段 ──
old = '''#[derive(Clone)]
struct ClipItem {
    text: String,
}'''
new = '''#[derive(Clone)]
struct ClipItem {
    text: String,
}

#[derive(Clone)]
struct AppEntry {
    name: String,
    target: String,
    icon: Option<Vec<u8>>, // 32x32 BGRA
}'''
assert old in s, '3a'
s = s.replace(old, new, 1)

old = '''    // 数据
    apps: Vec<(String, String)>, // 名称, 目标
    clips: Vec<ClipItem>,
    ini_path: String,'''
new = '''    // 数据
    apps: Vec<AppEntry>,
    clips: Vec<ClipItem>,
    ini_path: String,
    glass_liquid: bool,'''
assert old in s, '3b'
s = s.replace(old, new, 1)

old = '''            apps: Vec::new(),
            clips: Vec::new(),
            ini_path: String::new(),'''
new = '''            apps: Vec::new(),
            clips: Vec::new(),
            ini_path: String::new(),
            glass_liquid: false,'''
assert old in s, '3c'
s = s.replace(old, new, 1)

old = '''    fn item_label(&self, cat: usize, i: usize) -> String {
        match cat {
            0 => self.apps.get(i).map(|(n, _)| n.clone()).unwrap_or_default(),
            2 => ["打开配置文件", "重载配置"][i.min(1)].to_string(),
            _ => ["锁屏", "睡眠", "截图", "静音"][i.min(3)].to_string(),
        }
    }'''
new = '''    fn item_label(&self, cat: usize, i: usize) -> String {
        match cat {
            0 => self.apps.get(i).map(|a| a.name.clone()).unwrap_or_default(),
            2 => ["打开配置文件", "重载配置"][i.min(1)].to_string(),
            _ => ["锁屏", "睡眠", "截图", "静音"][i.min(3)].to_string(),
        }
    }
    fn item_icon(&self, cat: usize, i: usize) -> Option<&Vec<u8>> {
        if cat == 0 {
            self.apps.get(i).and_then(|a| a.icon.as_ref())
        } else {
            None
        }
    }'''
assert old in s, '3d'
s = s.replace(old, new, 1)

# ── 4. load_ini → unsafe，解码图标 ──
old = '''fn load_ini(st: &mut State) {
    st.apps.clear();
    if !std::path::Path::new(&st.ini_path).exists() {
        let _ = std::fs::write(&st.ini_path, default_ini());
    }
    if let Ok(text) = std::fs::read_to_string(&st.ini_path) {
        let mut in_apps = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_apps = line.eq_ignore_ascii_case("[apps]");
                continue;
            }
            if !in_apps || line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim());
                if !k.is_empty() && !v.is_empty() {
                    st.apps.push((k.to_string(), v.to_string()));
                }
            }
        }
    }
    if st.apps.is_empty() {
        st.apps.push(("记事本".into(), "notepad.exe".into()));
        st.apps.push(("资源管理".into(), "explorer.exe".into()));
    }
    if st.apps.len() > 12 {
        st.apps.truncate(12);
    }
}'''
new = '''unsafe fn load_ini(st: &mut State) {
    st.apps.clear();
    if std::path::Path::new(&st.ini_path).exists() {
        if let Ok(text) = std::fs::read_to_string(&st.ini_path) {
            let mut in_apps = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_apps = line.eq_ignore_ascii_case("[apps]");
                    continue;
                }
                if !in_apps || line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    if !k.is_empty() && !v.is_empty() {
                        st.apps.push(AppEntry { name: k.to_string(), target: v.to_string(), icon: None });
                    }
                }
            }
        }
    } else {
        let _ = std::fs::write(&st.ini_path, default_ini());
        for (k, v) in [("记事本", "notepad.exe"), ("计算器", "calc.exe"), ("画图", "mspaint.exe"), ("终端", "cmd.exe"), ("资源管理", "explorer.exe"), ("浏览器", "https://www.baidu.com"), ("设置", "ms-settings:")] {
            st.apps.push(AppEntry { name: k.into(), target: v.into(), icon: None });
        }
    }
    if st.apps.len() > 12 {
        st.apps.truncate(12);
    }
    for a in st.apps.iter_mut() {
        a.icon = decode_icon(&a.target);
    }
}

/// 解码目标文件的 32x32 ARGB 图标（URL / URI 无图标返回 None）
unsafe fn decode_icon(target: &str) -> Option<Vec<u8>> {
    if target.contains("://") || target.starts_with("ms-") {
        return None;
    }
    let wt = wide(target);
    let mut sfi = SHFILEINFOW::default();
    let r = SHGetFileInfoW(
        PCWSTR(wt.as_ptr()),
        FILE_ATTRIBUTE_NORMAL,
        Some(&mut sfi as *mut _ as *mut c_void),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
    );
    if r == 0 {
        return None;
    }
    let hicon = sfi.hIcon;
    let mut ii = ICONINFO::default();
    if GetIconInfo(hicon, &mut ii).is_err() {
        let _ = DestroyIcon(hicon);
        return None;
    }
    let mut out: Option<Vec<u8>> = None;
    if !ii.hbmColor.is_null() {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = 32;
        bmi.bmiHeader.biHeight = -32;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits = vec![0u8; 32 * 32 * 4];
        let hdc = GetDC(None);
        let got = GetDIBits(
            hdc,
            ii.hbmColor,
            0,
            32,
            Some(bits.as_mut_ptr() as *mut c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, hdc);
        if got > 0 && bits.chunks_exact(4).any(|c| c[3] != 0) {
            out = Some(bits);
        }
    }
    let _ = DeleteObject(ii.hbmColor);
    let _ = DeleteObject(ii.hbmMask);
    let _ = DestroyIcon(hicon);
    out
}'''
assert old in s, '4'
s = s.replace(old, new, 1)

io.open(p, 'w', encoding='utf-8', newline='\n').write(s)
print('patch4 part1 ok')
