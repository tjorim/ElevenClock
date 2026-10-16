# Development-only shell integration test for the real clock-app, not the spike.
# Uses the same native appbar/pointer APIs as test-rust-overlay.ps1. Never shipped.
param(
    [Parameter(Mandatory)][string]$Inspection,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1,5)][int]$Repeats = 2,
    [switch]$SecondaryOnly
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class AutoHideTest {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct Appbar { public uint Size; public IntPtr Window; public uint Message,Edge; public Rect Bounds; public IntPtr Param; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int X,Y; public uint Data,Flags,Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit,Size=40)] struct Input { [FieldOffset(0)] public uint Type; [FieldOffset(8)] public Mouse Mouse; }
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls,string title);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr parent,IntPtr after,string cls,string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,StringBuilder text,int length);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] static extern uint SendInput(uint count,Input[] inputs,int size);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("shell32.dll")] static extern UIntPtr SHAppBarMessage(uint message,ref Appbar data);
    public static uint State() { var data=new Appbar{Size=(uint)Marshal.SizeOf(typeof(Appbar))}; return SHAppBarMessage(4,ref data).ToUInt32(); }
    public static void State(uint state) { var data=new Appbar{Size=(uint)Marshal.SizeOf(typeof(Appbar)),Window=FindWindowW("Shell_TrayWnd",null),Param=new IntPtr(state)}; SHAppBarMessage(10,ref data); }
    public static void Move(int x,int y) {
        int left=GetSystemMetrics(76),top=GetSystemMetrics(77),width=GetSystemMetrics(78),height=GetSystemMetrics(79);
        var move=new Input{Mouse=new Mouse{X=(int)((long)(x-left)*65535/(width-1)),Y=(int)((long)(y-top)*65535/(height-1)),Flags=0xc001}};
        if(SendInput(1,new[]{move},Marshal.SizeOf(typeof(Input)))!=1) throw new InvalidOperationException("Pointer movement failed");
    }
    public static long[] Taskbars() {
        var result=new System.Collections.Generic.List<long>();
        foreach(var cls in new[]{"Shell_TrayWnd","Shell_SecondaryTrayWnd"}) {
            var after=IntPtr.Zero;
            for(int i=0;i<128;i++) { after=FindWindowExW(IntPtr.Zero,after,cls,null); if(after==IntPtr.Zero) break; result.Add(after.ToInt64()); }
        }
        return result.ToArray();
    }
    public static bool OverlayHit(int x,int y,uint expectedProcess) {
        var window=WindowFromPoint(new Point{X=x,Y=y}); var text=new StringBuilder(128); uint process;
        GetClassNameW(window,text,text.Capacity); GetWindowThreadProcessId(window,out process);
        return text.ToString()=="ElevenClock-rs.Overlay" && process==expectedProcess;
    }
}
'@
[AutoHideTest]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$baseline=Get-Content -LiteralPath $Inspection -Raw | ConvertFrom-Json
$overlayProcesses=@($baseline.windows | Where-Object Class -eq 'ElevenClock-rs.Overlay' | Select-Object -ExpandProperty Process -Unique)
if ($overlayProcesses.Count -ne 1 -or $baseline.taskbars.Count -ne 2) { throw 'Expected one app and two taskbars in the inspection' }
$appProcessId=[uint32]$overlayProcesses[0]
$taskbars=@(foreach ($bar in $baseline.taskbars) {
    $matching=@(foreach ($handle in [AutoHideTest]::Taskbars()) {
        $rect=[AutoHideTest+Rect]::new()
        if ([AutoHideTest]::GetWindowRect([IntPtr]$handle,[ref]$rect) -and $rect.Left -eq $bar.taskbar.left -and $rect.Top -eq $bar.taskbar.top -and $rect.Right -eq $bar.taskbar.right -and $rect.Bottom -eq $bar.taskbar.bottom) { $handle }
    })
    if ($matching.Count -ne 1) { throw 'Taskbar geometry differs from the inspection; take a fresh inspection' }
    [pscustomobject]@{ primary=$bar.primary;handle=$matching[0];taskbar=$bar.taskbar;clock=$bar.clock }
})
function Shown($bar) {
    $rect=[AutoHideTest+Rect]::new()
    [AutoHideTest]::GetWindowRect([IntPtr]$bar.handle,[ref]$rect) -and $rect.Left -eq $bar.taskbar.left -and $rect.Top -eq $bar.taskbar.top -and $rect.Right -eq $bar.taskbar.right -and $rect.Bottom -eq $bar.taskbar.bottom
}
function Capture-Bar($bar,[string]$name) {
    $r=$bar.taskbar; $b=[Drawing.Bitmap]::new([int]($r.right-$r.left),[int]($r.bottom-$r.top)); $g=[Drawing.Graphics]::FromImage($b)
    try {
        $g.CopyFromScreen($r.left,$r.top,0,0,$b.Size)
        $b.Save((Join-Path $OutputDirectory "$name.png"),[Drawing.Imaging.ImageFormat]::Png)
        $c=$bar.clock
        foreach ($point in @(@(($c.left+1),($c.top+1)),@(($c.right-2),($c.top+1)),@(($c.left+1),($c.bottom-2)),@(($c.right-2),($c.bottom-2)))) {
            $color=$b.GetPixel(($point[0]-$r.left),($point[1]-$r.top))
            [ordered]@{ x=$point[0];y=$point[1];rgb=@($color.R,$color.G,$color.B) }
        }
    }
    finally { $g.Dispose();$b.Dispose() }
}
$originalState=[AutoHideTest]::State(); $pointer=[AutoHideTest+Point]::new(); [AutoHideTest]::GetCursorPos([ref]$pointer) | Out-Null
$results=[ordered]@{ appProcess=$appProcessId;originalState=$originalState;attempts=@();restored=$false }
try {
    [AutoHideTest]::State($originalState -bor 1)
    if (([AutoHideTest]::State() -band 1) -eq 0) { throw 'Auto-hide state did not change' }
    foreach ($repeat in 1..$Repeats) {
        foreach ($bar in @($taskbars | Where-Object { !$SecondaryOnly -or !$_.primary })) {
            # Away from either taskbar. No synthetic display/visibility messages.
            [AutoHideTest]::Move(640,200)
            $parkDeadline=[DateTime]::UtcNow.AddSeconds(8)
            do { Start-Sleep -Milliseconds 100; $shown=@($taskbars | Where-Object { Shown $_ }).Count } while ($shown -ne 0 -and [DateTime]::UtcNow -lt $parkDeadline)
            if ($shown -ne 0) { throw 'Both taskbars did not park before the independent reveal' }
            $began=[DateTime]::UtcNow
            $r=$bar.taskbar; [AutoHideTest]::Move([int](($r.left+$r.right)/2),[int]($r.bottom-1))
            $deadline=$began.AddSeconds(20); $recovered=$false; $firstShown=$null
            do {
                Start-Sleep -Milliseconds 100
                if (Shown $bar) {
                    if ($null -eq $firstShown) { $firstShown=([DateTime]::UtcNow-$began).TotalMilliseconds }
                    $c=$bar.clock
                    $recovered=[AutoHideTest]::OverlayHit([int](($c.left+$c.right)/2),[int](($c.top+$c.bottom)/2),$appProcessId)
                }
            } while (!$recovered -and [DateTime]::UtcNow -lt $deadline)
            $elapsed=([DateTime]::UtcNow-$began).TotalMilliseconds
            # WindowFromPoint can change one compositor frame before the pixels.
            # Retain the hit latency, but capture after a separate settle period.
            if ($recovered) { Start-Sleep -Milliseconds 350 }
            $name=if ($bar.primary) { "primary-$repeat" } else { "secondary-$repeat" }
            $corners=@(Capture-Bar $bar $name)
            $actualPointer=[AutoHideTest+Point]::new(); [AutoHideTest]::GetCursorPos([ref]$actualPointer) | Out-Null
            $results.attempts += [ordered]@{ primary=$bar.primary;repeat=$repeat;bothParkedBeforeReveal=$true;shownAfterMs=$firstShown;overlayHit=$recovered;sampleAfterMs=$elapsed;captureAfterMs=([DateTime]::UtcNow-$began).TotalMilliseconds;pointer=$actualPointer;corners=$corners;otherTaskbarShown=@($taskbars | Where-Object { $_.handle -ne $bar.handle -and (Shown $_) }).Count }
        }
    }
} finally {
    [AutoHideTest]::State($originalState)
    [AutoHideTest]::Move($pointer.X,$pointer.Y)
    $results.restored=([AutoHideTest]::State() -eq $originalState)
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'autohide.json') -Encoding utf8
}
$results | ConvertTo-Json -Depth 8
if (!$results.restored -or @($results.attempts | Where-Object { !$_.overlayHit -or $_.otherTaskbarShown -ne 0 }).Count -ne 0) {
    throw 'Independent reveal did not pass; see the retained measurements and captures'
}
