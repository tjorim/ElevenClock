# Development-only, read-only coverage evidence for the real clock-app.
# No input injection, settings changes, UIA calls or window manipulation.
param(
    [Parameter(Mandatory)][string]$ProbeCapture,
    [Parameter(Mandatory)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class TaskbarInspection {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    public class Window { public long Handle; public uint Process; public string Class; public bool Visible; public Rect Bounds; }
    delegate bool EnumProc(IntPtr window,IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr parameter);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr parent,IntPtr after,string cls,string title);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,StringBuilder text,int size);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    static Window Describe(IntPtr window) {
        var text=new StringBuilder(128); GetClassNameW(window,text,text.Capacity);
        uint process; GetWindowThreadProcessId(window,out process);
        Rect bounds; GetWindowRect(window,out bounds);
        return new Window { Handle=window.ToInt64(),Process=process,Class=text.ToString(),Visible=IsWindowVisible(window),Bounds=bounds };
    }
    public static Window Hit(int x,int y) { return Describe(WindowFromPoint(new Point { X=x,Y=y })); }
    public static Window[] All() {
        var handles=new HashSet<IntPtr>();
        EnumWindows((window,parameter)=> { handles.Add(window); return true; },IntPtr.Zero);
        foreach(var cls in new[]{"Shell_TrayWnd","Shell_SecondaryTrayWnd","ElevenClock-rs.Overlay"}) {
            var after=IntPtr.Zero;
            for(int count=0;count<128;count++) {
                after=FindWindowExW(IntPtr.Zero,after,cls,null);
                if(after==IntPtr.Zero) break;
                handles.Add(after);
            }
        }
        var result=new List<Window>();
        foreach(var handle in handles) {
            var window=Describe(handle);
            if(window.Class=="Shell_TrayWnd" || window.Class=="Shell_SecondaryTrayWnd" || window.Class=="ElevenClock-rs.Overlay") result.Add(window);
        }
        return result.ToArray();
    }
}
'@
[TaskbarInspection]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$capture = Get-Content -LiteralPath $ProbeCapture | Select-Object -Last 1 | ConvertFrom-Json
if (!$capture.result.taskbars) { throw 'The probe capture contains no taskbar inventory' }
$windows = @([TaskbarInspection]::All())
$results = @()
foreach ($taskbar in $capture.result.taskbars) {
    $r = $taskbar.taskbar_rect
    $shellClass = if ($taskbar.primary) { 'Shell_TrayWnd' } else { 'Shell_SecondaryTrayWnd' }
    $live = @($windows | Where-Object { $_.Class -eq $shellClass -and $_.Process -eq $taskbar.shell_pid -and $_.Bounds.Left -eq $r.left -and $_.Bounds.Top -eq $r.top -and $_.Bounds.Right -eq $r.right -and $_.Bounds.Bottom -eq $r.bottom })
    if ($live.Count -ne 1) { throw 'Taskbar identity/geometry differs from the capture; collect a fresh probe' }
    $elements = @($taskbar.uia.elements)
    $clockIndexes = @(for ($i=0; $i -lt $elements.Count; $i++) {
        if ($elements[$i].class -in @('SystemTray.OmniButtonLeft','SystemTray.OmniButton') -and !$elements[$i].offscreen) { $i }
    })
    if ($clockIndexes.Count -ne 1) { throw 'Expected exactly one clock candidate in the fresh capture' }
    $index = $clockIndexes[0]; $clock = $elements[$index]; $c = $clock.rect
    if (!$c -or $c.left -lt $r.left -or $c.top -lt $r.top -or $c.right -gt $r.right -or $c.bottom -gt $r.bottom) { throw 'Clock bounds are not contained in the captured taskbar' }
    $expectedOverlay=@($windows | Where-Object { $_.Class -eq 'ElevenClock-rs.Overlay' -and $_.Visible -and $_.Bounds.Left -eq $c.left -and $_.Bounds.Top -eq $c.top -and $_.Bounds.Right -eq $c.right -and $_.Bounds.Bottom -eq $c.bottom })
    $overlayHits=0; $samples=0; $firstMiss=$null
    $xs=@(for ($x=$c.left; $x -lt $c.right; $x+=4) { $x }; $c.right-1)
    $ys=@(for ($y=$c.top; $y -lt $c.bottom; $y+=4) { $y }; $c.bottom-1)
    foreach ($y in $ys) {
        foreach ($x in $xs) {
            $hit=[TaskbarInspection]::Hit($x,$y); $samples++
            if ($expectedOverlay.Count -eq 1 -and $hit.Handle -eq $expectedOverlay[0].Handle) { $overlayHits++ }
            elseif (!$firstMiss) { $firstMiss=[ordered]@{ x=$x;y=$y;hit=$hit } }
        }
    }
    $neighbours=@(foreach ($element in $elements) {
        if ($element -ne $clock -and $element.parent -eq $clock.parent -and $element.control_type -eq 50000 -and !$element.offscreen -and $element.rect) {
            $n=$element.rect; $x=[int](($n.left+$n.right)/2); $y=[int](($n.top+$n.bottom)/2)
            $nativeHits=0; $neighbourSamples=0
            $nx=@(for ($px=$n.left; $px -lt $n.right; $px+=4) { $px }; $n.right-1)
            $ny=@(for ($py=$n.top; $py -lt $n.bottom; $py+=4) { $py }; $n.bottom-1)
            foreach ($py in $ny) { foreach ($px in $nx) {
                $nativeHit=[TaskbarInspection]::Hit($px,$py); $neighbourSamples++
                if ($nativeHit.Process -eq $taskbar.shell_pid -and $nativeHit.Class -ne 'ElevenClock-rs.Overlay') { $nativeHits++ }
            } }
            $overlap=$n.left -lt $c.right -and $n.right -gt $c.left -and $n.top -lt $c.bottom -and $n.bottom -gt $c.top
            [ordered]@{ class=$element.class;rect=$n;x=$x;y=$y;hit=[TaskbarInspection]::Hit($x,$y);nativeHits=$nativeHits;samples=$neighbourSamples;overlapsClock=$overlap }
        }
    })
    $name=if ($taskbar.primary) { 'primary' } else { 'secondary' }
    $bitmap=[Drawing.Bitmap]::new([int]($r.right-$r.left),[int]($r.bottom-$r.top))
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    $corners=@()
    try {
        $graphics.CopyFromScreen($r.left,$r.top,0,0,$bitmap.Size)
        $bitmap.Save((Join-Path $OutputDirectory "$name-taskbar.png"),[Drawing.Imaging.ImageFormat]::Png)
        foreach ($point in @(@(($c.left+1),($c.top+1)),@(($c.right-2),($c.top+1)),@(($c.left+1),($c.bottom-2)),@(($c.right-2),($c.bottom-2)))) {
            $color=$bitmap.GetPixel(($point[0]-$r.left),($point[1]-$r.top))
            $corners += [ordered]@{ x=$point[0];y=$point[1];rgb=@($color.R,$color.G,$color.B) }
        }
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    $results += [pscustomobject][ordered]@{ primary=$taskbar.primary;taskbar=$r;clock=$c;overlayHits=$overlayHits;samples=$samples;firstMiss=$firstMiss;neighbours=$neighbours;corners=$corners }
}
[ordered]@{ capturedUtc=[DateTime]::UtcNow.ToString('o');windows=$windows;taskbars=$results } | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'inspection.json') -Encoding utf8
$results | Select-Object primary,overlayHits,samples,firstMiss | ConvertTo-Json -Depth 4
