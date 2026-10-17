//! 把 `app.rc`（exe 图标）编成 `.res` 并交给链接器。
//!
//! 为什么不用 `embed-resource` / `winresource` 这类 crate：本项目**零第三方依赖**
//! （只有 windows crate），为了一张图标引入构建依赖不划算。直接用 Windows SDK 自带的
//! `rc.exe` 即可 —— 找得到就编，找不到就**静默跳过**（本地没装 SDK 时不该编译失败）。
//!
//! `cargo:rustc-link-arg-bins` 是稳定特性（1.56+），把 .res 直接喂给 link.exe。

use std::path::PathBuf;
use std::process::Command;

/// 在 Windows Kits 目录下找**版本最高**的 x64 rc.exe。
fn find_rc() -> Option<PathBuf> {
    let roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for root in roots {
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // 只要形如 10.0.17763.0 的版本目录
            let Ok(ver) = name.split('.').map(|s| s.parse::<u32>()).collect::<Result<Vec<_>, _>>()
            else {
                continue;
            };
            let rc = e.path().join("x64").join("rc.exe");
            if rc.is_file() && best.as_ref().map_or(true, |(bv, _)| ver > *bv) {
                best = Some((ver, rc));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.ico");

    if !cfg!(target_os = "windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=未找到 Windows SDK 的 rc.exe —— 跳过 exe 图标资源");
        return;
    };

    let res = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.res");

    // rc.exe 的相对路径按「.rc 文件所在目录」解析 → cwd 设成 crate 根即可
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("app.rc")
        .status();

    match status {
        Ok(s) if s.success() && res.is_file() => {
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
        other => {
            println!(
                "cargo:warning=rc.exe 编译 app.rc 失败（{:?}）—— 跳过 exe 图标资源",
                other
            );
        }
    }
}
