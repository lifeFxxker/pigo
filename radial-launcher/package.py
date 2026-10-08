# -*- coding: utf-8 -*-
"""一键打包：编译 → 生成 dist/ → 打 zip。

用法（在项目根目录）：python package.py
产物：
  dist/RadialLauncher.exe      主程序（单文件）
  dist/README.txt              用法说明
  dist/径向启动器-v<版本>-win64.zip   可直接发给别人

⚠️ 打包前请确认没有实例在跑 —— 正在运行的 exe 被系统锁着，覆盖会失败
（报 "Device or resource busy" / "另一个程序正在使用此文件"）。
脚本会自动先尝试退出在跑的实例。
"""
import hashlib
import os
import shutil
import struct
import subprocess
import sys
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
DIST = os.path.join(HERE, 'dist')
BUILT = os.path.join(HERE, 'target', 'release', 'radial-launcher.exe')
NAME = 'RadialLauncher.exe'
VERSION = '0.1.0'


def sh(cmd, **kw):
    print('$', ' '.join(cmd), flush=True)
    return subprocess.run(cmd, cwd=HERE, **kw)


def kill_running():
    """先干掉在跑的实例，否则覆盖 exe 会失败。"""
    for n in (NAME, 'RadialMenu.exe'):
        subprocess.run(['taskkill', '/F', '/IM', n], capture_output=True)


def pe_subsystem(path):
    """读 PE 头的子系统字段 —— 2 = GUI（无黑窗口），3 = Console。"""
    d = open(path, 'rb').read()
    pe = struct.unpack_from('<I', d, 0x3c)[0]
    assert d[pe:pe + 4] == b'PE\x00\x00', '不是有效的 PE 文件'
    return struct.unpack_from('<H', d, pe + 24 + 68)[0]


def main():
    print('=== 1/4 编译（release，LTO + strip）===', flush=True)
    r = sh(['cargo', 'build', '--release'])
    if r.returncode != 0:
        print('编译失败')
        return 1
    if not os.path.exists(BUILT):
        print('找不到产物', BUILT)
        return 1

    print('=== 2/4 检查产物 ===', flush=True)
    subsys = pe_subsystem(BUILT)
    size = os.path.getsize(BUILT)
    md5 = hashlib.md5(open(BUILT, 'rb').read()).hexdigest()
    print('大小 %.1f KB' % (size / 1024))
    print('子系统 =', {2: 'GUI（无黑窗口）✓', 3: 'Console（会有黑窗口！）'}.get(subsys, subsys))
    print('md5 =', md5)
    if subsys != 2:
        print('⚠️ 警告：release 应该是 GUI 子系统。检查 main.rs 顶部的')
        print('   `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`')
    if size > 2 * 1024 * 1024:
        print('⚠️ 警告：体积超过 2MB，可能链接了不该有的东西')

    print('=== 3/4 生成 dist/ ===', flush=True)
    kill_running()
    os.makedirs(DIST, exist_ok=True)
    shutil.copy2(BUILT, os.path.join(DIST, NAME))
    # README 若已存在就保留（它不在构建产物里）
    if not os.path.exists(os.path.join(DIST, 'README.txt')):
        print('（dist/README.txt 不存在，跳过）')

    print('=== 4/4 打 zip ===', flush=True)
    zname = '径向启动器-v%s-win64.zip' % VERSION
    zpath = os.path.join(DIST, zname)
    with zipfile.ZipFile(zpath, 'w', zipfile.ZIP_DEFLATED) as z:
        for fn in sorted(os.listdir(DIST)):
            if fn == zname:
                continue
            z.write(os.path.join(DIST, fn), fn)
    print('已生成 %s（%.1f KB）' % (zpath, os.path.getsize(zpath) / 1024))
    print()
    print('打包完成：')
    for fn in sorted(os.listdir(DIST)):
        print('  dist/%s' % fn)
    return 0


if __name__ == '__main__':
    sys.exit(main())