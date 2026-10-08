@echo off
rem 径向启动器 —— 启动脚本
rem 双击本文件即可启动（不会重复启动已运行的实例）

setlocal
set EXE=%~dp0RadialLauncher.exe

if not exist "%EXE%" (
    echo [错误] 找不到 %EXE%
    pause
    exit /b 1
)

rem 检查是否已在运行（注意：更名前的旧版叫 RadialMenu.exe，也要查）
tasklist /FI "IMAGENAME eq RadialLauncher.exe" 2>nul | find /I "RadialLauncher.exe" >nul
if %errorlevel%==0 (
    echo [提示] 径向启动器已在运行，无需重复启动。
    pause
    exit /b 0
)
tasklist /FI "IMAGENAME eq RadialMenu.exe" 2>nul | find /I "RadialMenu.exe" >nul
if %errorlevel%==0 (
    echo [警告] 检测到旧版进程 RadialMenu.exe 还在运行。
    echo        它可能是"僵尸壳"（杀不掉、会让鼠标发卡），建议重启电脑。
    pause
)

start "" "%EXE%"
timeout /t 2 /nobreak >nul

tasklist /FI "IMAGENAME eq RadialLauncher.exe" 2>nul | find /I "RadialLauncher.exe" >nul
if %errorlevel%==0 (
    echo [成功] 径向启动器已启动。
    echo        中键 / 侧键 / 长按右键 呼出轮盘。
) else (
    echo [失败] 启动未成功，请看同目录的 radial_log.txt。
    pause
    exit /b 1
)
timeout /t 3 /nobreak >nul
endlocal
