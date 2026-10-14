# Development-only hosted lifecycle harness for R08; never part of the installed app.
#
# Exercises the real clock-app.exe the way a user's session does: quiet login-style
# start, second launch, close-to-background, Exit, and (optionally) Explorer
# kill/restart while measuring processes, threads and handles. It uses only
# public process/window state: the product has no command suite or debug channel.
# Overlay observation (-ObserveOverlays) is informational evidence for the
# development-only `unverified-overlays` build; the production gate is closed and
# creates no overlay, so it is never an acceptance check.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/debug/clock-app.exe',
    [string]$OutputDirectory = 'native-evidence/lifecycle',
    [int]$ExplorerRestarts = 3,
    [int]$IdleSeconds = 20,
    [switch]$ObserveOverlays
)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class Lifecycle {
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string className, string title);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr window, StringBuilder text, int count);
    // Diagnostic: every top-level window of a process, as "class | title | visible".
    public static string[] Describe(uint process) {
        var found = new List<string>();
        EnumWindows((window, parameter) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != process) return true;
            var cls = new StringBuilder(128); var title = new StringBuilder(256);
            GetClassNameW(window, cls, cls.Capacity); GetWindowTextW(window, title, title.Capacity);
            found.Add(cls + " | " + title + " | " + (IsWindowVisible(window) ? "visible" : "hidden"));
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
    public static int Count(uint process, string className, bool visibleOnly) {
        int count = 0;
        EnumWindows((window, parameter) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != process) return true;
            var text = new StringBuilder(128);
            if (GetClassNameW(window, text, text.Capacity) > 0 && text.ToString() == className && (!visibleOnly || IsWindowVisible(window))) count++;
            return true;
        }, IntPtr.Zero);
        return count;
    }
}
'@
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$results = [ordered]@{ executable = $Executable; steps = [ordered]@{} }
$exe = (Resolve-Path $Executable).Path
$baseline = @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
function Others { @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $baseline }) }
function Wait-Until([scriptblock]$Condition, [int]$Seconds, [string]$What) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (!(& $Condition)) {
        if ([DateTime]::UtcNow -ge $deadline) { throw "Timed out: $What" }
        Start-Sleep -Milliseconds 100
    }
}
function Settings-Open([Diagnostics.Process]$P) {
    $P.Refresh()
    return (!$P.HasExited -and $P.MainWindowHandle -ne 0 -and $P.MainWindowTitle -eq 'ElevenClock Settings')
}
function Sample([Diagnostics.Process]$P) {
    $P.Refresh()
    [ordered]@{ threads = $P.Threads.Count; handles = $P.HandleCount; workingSetKiB = [int]($P.WorkingSet64 / 1KB); processes = (Others).Count }
}
function Max-Processes([Diagnostics.Process]$P, [int]$Seconds) {
    $max = 0
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $max = [Math]::Max($max, (Others).Count)
        if ($P.HasExited) { throw 'Application exited unexpectedly' }
        Start-Sleep -Milliseconds 100
    }
    return $max
}
function Overlay-Count([Diagnostics.Process]$P) { [Lifecycle]::Count([uint32]$P.Id, 'ElevenClock.Overlay', $true) }

$app = $null
try {
    # 1. Quiet (login-style) start: the clock runs without opening Settings.
    $app = Start-Process $exe -ArgumentList '--startup' -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ([Lifecycle]::FindWindowW('ElevenClock.Runtime.Host', [NullString]::Value) -eq [IntPtr]::Zero -and !$app.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 200 }
    Start-Sleep -Seconds 2
    $app.Refresh()
    $results.steps.startupWindows = @([Lifecycle]::Describe([uint32]$app.Id))
    if ($app.HasExited) { throw "Quiet startup exited with code $($app.ExitCode)" }
    if ($app.MainWindowHandle -ne 0) { throw 'Quiet startup must not open Settings' }
    if ([Lifecycle]::FindWindowW('ElevenClock.Runtime.Host', [NullString]::Value) -eq [IntPtr]::Zero) { throw "Runtime host window missing; windows: $($results.steps.startupWindows -join '; ')" }
    $results.steps.quietStart = 'Process running, no Settings window, host window present'

    # 2. Idle: at most the app plus one discovery helper at any time.
    $first = Sample $app
    $results.steps.startupSample = $first
    $results.steps.maxProcessesWhileIdle = Max-Processes $app $IdleSeconds
    if ($results.steps.maxProcessesWhileIdle -gt 2) { throw "More than one helper at a time: $($results.steps.maxProcessesWhileIdle) processes" }
    $idle = Sample $app
    $results.steps.idleSample = $idle
    if ($ObserveOverlays) { $results.steps.overlaysVisibleAtStart = Overlay-Count $app }

    # 3. A second launch shows Settings in the same process and adds no clock.
    $second = Start-Process $exe -PassThru
    if (!$second.WaitForExit(15000)) { $second.Kill(); throw 'Second launch did not exit' }
    if ($second.ExitCode -ne 0) { throw "Second launch exit code $($second.ExitCode)" }
    Wait-Until { Settings-Open $app } 15 'Settings in the first process'
    if ((Get-Process -Name clock-app -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -eq 'ElevenClock Settings' }).Count -ne 1) { throw 'Expected exactly one Settings window' }
    $results.steps.secondLaunch = 'Existing process showed Settings; the duplicate exited with code 0'

    # 4. Closing Settings keeps the clock running.
    if (!$app.CloseMainWindow()) { throw 'Could not close Settings' }
    Wait-Until { !(Settings-Open $app) -or $app.HasExited } 10 'Settings to close'
    $app.Refresh()
    if ($app.HasExited) { throw 'Closing Settings stopped the application' }
    $results.steps.closeToBackground = 'Settings closed; process kept running'

    # 5. Explorer restarts: the process survives, helpers do not accumulate, and
    #    handles/threads return to their steady state.
    $restarts = @()
    for ($i = 1; $i -le $ExplorerRestarts; $i++) {
        $session = (Get-Process -Id $PID).SessionId
        $old = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session })
        $old | Stop-Process -Force
        Start-Sleep -Milliseconds 500
        if ($ObserveOverlays) { $lost = Overlay-Count $app }
        if ([Lifecycle]::FindWindowW('Shell_TrayWnd', [NullString]::Value) -eq [IntPtr]::Zero) {
            # Windows normally restarts the shell itself; start it if it did not.
            Start-Sleep -Seconds 3
            if ([Lifecycle]::FindWindowW('Shell_TrayWnd', [NullString]::Value) -eq [IntPtr]::Zero) { Start-Process explorer.exe | Out-Null }
        }
        Wait-Until { [Lifecycle]::FindWindowW('Shell_TrayWnd', [NullString]::Value) -ne [IntPtr]::Zero } 45 'Explorer taskbar to return'
        $maxDuring = Max-Processes $app 10
        $app.Refresh()
        if ($app.HasExited) { throw "Application exited during Explorer restart $i" }
        if ($maxDuring -gt 2) { throw "Helper accumulation during restart ${i}: $maxDuring processes" }
        $entry = [ordered]@{ restart = $i; maxProcesses = $maxDuring; after = (Sample $app) }
        if ($ObserveOverlays) { $entry.overlaysRightAfterLoss = $lost; $entry.overlaysAfterRecovery = Overlay-Count $app }
        $restarts += $entry
    }
    $results.steps.explorerRestarts = $restarts
    if ($ExplorerRestarts -gt 0) {
        Start-Sleep -Seconds 5
        $final = Sample $app
        $results.steps.afterRestartsSample = $final
        if ($final.handles -gt $idle.handles + 40) { throw "Handle growth across Explorer restarts: $($idle.handles) -> $($final.handles)" }
        if ($final.threads -gt $idle.threads + 4) { throw "Thread growth across Explorer restarts: $($idle.threads) -> $($final.threads)" }
        if ($final.processes -gt 1) { throw "Helper processes remain: $($final.processes)" }
    }

    # 6. Exit through Settings stops the application and everything it started.
    $third = Start-Process $exe -PassThru
    if (!$third.WaitForExit(15000)) { $third.Kill(); throw 'Launch before Exit did not return' }
    Wait-Until { Settings-Open $app } 15 'Settings before Exit'
    [Lifecycle]::SendMessageW([Lifecycle]::GetDlgItem($app.MainWindowHandle, 1003), 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (!$app.WaitForExit(15000)) { throw 'Exit did not stop the application' }
    if ($app.ExitCode -ne 0) { throw "Exit code $($app.ExitCode)" }
    Wait-Until { (Others).Count -eq 0 } 10 'all helper processes to be gone'
    $results.steps.exit = 'Exit code 0; no clock-app process remained'
    if ([Lifecycle]::FindWindowW('Shell_TrayWnd', [NullString]::Value) -eq [IntPtr]::Zero) { throw 'Explorer taskbar missing after Exit' }
} finally {
    if ($app -and !$app.HasExited) { $app.Kill() }
    Others | ForEach-Object { try { $_.Kill() } catch { } }
    $results | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'lifecycle.json')
}
