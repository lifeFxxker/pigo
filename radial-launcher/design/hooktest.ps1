Add-Type -AssemblyName System.Drawing
Add-Type '
using System;
using System.Runtime.InteropServices;
public class M2 {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint data, UIntPtr extra);
}'
$null = [M2]::SetCursorPos(1500, 700)
Start-Sleep -m 250
[M2]::mouse_event(0x20, 0, 0, 0, [UIntPtr]::Zero)   # middle down
Start-Sleep -m 800
$bmp = New-Object System.Drawing.Bitmap(460, 460)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(1270, 470, 0, 0, $bmp.Size)
$g.Dispose()
$bmp.Save('D:\work_t_open.png')
$bmp.Dispose()
[M2]::mouse_event(0x40, 0, 0, 0, [UIntPtr]::Zero)   # middle up
Start-Sleep -m 700
$bmp2 = New-Object System.Drawing.Bitmap(120, 120)
$g2 = [System.Drawing.Graphics]::FromImage($bmp2)
$g2.CopyFromScreen(1440, 640, 0, 0, $bmp2.Size)
$g2.Dispose()
$bmp2.Save('D:\work_t_closed.png')
$bmp2.Dispose()
[M2]::SetCursorPos(1500, 700)
"TEST_DONE"
