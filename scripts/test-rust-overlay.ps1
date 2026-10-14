# R03 development-only evidence on an ephemeral Windows desktop. Not installed.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/debug/clock-overlay-spike.exe',
    [string]$OutputDirectory = 'native-evidence/r03',
    [string]$ProbeTarget = 'x86_64-pc-windows-msvc'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing, System.Windows.Forms, UIAutomationClient, UIAutomationTypes, WindowsBase
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class OverlayEvidence {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Appbar { public uint Size; public IntPtr Window; public uint Message,Edge; public Rect Rect; public IntPtr Param; }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls,string title);
    public delegate bool EnumProc(IntPtr window,IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr param);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,System.Text.StringBuilder text,int size);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window,out uint process);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr parent,IntPtr after,string cls,string title);
    public static IntPtr FindOverlay() {
        var found=IntPtr.Zero;
        EnumWindows((window,param)=> {var text=new System.Text.StringBuilder(256);GetClassNameW(window,text,256);if(text.ToString()=="ElevenClock.R03.Overlay") {found=window;return false;}return true;},IntPtr.Zero);
        if(found!=IntPtr.Zero) return found;
        return FindWindowExW(IntPtr.Zero,IntPtr.Zero,"ElevenClock.R03.Overlay",null);
    }
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window,out Rect rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr window,int index);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("shell32.dll")] public static extern UIntPtr SHAppBarMessage(uint message,ref Appbar data);
    [StructLayout(LayoutKind.Sequential)] public struct Mouse { public int X,Y; public uint Data,Flags,Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit,Size=40)] public struct Input { [FieldOffset(0)]public uint Type; [FieldOffset(8)]public Mouse Mouse; }
    [DllImport("user32.dll")] static extern uint SendInput(uint count,Input[] inputs,int size);
    public static void Click(int x,int y) {
        if (!SetCursorPos(x,y)) throw new InvalidOperationException("Cursor failed");
        var down=new Input { Mouse=new Mouse { Flags=2 } }; var up=new Input { Mouse=new Mouse { Flags=4 } };
        if(SendInput(2,new[]{down,up},Marshal.SizeOf(typeof(Input)))!=2) throw new InvalidOperationException("Harness click failed");
    }
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    public static void Move(int x,int y) {
        int left=GetSystemMetrics(76),top=GetSystemMetrics(77),width=GetSystemMetrics(78),height=GetSystemMetrics(79);
        var move=new Input { Mouse=new Mouse { X=(int)((long)(x-left)*65535/(width-1)),Y=(int)((long)(y-top)*65535/(height-1)),Flags=0xc001 } };
        if(SendInput(1,new[]{move},Marshal.SizeOf(typeof(Input)))!=1) throw new InvalidOperationException("Harness movement failed");
    }
    public static uint State(uint state) { var d=new Appbar { Size=(uint)Marshal.SizeOf(typeof(Appbar)),Param=new IntPtr(state) };return SHAppBarMessage(10,ref d).ToUInt32(); }
    public static uint AutoHide(bool enabled) {return State((TaskbarState()&~1u)|(enabled?1u:0u));}
    public static uint TaskbarState() { var d=new Appbar { Size=(uint)Marshal.SizeOf(typeof(Appbar)) };return SHAppBarMessage(4,ref d).ToUInt32(); }
}
'@
[OverlayEvidence]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$exe = (Resolve-Path $Executable).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path $OutputDirectory).Path
$reg = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$r02=Get-Content 'native-evidence/r02/context.json' -Raw|ConvertFrom-Json
$context = [ordered]@{nativeArchitecture=$r02.nativeArchitecture;os=$r02.os;displays=$r02.displays;source=(& git rev-parse HEAD).Trim(); build=$reg.CurrentBuildNumber; ubr=$reg.UBR; probeTarget=$ProbeTarget; executableSha256=(Get-FileHash $exe).Hash; results=[ordered]@{}; limitations=@('No production selector or P1 GO', 'Stock native x64 and mixed-DPI repetitions remain in #21', 'Screenshots require inspection; API success alone does not prove calendar visibility')}
function Save-Context { $context | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 (Join-Path $output 'context.json') }
function Capture([string]$name) {
    $r=[Windows.Forms.SystemInformation]::VirtualScreen
    $b=New-Object Drawing.Bitmap($r.Width,$r.Height);$g=[Drawing.Graphics]::FromImage($b)
    try { $g.CopyFromScreen($r.Left,$r.Top,0,0,$b.Size);$b.Save((Join-Path $output "$name.png"),[Drawing.Imaging.ImageFormat]::Png) } finally {$g.Dispose();$b.Dispose()}
}
function Overlay { [OverlayEvidence]::FindOverlay() }
function Wait-Overlay([bool]$visible, [int]$seconds=20) {
    $end=[DateTime]::UtcNow.AddSeconds($seconds)
    do { [Windows.Forms.Application]::DoEvents();$h=Overlay;if ($h -ne [IntPtr]::Zero -and [OverlayEvidence]::IsWindowVisible($h) -eq $visible) { return $h };Start-Sleep -Milliseconds 25 } while ([DateTime]::UtcNow -lt $end)
    if (!$visible -and (Overlay) -eq [IntPtr]::Zero) { return [IntPtr]::Zero }
    Capture 'wait-failed'; throw "Overlay visible=$visible not observed"
}
function Wait-Action([string]$name) {
    $deadline=[DateTime]::UtcNow.AddSeconds(12)
    do {
        $report=$null
        try {$report=Get-Content (Join-Path $output "$name.json") -Raw|ConvertFrom-Json} catch { }
        if (@($report.reports.result.action|Where-Object {$_}).Count) {return $report}
        if ($report.status -eq 'fatal_supervision_failure') {throw 'Prototype supervision failed'}
        [Windows.Forms.Application]::DoEvents();Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "No action result within bound for $name; no input retried"
}
function Set-Scale([string]$label) {
    $settings=[Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,'Settings'))
    if (!$settings) {throw 'Display Settings window disappeared'}
    $scale=$settings.FindFirst([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty,$script:scaleId))
    [OverlayEvidence]::SetForegroundWindow([IntPtr]$settings.Current.NativeWindowHandle)|Out-Null
    $scale.SetFocus();$scale.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    Start-Sleep -Milliseconds 300
    $items=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::ListItem))
    $item=$items|Where-Object {$_.Current.Name -eq $label -or $_.Current.Name -like "$label *"}|Select-Object -First 1
    if (!$item) {throw "Actual display scale $label unavailable"}
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Seconds 1
}
function Start-Spike([string]$name,[string]$selection) {
    $report=Join-Path $output "$name.json"
    Start-Process $exe -ArgumentList @($report,'180',$selection) -PassThru
}
function Stop-Spike($p) {
    if ($p -and !$p.HasExited) { $p.Kill();if (!$p.WaitForExit(3000)) {throw 'Prototype did not terminate'} }
    $deadline=[DateTime]::UtcNow.AddSeconds(3)
    do {
        $left=@(Get-CimInstance Win32_Process -Filter "Name = 'clock-overlay-spike.exe'" | Where-Object {$_.ExecutablePath -eq $exe})
        if (!$left.Count) {break};Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($left.Count) {throw 'Helper survived prototype termination'}
    if ((Overlay) -ne [IntPtr]::Zero) {throw 'Overlay survived termination'}
}
function Clock-Point($h) {
    $r=New-Object OverlayEvidence+Rect
    if (![OverlayEvidence]::GetWindowRect($h,[ref]$r)) {throw 'Missing overlay rectangle'}
    return @([int](($r.Left+$r.Right)/2),[int](($r.Top+$r.Bottom)/2))
}
$p=$null;$fixture=$null;$originalScale=$null;$settings=$null;$oldState=[OverlayEvidence]::TaskbarState();$oldReject=$env:ELEVENCLOCK_R03_REJECT_DISCOVERY
try {
    Capture 'native-baseline'
    $fixture=New-Object Windows.Forms.Form
    $fixture.Text='R03 foreground fixture';$fixture.StartPosition='Manual';$fixture.Bounds=New-Object Drawing.Rectangle(100,100,480,320)
    $fixture.Show();[Windows.Forms.Application]::DoEvents();[OverlayEvidence]::SetForegroundWindow($fixture.Handle)|Out-Null
    # Establish foreground with a real title-bar click outside the hosted Start
    # menu's bounds. SetForegroundWindow can be denied by the foreground lock.
    [OverlayEvidence]::Click(150,115);[Windows.Forms.Application]::DoEvents()
    if ([OverlayEvidence]::GetForegroundWindow() -ne $fixture.Handle) {throw 'Could not establish foreground fixture before prototype launch'}
    $p=Start-Spike 'primary' 'primary-win-n'
    $h=Wait-Overlay $true
    $point=Clock-Point $h
    $ex=[OverlayEvidence]::GetWindowLongW($h,-20)
    if (($ex -band 0x08000080) -ne 0x08000080 -or ($ex -band 0x40000)) {throw 'Unexpected activation/Alt-Tab styles'}
    if ([OverlayEvidence]::GetForegroundWindow() -ne $fixture.Handle) {throw 'Overlay display changed foreground'}
    $context.results.nonactivation=[ordered]@{passed=$true; extendedStyles=$ex; foregroundStayedOnFixture=$true}
    Capture 'primary-visible';Save-Context
    # Actual fullscreen window on the same monitor, no fake WM_* messages.
    $fixture.TopMost=$true;$fixture.FormBorderStyle='None';$fixture.Bounds=[Windows.Forms.Screen]::PrimaryScreen.Bounds
    [OverlayEvidence]::SetForegroundWindow($fixture.Handle)|Out-Null
    Wait-Overlay $false 3|Out-Null;Capture 'fullscreen-hidden'
    $context.results.fullscreenHidden=$true
    $fixture.TopMost=$false;$fixture.FormBorderStyle='Sizable';$fixture.Bounds=New-Object Drawing.Rectangle(100,100,480,320)
    Wait-Overlay $true|Out-Null
    # One physical click made by the test harness, not relayed by the overlay.
    [OverlayEvidence]::Click($point[0],$point[1]);$report=Wait-Action 'primary';Start-Sleep -Seconds 1
    Capture 'calendar-after-click' 
    $actions=@($report.reports.result.action|Where-Object {$_})
    if ($actions.Count -ne 1) {throw "Expected exactly one calendar action, observed $($actions.Count)"}
    foreach ($key in @(0x5b,0x5c,0x4e)) {if ([OverlayEvidence]::GetAsyncKeyState($key) -lt 0) {throw 'Injected shortcut left a key down'}}
    $context.results.calendar=[ordered]@{actions=$actions; oneAction=$true; noStuckKeys=$true; visualAcceptance='Pending screenshot inspection'}
    Stop-Spike $p;$p=$null;Capture 'forced-termination-native'
    $native=[Windows.Automation.AutomationElement]::FromPoint(([System.Windows.Point]::new($point[0],$point[1])))
    $context.results.forcedTermination=[ordered]@{noRemainingProcesses=$true;nativeHitClass=$native.Current.ClassName;nativeHitProcess=$native.Current.ProcessId}
    if ($native.Current.ProcessId -eq $PID) {throw 'Native clock not restored'}
    # Close the flyout by clicking our ordinary fixture before more discovery.
    [OverlayEvidence]::Click(200,200)
    # Test a separate deliberate keyboard strategy: never retry an ambiguous
    # UIA invoke with keyboard input on the same click.
    [OverlayEvidence]::Click(150,115)
    $p=Start-Spike 'keyboard-calendar' 'primary-keyboard';$h=Wait-Overlay $true
    $keyboardPoint=Clock-Point $h
    [OverlayEvidence]::Click($keyboardPoint[0],$keyboardPoint[1]);$keyboardReport=Wait-Action 'keyboard-calendar';Start-Sleep -Seconds 1
    Capture 'calendar-keyboard' 
    $keyboardActions=@($keyboardReport.reports.result.action|Where-Object {$_})
    if ($keyboardActions.Count -ne 1 -or $keyboardActions[0] -ne 'win_n_once') {throw 'Separate deliberate keyboard click did not produce exactly one Win+N'}
    foreach ($key in @(0x5b,0x5c,0x4e)) {if ([OverlayEvidence]::GetAsyncKeyState($key) -lt 0) {throw 'Keyboard strategy left a key down'}}
    $context.results.keyboardCalendar=[ordered]@{actions=$keyboardActions;oneAction=$true;noStuckKeys=$true;separateClickFromUia=$true;visualAcceptance='Pending screenshot inspection'}
    Stop-Spike $p;$p=$null
    [OverlayEvidence]::Click(150,115)
    $p=Start-Spike 'all' 'all-win-n';$h=Wait-Overlay $true;Capture 'all-visible'
    $context.results.allDisplays=[ordered]@{availableDisplays=[Windows.Forms.Screen]::AllScreens.Count; claimedMixedDpi=$false}
    # Actual auto-hide state on an ephemeral hosted desktop, restored in finally.
    if ($env:GITHUB_ACTIONS -ne 'true') {throw 'Auto-hide/restart tests require the explicitly ephemeral hosted runner'}
    [OverlayEvidence]::AutoHide($true)|Out-Null
    [OverlayEvidence]::Move(400,300)
    Wait-Overlay $false 5|Out-Null;Capture 'autohide-hidden'
    $screen=[Windows.Forms.Screen]::PrimaryScreen.Bounds
    [OverlayEvidence]::Move($point[0],$screen.Bottom-1)
    Wait-Overlay $true 20|Out-Null;Capture 'autohide-shown'
    $context.results.autoHide=[ordered]@{hidden=$true;shown=$true}
    [OverlayEvidence]::AutoHide($false)|Out-Null
    # Use the actual Settings selector for a single-display DPI transition.
    # This is not the missing mixed-DPI topology evidence in #21.
    Stop-Spike $p;$p=$null
    Start-Process 'ms-settings:display'
    $deadline=[DateTime]::UtcNow.AddSeconds(20);$scale=$null
    do {
        $settings=[Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty,'Settings'))
        if ($settings) {
            $combos=$settings.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::ComboBox))
            $scale=$combos|Where-Object {$_.Current.Name -match 'Scale|size of text' -or $_.Current.AutomationId -match 'Scale|Dpi'}|Select-Object -First 1
        }
        if (!$scale) {Start-Sleep -Milliseconds 250}
    } while (!$scale -and [DateTime]::UtcNow -lt $deadline)
    if (!$scale) {throw 'Display Scale selector unavailable'}
    $selected=$scale.GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern).Current.GetSelection()
    if ($selected.Count -ne 1) {throw 'Cannot record initial scale'}
    $originalScale=$selected[0].Current.Name;$script:scaleId=$scale.Current.AutomationId
    Set-Scale '100%'
    [OverlayEvidence]::SetForegroundWindow($fixture.Handle)|Out-Null
    $p=Start-Spike 'dpi-transition' 'primary-win-n'
    $h=Wait-Overlay $true
    $beforeDpi=[OverlayEvidence]::GetDpiForWindow($h)
    Set-Scale '125%'
    [OverlayEvidence]::SetForegroundWindow($fixture.Handle)|Out-Null
    $deadline=[DateTime]::UtcNow.AddSeconds(20)
    do {$h=Wait-Overlay $true 20;$afterDpi=[OverlayEvidence]::GetDpiForWindow($h);if ($afterDpi -eq 120) {break};Start-Sleep -Milliseconds 50} while ([DateTime]::UtcNow -lt $deadline)
    if ($beforeDpi -ne 96 -or $afterDpi -ne 120) {throw "DPI transition not observed: $beforeDpi -> $afterDpi"}
    Capture 'dpi-125';$context.results.dpiChange=[ordered]@{before=$beforeDpi;after=$afterDpi;actualSettingsSelection=$true;mixedDpi=$false}
    Set-Scale $originalScale;$originalScale=$null
    $settings.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close();$settings=$null
    Stop-Spike $p;$p=$null
    $p=Start-Spike 'all-after-dpi' 'all-win-n';Wait-Overlay $true|Out-Null
    # Neighbor controls remain physically outside the opaque slot. Record fresh
    # UIA point targets and actual click screenshots, leaving visual semantics
    # for inspection instead of claiming them from API return codes.
    Stop-Spike $p;$p=$null
    $root=[Windows.Automation.AutomationElement]::FromHandle([OverlayEvidence]::FindWindowW('Shell_TrayWnd',$null))
    $nodes=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
    $neighbors=@($nodes|Where-Object {$_.Current.ClassName -in @('SystemTray.OmniButtonRight','SystemTray.ShowDesktopButton')})
    if (!($neighbors|Where-Object {$_.Current.ClassName -eq 'SystemTray.OmniButtonRight'}) -or !($neighbors|Where-Object {$_.Current.ClassName -eq 'SystemTray.ShowDesktopButton'})) {throw 'Incomplete neighboring controls'}
    $p=Start-Spike 'neighbors' 'primary-win-n';Wait-Overlay $true|Out-Null
    $context.results.neighbors=@()
    $neighborIndex=0
    foreach ($node in $neighbors) {
        $neighborIndex++
        $r=$node.Current.BoundingRectangle;$x=[int]($r.X+$r.Width/2);$y=[int]($r.Y+$r.Height/2)
        $hit=[Windows.Automation.AutomationElement]::FromPoint(([System.Windows.Point]::new($x,$y)))
        if ($hit.Current.ProcessId -eq $p.Id) {throw 'Overlay obstructs neighboring control'}
        $context.results.neighbors+= [ordered]@{class=$node.Current.ClassName;hitClass=$hit.Current.ClassName;hitProcess=$hit.Current.ProcessId;point=@($x,$y)}
        [OverlayEvidence]::Click($x,$y);Start-Sleep -Milliseconds 750;Capture "neighbor-$neighborIndex"
        [OverlayEvidence]::Click(200,200)
    }
    # Restart only Explorer in this runner session while the spike is alive.
    $session=(Get-Process -Id $PID).SessionId
    $old=@(Get-Process explorer|Where-Object {$_.SessionId -eq $session});$old|Stop-Process -Force
    Wait-Overlay $false 3|Out-Null;Capture 'explorer-loss'
    Start-Sleep -Seconds 1
    if ([OverlayEvidence]::FindWindowW('Shell_TrayWnd',$null) -eq [IntPtr]::Zero) {Start-Process explorer.exe|Out-Null}
    Wait-Overlay $true 30|Out-Null;Capture 'explorer-recovered'
    $context.results.explorerRestart=[ordered]@{oldPids=@($old.Id);newPids=@((Get-Process explorer|Where-Object {$_.SessionId -eq $session}).Id);lossHides=$true;recovered=$true}
    Stop-Spike $p;$p=$null
    # Controlled unsupported discovery while Explorer remains unchanged. This
    # explicitly injected rejection tests removal, not a real provider failure.
    $env:ELEVENCLOCK_R03_REJECT_DISCOVERY='1'
    $p=Start-Spike 'unsupported' 'primary';Start-Sleep -Seconds 3
    if ((Overlay) -ne [IntPtr]::Zero) {throw 'Unsupported discovery created an overlay'}
    $context.results.unsupported=[ordered]@{noOverlay=$true;nativeExplorerMutation=$false;injectedRejection=$true}
    Capture 'unsupported-native'
    Stop-Spike $p;$p=$null
    Remove-Item Env:ELEVENCLOCK_R03_REJECT_DISCOVERY
    $context.status='hosted_checks_passed_visual_review_and_desktop_variants_pending'
} catch {
    $context.status='harness_failed';$context.failure=$_.Exception.Message;Capture 'failure';throw
} finally {
    if ($p) {Stop-Spike $p}
    if ($originalScale) {Set-Scale $originalScale}
    if ($settings) {$settings.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close()}
    [OverlayEvidence]::State($oldState)|Out-Null
    if ($fixture) {$fixture.Close();$fixture.Dispose()}
    [Environment]::SetEnvironmentVariable('ELEVENCLOCK_R03_REJECT_DISCOVERY',$oldReject,'Process')
    Save-Context
}
