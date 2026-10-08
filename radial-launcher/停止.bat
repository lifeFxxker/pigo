@echo off
rem 径向启动器 —— 停止脚本
rem 优先用 --quit 优雅退出，失败则强制结束进程

if exist "%~dp0RadialLauncher.exe" "%~dp0RadialLauncher.exe" --quit
timeout /t 1 /nobreak >nul

taskkill /F /IM RadialLauncher.exe >nul 2>&1
taskkill /F /IM RadialMenu.exe >nul 2>&1

tasklist /FI "IMAGENAME eq RadialLauncher.exe" 2>nul | find /I "RadialLauncher.exe" >nul
if %errorlevel%==0 (
    echo [失败] 进程仍在。它可能卡在系统钩子里（"僵尸壳"），只能重启电脑。
) else (
    echo [成功] 径向启动器已停止。
)
timeout /t 2 /nobreak >nul
