import ctypes, time, subprocess, sys
from ctypes import wintypes as w

u = ctypes.WinDLL('user32', use_last_error=True)
k = ctypes.WinDLL('kernel32', use_last_error=True)

FindWindowW = u.FindWindowW
FindWindowW.restype = w.HWND
FindWindowW.argtypes = [w.LPCWSTR, w.LPCWSTR]
IsWindowVisible = u.IsWindowVisible
IsWindowVisible.restype = w.BOOL
IsWindowVisible.argtypes = [w.HWND]

MIDDLEDOWN, MIDDLEUP = 0x0020, 0x0040
VK_ESCAPE, VK_RIGHT, VK_RETURN = 0x1B, 0x27, 0x0D
KEYUP = 2

def ring():
    return FindWindowW('RLRing', None)

def vis():
    h = ring()
    if not h:
        return None
    return bool(IsWindowVisible(h))

def click_middle():
    u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
    time.sleep(0.05)
    u.mouse_event(MIDDLEUP, 0, 0, 0, 0)

def tap(vk):
    u.keybd_event(vk, 0, 0, 0)
    time.sleep(0.03)
    u.keybd_event(vk, 0, KEYUP, 0)

def main():
    p = subprocess.Popen([r'RadialLauncher.exe'],
                         creationflags=0x00000008 | 0x00000200,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    time.sleep(2.5)
    results = []

    # 1) 菜单初始应隐藏
    results.append(('1. 初始隐藏', vis() is False))

    # 2) 中键按下 → 菜单出现
    u.SetCursorPos(900, 500)
    time.sleep(0.2)
    u.mouse_event(MIDDLEDOWN, 0, 0, 0, 0)
    time.sleep(0.45)
    results.append(('2. 中键按下→弹出', vis() is True))

    # 3) 中键松开 → 常驻模式应保持显示
    u.mouse_event(MIDDLEUP, 0, 0, 0, 0)
    time.sleep(0.5)
    results.append(('3. 松开后保持(常驻)', vis() is True))

    # 4) 方向键 → 菜单仍在（不应崩溃）
    tap(VK_RIGHT)
    time.sleep(0.25)
    tap(VK_RIGHT)
    time.sleep(0.25)
    results.append(('4. 方向键导航后仍在', vis() is True))

    # 5) Esc → 关闭
    tap(VK_ESCAPE)
    time.sleep(0.6)
    results.append(('5. Esc→关闭', vis() is False))

    # 6) 再次呼出并确认进程存活
    click_middle()
    time.sleep(0.6)
    alive = vis() is True
    tap(VK_ESCAPE)
    time.sleep(0.4)
    results.append(('6. 二次呼出正常', alive))

    p.terminate()
    try:
        out = p.communicate(timeout=3)[0].decode('utf-8', 'ignore')
    except Exception:
        out = ''
    print('--- 交互测试 ---')
    ok = True
    for name, r in results:
        print(('  PASS  ' if r else '  FAIL  ') + name)
        ok = ok and r
    print('--- 程序输出 ---')
    print(out.strip()[:600])
    print('结果:', 'ALL PASS' if ok else 'HAS FAILURE')
    return 0 if ok else 1

sys.exit(main())
