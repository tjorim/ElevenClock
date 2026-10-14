# Development-only harness for ephemeral Windows desktops; never installed.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/debug/examples/taskbar_probe.exe',
    [string]$OutputDirectory = 'native-evidence/r02',
    [string]$ProbeTarget = 'x86_64-pc-windows-msvc',
    [switch]$RestartExplorer,
    [switch]$PrepareHostedDesktop
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TaskbarEvidenceNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct SystemInfo {
        public ushort Architecture, Reserved;
        public uint PageSize;
        public IntPtr Minimum, Maximum, Mask;
        public uint Processors, ProcessorType, Granularity;
        public ushort Level, Revision;
    }
    [DllImport("kernel32.dll")] public static extern void GetNativeSystemInfo(out SystemInfo info);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [StructLayout(LayoutKind.Sequential)]
    public struct Keyboard { public ushort Key, Scan; public uint Flags, Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input { [FieldOffset(0)] public uint Type; [FieldOffset(8)] public Keyboard Keyboard; }
    [DllImport("user32.dll")] public static extern uint SendInput(uint count, Input[] inputs, int size);
    public static void Enter() {
        var down = new Input { Type=1, Keyboard=new Keyboard { Key=0x0d } };
        var up = new Input { Type=1, Keyboard=new Keyboard { Key=0x0d, Flags=2 } };
        if (SendInput(2, new [] {down, up}, Marshal.SizeOf(typeof(Input))) != 2) throw new InvalidOperationException("Setup input failed");
    }
}
'@
[TaskbarEvidenceNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
$exe = (Resolve-Path $Executable).Path
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path $OutputDirectory).Path
$os = Get-CimInstance Win32_OperatingSystem
$registry = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$systemInfo = New-Object TaskbarEvidenceNative+SystemInfo
[TaskbarEvidenceNative]::GetNativeSystemInfo([ref]$systemInfo)
$context = [ordered]@{
    source = (& git rev-parse HEAD).Trim()
    os = $os.Caption
    build = $registry.CurrentBuildNumber
    ubr = $registry.UBR
    nativeArchitecture = $systemInfo.Architecture # 9=x64; 12=ARM64
    probeTarget = $ProbeTarget
    x64Emulated = $systemInfo.Architecture -eq 12 -and $ProbeTarget -eq 'x86_64-pc-windows-msvc'
    image = $env:ImageOS
    imageVersion = $env:ImageVersion
    executableSha256 = (Get-FileHash $exe -Algorithm SHA256).Hash
    capturedUtc = [DateTime]::UtcNow.ToString('o')
    displays = @([Windows.Forms.Screen]::AllScreens | ForEach-Object {
        [ordered]@{device=$_.DeviceName; primary=$_.Primary; left=$_.Bounds.Left; top=$_.Bounds.Top; width=$_.Bounds.Width; height=$_.Bounds.Height}
    })
    results = [ordered]@{}
    limitations = @('Hosted image, not two stock Windows 11 x64 builds', 'No mixed-DPI second display unless independently recorded', 'No selector is accepted by this experiment')
}
function Save-Context {
    $context | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8 (Join-Path $output 'context.json')
}
Save-Context

function Start-Probe([string]$Name, [int]$Samples) {
    Start-Process -FilePath $exe -ArgumentList @($Samples.ToString()) -PassThru `
        -RedirectStandardOutput (Join-Path $output "$Name.jsonl") `
        -RedirectStandardError (Join-Path $output "$Name.stderr.txt")
}
function Read-Probe([Diagnostics.Process]$Process, [string]$Name, [int]$TimeoutSeconds) {
    if (!$Process.WaitForExit($TimeoutSeconds * 1000)) {
        $Process.Kill()
        throw "Supervisor exceeded harness bound for $Name"
    }
    $Process.WaitForExit() # flush redirected output after observed exit
    if ($Process.ExitCode -ne 0) { throw "Supervisor $Name exited $($Process.ExitCode); retain stderr" }
    $records = @(Get-Content (Join-Path $output "$Name.jsonl") | Where-Object { $_.Trim() } | ForEach-Object { $_ | ConvertFrom-Json })
    if (!$records.Count) { throw "No structured records for $Name" }
    foreach ($record in $records) {
        if ($record.result.schema -ne 1) { throw 'Unknown schema' }
        if ($record.result.status -notin @('unsupported_no_verified_selector','probe_error','deadline','helper_exit','stale_generation')) { throw 'Unknown probe status' }
        foreach ($taskbar in $record.result.taskbars) {
            if ($null -ne $taskbar.clock_rect) { throw 'Unverified clock rectangle was accepted' }
        }
    }
    return $records
}
function Wait-Helper([Diagnostics.Process]$Parent) {
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        $child = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($Parent.Id)" |
            Where-Object { $_.Name -eq 'taskbar_probe.exe' -and $_.CommandLine -match '--helper' } | Select-Object -First 1
        if ($child) {
            try { return [Diagnostics.Process]::GetProcessById($child.ProcessId) } catch { }
        }
        if ($Parent.HasExited) { throw 'Supervisor ended before an outstanding helper was observed' }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'No outstanding helper observed within the harness bound'
}
function Assert-NoHelpers([string]$Phase) {
    $deadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $remaining = @(Get-CimInstance Win32_Process -Filter "Name = 'taskbar_probe.exe'" | Where-Object { $_.ExecutablePath -eq $exe })
        if (!$remaining.Count) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Probe processes remained after $Phase"
}
function Wait-Stall([Diagnostics.Process]$Parent, [string]$Marker) {
    # A cold, freshly built binary on a loaded hosted ARM64 runner can need more than
    # a few seconds to start; the marker still proves entry into the stalled operation.
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (!(Test-Path $Marker) -and [DateTime]::UtcNow -lt $deadline -and !$Parent.HasExited) { Start-Sleep -Milliseconds 20 }
    if (!(Test-Path $Marker)) { throw 'Injected helper did not confirm entry into its stalled operation' }
    return [Diagnostics.Process]::GetProcessById([int](Get-Content $Marker))
}
function Capture-Desktop([string]$Name, $Record) {
    $bounds = [Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = New-Object Drawing.Bitmap($bounds.Width, $bounds.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $pen = New-Object Drawing.Pen([Drawing.Color]::Red, 1)
    $font = New-Object Drawing.Font('Segoe UI', 8)
    try {
        $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $output "$Name-original.png"), [Drawing.Imaging.ImageFormat]::Png)
        foreach ($taskbar in $Record.result.taskbars) {
            $index = 0
            foreach ($element in $taskbar.uia.elements) {
                $r = $element.rect
                if ($r -and $r.right -gt $r.left -and $r.bottom -gt $r.top) {
                    $graphics.DrawRectangle($pen, $r.left-$bounds.Left, $r.top-$bounds.Top, $r.right-$r.left, $r.bottom-$r.top)
                    $graphics.DrawString("U$index", $font, [Drawing.Brushes]::Red, [single]($r.left-$bounds.Left), [single]($r.top-$bounds.Top))
                }
                $index++
            }
        }
        $bitmap.Save((Join-Path $output "$Name-inventory.png"), [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $font.Dispose(); $pen.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
    }
}

$active = $null
$oldStall = $env:ELEVENCLOCK_R02_SIMULATE_STALL
$oldMarker = $env:ELEVENCLOCK_R02_STALL_READY
try {
    if ($PrepareHostedDesktop) {
        if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Hosted desktop preparation is restricted to ephemeral GitHub runners' }
        Capture-Desktop 'startup' $null
        $session = (Get-Process -Id $PID).SessionId
        # Some hosted ARM images start with the profile privacy/OOBE broker in
        # front of Explorer. Close only those brokers in this ephemeral session;
        # do not claim a root-only UIA inventory is a usable clock capture.
        $brokers = @(Get-Process -Name UserOOBEBroker, CloudExperienceHostBroker, CloudExperienceHost -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session })
        $context.results.desktopPreparation = @($brokers | ForEach-Object { [ordered]@{name=$_.ProcessName; pid=$_.Id; action='terminated ephemeral startup broker'} })
        $brokers | Stop-Process -Force
        Start-Sleep -Seconds 2
        # Current images can use a separate privacy/OOBE host. Only advance a
        # known setup process in this ephemeral session, via the default native
        # Enter gesture. This prepares the OS profile, never selects a clock.
        $context.results.setupGestures = @()
        Get-Process | Where-Object { $_.SessionId -eq $session } | Select-Object ProcessName, Id |
            ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $output 'startup-processes.json')
        foreach ($iteration in 1..6) {
            [uint32]$ownerId = 0
            [TaskbarEvidenceNative]::GetWindowThreadProcessId([TaskbarEvidenceNative]::GetForegroundWindow(), [ref]$ownerId) | Out-Null
            $owner = Get-Process -Id $ownerId -ErrorAction SilentlyContinue
            $context.results.setupGestures += [ordered]@{iteration=$iteration; foregroundOwner=$owner.ProcessName; pid=$ownerId}
            if (!$owner -or $owner.SessionId -ne $session -or $owner.ProcessName -notmatch 'OOBE|Privacy|WWAHost|CloudExperience') { break }
            [TaskbarEvidenceNative]::Enter()
            Start-Sleep -Milliseconds 750
            Capture-Desktop "startup-advance-$iteration" $null
        }
        Save-Context
    }
    # Completing the profile can asynchronously recreate Explorer and its
    # taskbar. Wait for a real clock subtree, not just an early Shell_TrayWnd.
    $context.results.desktopReadiness = @()
    $ready = $false
    $readyDeadline = [DateTime]::UtcNow.AddSeconds(40)
    $attempt = 0
    do {
        $attempt++
        $active = Start-Probe "desktop-ready-$attempt" 1
        $readyRecords = @(Read-Probe $active "desktop-ready-$attempt" 10)
        $readyClock = @($readyRecords.result.taskbars.uia.elements | Where-Object { $_.automation_id -in @('TimeInnerTextBlock','DateInnerTextBlock') })
        $context.results.desktopReadiness += [ordered]@{attempt=$attempt; statuses=@($readyRecords.result.status); clockTextNodes=$readyClock.Count}
        if ($readyClock.Count) { $ready = $true; break }
        Start-Sleep -Milliseconds 750
    } while ([DateTime]::UtcNow -lt $readyDeadline)
    Save-Context
    if (!$ready) {
        Capture-Desktop 'not-ready' $null
        throw 'No real clock subtree after desktop startup bound; unsupported hosted environment'
    }
    $shell = [TaskbarEvidenceNative]::FindWindowW('Shell_TrayWnd', $null)
    if ($shell -eq [IntPtr]::Zero) { throw 'No interactive Explorer taskbar on this hosted desktop' }
    Assert-NoHelpers 'initial state'
    $active = Start-Probe 'baseline' 2
    $records = @(Read-Probe $active 'baseline' 20)
    $context.results.baseline = @($records | ForEach-Object { [ordered]@{status=$_.result.status; supervisorMicros=$_.supervisor_elapsed_micros; taskbars=@($_.result.taskbars).Count} })
    $inventory = $records | Where-Object { $_.result.taskbars.Count -gt 0 } | Select-Object -First 1
    if (!$inventory) { throw 'No completed native inventory; failure evidence retained' }
    Capture-Desktop 'baseline' $inventory
    $clockText = @($inventory.result.taskbars.uia.elements | Where-Object { $_.automation_id -in @('TimeInnerTextBlock','DateInnerTextBlock') })
    if (!$clockText.Count) { throw 'No actual clock subtree available; root-only/covered taskbar is unsupported and not successful desktop evidence' }
    Assert-NoHelpers 'baseline'
    Save-Context

    if ($RestartExplorer) {
        # Only ephemeral developer desktops. Scope the restart to this session.
        $session = (Get-Process -Id $PID).SessionId
        $before = @(Get-Process explorer | Where-Object { $_.SessionId -eq $session })
        if (!$before.Count) { throw 'No Explorer process in this desktop session' }
        $active = Start-Probe 'restart-in-flight' 6
        $helper = Wait-Helper $active
        $restartUtc = [DateTime]::UtcNow.ToString('o')
        $context.results.restart = [ordered]@{atUtc=$restartUtc; outstandingHelperPid=$helper.Id; oldExplorerPids=@($before.Id); overlapClaim='Helper observed; an exact COM-call overlap is not asserted'}
        $before | Stop-Process -Force
        Start-Sleep -Milliseconds 500
        if ([TaskbarEvidenceNative]::FindWindowW('Shell_TrayWnd', $null) -eq [IntPtr]::Zero) { Start-Process explorer.exe | Out-Null }
        $records = @(Read-Probe $active 'restart-in-flight' 50)
        $context.results.restart.statuses = @($records | ForEach-Object { $_.result.status })
        Assert-NoHelpers 'Explorer restart'
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while ([TaskbarEvidenceNative]::FindWindowW('Shell_TrayWnd', $null) -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
        if ([TaskbarEvidenceNative]::FindWindowW('Shell_TrayWnd', $null) -eq [IntPtr]::Zero) { throw 'Explorer taskbar did not recover' }
        $active = Start-Probe 'recovered' 2
        $records = @(Read-Probe $active 'recovered' 20)
        $inventory = $records | Where-Object { $_.result.taskbars.Count -gt 0 } | Select-Object -First 1
        if (!$inventory) { throw 'No recovered native inventory' }
        $context.results.restart.newShellPids = @($inventory.result.taskbars.shell_pid)
        if (@($inventory.result.taskbars.shell_pid | Where-Object { $_ -in $before.Id }).Count) { throw 'Discovery retained an old Explorer identity' }
        Capture-Desktop 'recovered' $inventory
        Save-Context
    }

    # Fault injection proves OS teardown without claiming COM cancellation.
    $env:ELEVENCLOCK_R02_SIMULATE_STALL = '1'
    $marker = Join-Path $output 'stall-ready.txt'
    $env:ELEVENCLOCK_R02_STALL_READY = $marker
    $context.results.injectedStalls = @()
    foreach ($iteration in 1..3) {
        Remove-Item $marker -Force -ErrorAction SilentlyContinue
        $active = Start-Probe "stall-$iteration" 1
        $helper = Wait-Stall $active $marker
        $records = @(Read-Probe $active "stall-$iteration" 10)
        if ($records.Count -ne 1 -or $records[0].result.status -ne 'deadline') { throw 'Blocked helper did not produce a deadline result' }
        if (!$helper.WaitForExit(1000)) { throw 'Deadline helper did not exit' }
        Assert-NoHelpers "injected stall $iteration"
        $context.results.injectedStalls += [ordered]@{supervisorMicros=$records[0].supervisor_elapsed_micros; teardownMicros=$records[0].result.teardown_micros; noRemainingHelpers=$true; synthetic=$true}
        Save-Context
    }
    Remove-Item $marker -Force -ErrorAction SilentlyContinue
    $active = Start-Probe 'parent-loss' 60
    $helper = Wait-Stall $active $marker
    $helperId = $helper.Id
    $active.Kill()
    if (!$active.WaitForExit(3000)) { throw 'Supervisor termination not observed' }
    if (!$helper.WaitForExit(3000)) { throw 'Job-owned helper survived supervisor termination' }
    Assert-NoHelpers 'parent termination'
    $context.results.parentLoss = [ordered]@{helperPid=$helperId; helperExitObserved=$true; noRemainingHelpers=$true; confirmedStalledHelper=$true; synthetic=$true}
    $context.status = 'hosted_checks_passed_required_x64_mixed_dpi_gate_still_open'
} catch {
    $context.status = 'harness_failed'
    $context.failure = $_.Exception.Message
    throw
} finally {
    if ($active -and !$active.HasExited) { $active.Kill(); $active.WaitForExit(3000) | Out-Null }
    [Environment]::SetEnvironmentVariable('ELEVENCLOCK_R02_SIMULATE_STALL', $oldStall, 'Process')
    [Environment]::SetEnvironmentVariable('ELEVENCLOCK_R02_STALL_READY', $oldMarker, 'Process')
    Save-Context
}
