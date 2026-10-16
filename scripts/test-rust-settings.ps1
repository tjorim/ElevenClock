# Development-only native Settings harness; never part of the installed app.
# Drives the real window with the Windows input queue and reads the real controls:
# structure, tab order and mnemonics, live preview (including the ISO year boundary),
# format errors, Apply persistence, Cancel, close-to-background, second-launch focus
# and Exit. It saves and restores the current user's preferences file.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/release/elevenclock-rs.exe',
    [string]$OutputDirectory = 'native-evidence',
    [int]$ExpectedDpi = 0,
    [ValidateSet('Window', 'Escape', 'Enter', 'Mnemonic')]
    [string]$CloseGesture = 'Window'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName Accessibility
Add-Type -ReferencedAssemblies @('System.dll', [Accessibility.IAccessible].Assembly.Location) @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class SettingsNative {
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr window, StringBuilder text, int count);
    // The process' visible Settings window, found by title. A visible overlay can be
    // the process' MainWindowHandle, so that property cannot identify Settings.
    public static IntPtr Settings(int process) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, parameter) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            if (owner != (uint)process || !IsWindowVisible(window)) return true;
            var title = new StringBuilder(256); GetWindowTextW(window, title, title.Capacity);
            if (title.ToString() == "ElevenClock-rs Settings") { found = window; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct GuiInfo {
        public int Size, Flags;
        public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Keyboard { public ushort Key, Scan; public uint Flags, Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct Input { [FieldOffset(0)] public uint Type; [FieldOffset(8)] public Keyboard Keyboard; }
    [DllImport("user32.dll")] static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr control);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr window, uint message, int wparam, StringBuilder text);

    // Key presses go through the real input queue and the dialog manager.
    public static void KeyStroke(ushort key, ushort modifier) {
        if (!Environment.Is64BitProcess) throw new InvalidOperationException("Harness expects x64");
        var inputs = new System.Collections.Generic.List<Input>();
        if (modifier != 0) inputs.Add(Key(modifier, false));
        inputs.Add(Key(key, false)); inputs.Add(Key(key, true));
        if (modifier != 0) inputs.Add(Key(modifier, true));
        if (SendInput((uint)inputs.Count, inputs.ToArray(), Marshal.SizeOf(typeof(Input))) != inputs.Count) {
            // Release both keys if input injection fails partially.
            SendInput(2, new [] {Key(key, true), Key(modifier, true)}, Marshal.SizeOf(typeof(Input)));
            throw new InvalidOperationException("Windows keyboard input injection failed");
        }
    }
    private static Input Key(ushort key, bool up) {
        return new Input {Type = 1, Keyboard = new Keyboard {Key = key, Flags = up ? 2u : 0u}};
    }
    // The focused control of a window's thread, readable from another process.
    public static IntPtr Focus(IntPtr window) {
        uint process; uint thread = GetWindowThreadProcessId(window, out process);
        var info = new GuiInfo(); info.Size = Marshal.SizeOf(info);
        return GetGUIThreadInfo(thread, ref info) ? info.Focus : IntPtr.Zero;
    }
    // Alert events (EVENT_SYSTEM_ALERT) are what the app raises so a reader speaks a status
    // change; this records which window each one came from. Needs a pumping thread.
    public delegate void WinEventProc(IntPtr hook, uint eventType, IntPtr window, int objectId, int childId, uint thread, uint time);
    [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr module, WinEventProc callback, uint process, uint thread, uint flags);
    static WinEventProc alertCallback;
    static readonly System.Collections.Generic.List<IntPtr> alerts = new System.Collections.Generic.List<IntPtr>();
    public static void WatchAlerts(uint process) {
        alertCallback = (hook, eventType, window, objectId, childId, thread, time) => { lock (alerts) alerts.Add(window); };
        if (SetWinEventHook(0x0002, 0x0002, IntPtr.Zero, alertCallback, process, 0, 0) == IntPtr.Zero) throw new InvalidOperationException("Could not watch accessibility alerts");
    }
    public static int AlertsFrom(IntPtr window) { lock (alerts) { return alerts.FindAll(w => w == window).Count; } }
    // The classic control's default IAccessible: what Narrator and other readers query.
    [DllImport("oleacc.dll")] static extern int AccessibleObjectFromWindow(IntPtr window, uint objectId, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out Accessibility.IAccessible accessible);
    public static Accessibility.IAccessible Accessible(IntPtr window) {
        var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        Accessibility.IAccessible accessible;
        Marshal.ThrowExceptionForHR(AccessibleObjectFromWindow(window, 0xfffffffc, ref iid, out accessible));
        return accessible;
    }
    public static string Text(IntPtr control) {
        var text = new StringBuilder(8192); SendMessageW(control, 0x000D, 8192, text); // WM_GETTEXT
        return text.ToString().Replace("\r\n", "\n");
    }
}
'@
[SettingsNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

# --- control lookup: pages are children of the dialog, controls children of pages ---
$PAGE = @{ Date = 111; Appearance = 112; Startup = 113; Support = 114 }
function Control([IntPtr]$Window, [int]$Page, [int]$Id) {
    if ($Page -eq 0) { return [SettingsNative]::GetDlgItem($Window, $Id) }
    return [SettingsNative]::GetDlgItem([SettingsNative]::GetDlgItem($Window, $Page), $Id)
}
function Press([int]$Key, [int]$Modifier = 0) { [SettingsNative]::KeyStroke([uint16]$Key, [uint16]$Modifier); Start-Sleep -Milliseconds 150 }
function Type-Text([string]$Text) {
    foreach ($c in $Text.ToCharArray()) {
        if ($c -eq '%') { [SettingsNative]::KeyStroke(0x35, 0x10) }
        elseif ($c -eq '.') { [SettingsNative]::KeyStroke(0xBE, 0) }
        elseif ([char]::IsUpper($c)) { [SettingsNative]::KeyStroke([uint16][char]$c, 0x10) }
        elseif ([char]::IsLower($c)) { [SettingsNative]::KeyStroke([uint16][char]::ToUpper($c), 0) }
        elseif ([char]::IsDigit($c)) { [SettingsNative]::KeyStroke([uint16][char]$c, 0) }
        else { throw "Type-Text does not support '$c'" }
        Start-Sleep -Milliseconds 40
    }
    Start-Sleep -Milliseconds 300
}
function Click([IntPtr]$Button) { [SettingsNative]::SendMessageW($Button, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 500 } # BM_CLICK
function Focused-Id([IntPtr]$Window) { [SettingsNative]::GetDlgCtrlID([SettingsNative]::Focus($Window)) }
function Assert([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
Add-Type -AssemblyName System.Windows.Forms
# Let queued out-of-process accessibility events reach this thread's callbacks.
function Pump([int]$Milliseconds) {
    $end = [DateTime]::UtcNow.AddMilliseconds($Milliseconds)
    while ([DateTime]::UtcNow -lt $end) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 }
}
function Capture-Window([IntPtr]$Window, [string]$Name) {
    $rect = New-Object SettingsNative+Rect
    if (![SettingsNative]::GetWindowRect($Window, [ref]$rect)) { throw 'Could not measure Settings window' }
    $bitmap = [Drawing.Bitmap]::new(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try { if (![SettingsNative]::PrintWindow($Window, $dc, 2)) { throw 'Native dialog capture failed' } }
            finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path (Resolve-Path $OutputDirectory) $Name), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
}

# --- the current user's preferences are saved and restored around the run ---
$preferences = Join-Path $env:LOCALAPPDATA 'ElevenClock-rs\preferences.dat'
$backup = $null
if (Test-Path $preferences) { $backup = [IO.File]::ReadAllBytes($preferences); Remove-Item $preferences -Force }

# Processes of this executable that exist before the run (other tests, helpers).
$baseline = @(Get-Process -Name elevenclock-rs -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$app = Start-Process $Executable -PassThru
function Wait-Settings([bool]$Open, [string]$What) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 100
        $app.Refresh()
        if ($app.HasExited) { throw "Application exited while waiting for: $What" }
        $isOpen = [SettingsNative]::Settings($app.Id) -ne [IntPtr]::Zero
    } while ($isOpen -ne $Open -and [DateTime]::UtcNow -lt $deadline)
    if ($isOpen -ne $Open) { throw "Timed out waiting for: $What" }
}
function Foreground([IntPtr]$Window) {
    [SettingsNative]::SetForegroundWindow($Window) | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds(3)
    while ([SettingsNative]::GetForegroundWindow() -ne $Window -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Assert ([SettingsNative]::GetForegroundWindow() -eq $Window) 'Keyboard test cannot activate the actual dialog'
}
$steps = [ordered]@{}
try {
    Wait-Settings $true 'first launch to open Settings'
    $window = [SettingsNative]::Settings($app.Id)
    $dpi = [SettingsNative]::GetDpiForWindow($window)
    if ($ExpectedDpi -ne 0 -and $dpi -ne $ExpectedDpi) { throw "Actual window DPI $dpi differs from requested $ExpectedDpi" }
    Start-Sleep -Milliseconds 500

    # 1. First run: welcome text, and nothing is written until the user acts.
    Assert ([SettingsNative]::Text((Control $window 0 1101)) -like 'Welcome to ElevenClock-rs*') 'First launch must show the welcome text'
    $steps.firstRun = 'Welcome text shown on the first launch'

    # 2. Structure: every page and control the contract names exists.
    foreach ($id in 1100, 1102, 1003, 1104, 1105, 1, 2) { Assert ((Control $window 0 $id) -ne [IntPtr]::Zero) "Missing main control $id" }
    foreach ($id in 1201, 1202, 1203, 1204, 1205) { Assert ((Control $window $PAGE.Date $id) -ne [IntPtr]::Zero) "Missing Date and time control $id" }
    foreach ($id in 1301, 1302, 1303, 1304, 1305, 1306) { Assert ((Control $window $PAGE.Appearance $id) -ne [IntPtr]::Zero) "Missing Appearance control $id" }
    Assert ((Control $window $PAGE.Startup 1401) -ne [IntPtr]::Zero) 'Missing Startup page'
    Assert ((Control $window $PAGE.Support 1501) -ne [IntPtr]::Zero) 'Missing Support page'
    Assert ([SettingsNative]::Text((Control $window $PAGE.Support 1501)) -like '*version*') 'Support page must show the version'
    Assert (-not [SettingsNative]::IsWindowEnabled((Control $window 0 1105))) 'Apply must be disabled while nothing changed'
    $steps.structure = 'All pages and controls present; Apply disabled when clean'

    # 3. Tab order and mnemonics, through the real input queue.
    Foreground $window
    $expected = 1100, 1201, 1202, 1204, 1205, 1003, 1104, 1, 2
    $actual = @(Focused-Id $window)
    1..($expected.Count - 1) | ForEach-Object { Press 9; $actual += Focused-Id $window }
    Assert (($actual -join ',') -eq ($expected -join ',')) "Tab order was $($actual -join ',') but expected $($expected -join ',')"
    Press 9; Assert ((Focused-Id $window) -eq 1100) 'Tab cycle must wrap to the page tabs'
    Press 9 0x10; Assert ((Focused-Id $window) -eq 2) 'Shift+Tab must go back'
    Press 70 18; Assert ((Focused-Id $window) -eq 1202) 'Alt+F must focus the format editor'
    $steps.keyboard = "Tab order $($expected -join ' > '), wrap, Shift+Tab and Alt+F passed"

    # 4. Live preview: the documented fixture and the ISO year boundary.
    $preview = [SettingsNative]::Text((Control $window $PAGE.Date 1204))
    Assert ($preview -match 'Tue 13 Oct 2026 \(ISO week-year 2026, week 42\): 12:34 . 2642\.2') "Fixture preview missing: $preview"
    Assert ($preview -match 'Fri 1 Jan 2027 \(ISO week-year 2026, week 53\): 12:34 . 2653\.5') "ISO year boundary preview missing: $preview"
    Press 65 0x11                  # Ctrl+A in the format editor
    Type-Text '%y%V.%u'            # calendar-year preset, typed
    $preview = [SettingsNative]::Text((Control $window $PAGE.Date 1204))
    Assert ($preview -match 'Tue 13 Oct 2026 .*: 2642\.2') "Fixture must still preview 2642.2: $preview"
    Assert ($preview -match 'Fri 1 Jan 2027 .*: 2753\.5') "Calendar-year preset must preview 2753.5 on 1 Jan 2027: $preview"
    $selected = [SettingsNative]::SendMessageW((Control $window $PAGE.Date 1201), 0x147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() # CB_GETCURSEL
    Assert ($selected -eq 1) "Typing a preset's format must select that preset, not index $selected"
    Assert ([SettingsNative]::IsWindowEnabled((Control $window 0 1105))) 'Apply must enable after an edit'
    $steps.preview = 'Fixture previews 2642.2; ISO week-year 2653.5 vs calendar year 2753.5 on 2027-01-01'

    # 5. An invalid token is explained beside the editor and is not applied.
    Press 65 0x11; Type-Text '%Q'
    $problem = [SettingsNative]::Text((Control $window $PAGE.Date 1203))
    Assert ($problem -like '*not a supported token*') "Format error not shown beside the editor: '$problem'"
    Click (Control $window 0 1105)
    Assert ([SettingsNative]::Text((Control $window 0 1102)) -like '*not applied*') 'Applying an invalid format must say it was not applied'
    Assert (-not (Test-Path $preferences)) 'An invalid format must not write preferences'
    $steps.invalidFormat = 'Error shown beside the editor; Apply refused and nothing was written'

    # 6. A valid edit is applied and persisted.
    Press 65 0x11; Type-Text '%y%V.%u'
    Click (Control $window 0 1105)
    Assert ([SettingsNative]::Text((Control $window 0 1102)) -like 'Settings applied*') 'Apply did not report success'
    Assert ((Test-Path $preferences) -and ((Get-Content $preferences -Raw) -like '*%y%V.%u*')) 'Applied format was not persisted'
    Assert (-not [SettingsNative]::IsWindowEnabled((Control $window 0 1105))) 'Apply must disable again once applied'
    $steps.apply = 'Apply reported success and persisted the format without a restart'

    # 7. Font bounds are reported beside the control.
    Press 0x22 0x11                # Ctrl+PageDown: Appearance
    Press 9; Press 9               # radio group, then the font size
    Assert ((Focused-Id $window) -eq 1303) 'Font size must be reachable by Tab'
    Type-Text '30'
    Assert ([SettingsNative]::Text((Control $window $PAGE.Appearance 1305)) -like '*between 6 and 24*') 'Font bound error not shown beside the control'
    $steps.fontBounds = 'Out-of-range size explained beside the control'

    # 8. Cancel discards; closing keeps the clock running; a second launch shows
    #    Settings again with the saved values, never the discarded ones.
    Click (Control $window 0 2)
    Wait-Settings $false 'Cancel to close Settings'
    $app.Refresh(); Assert (-not $app.HasExited) 'Cancel must leave the clock running'
    $again = Start-Process $Executable -PassThru
    if (!$again.WaitForExit(15000)) { $again.Kill(); throw 'Second launch did not exit' }
    Assert ($again.ExitCode -eq 0) "Second launch exit code $($again.ExitCode)"
    Wait-Settings $true 'second launch to show Settings in the existing process'
    $window = [SettingsNative]::Settings($app.Id)
    Start-Sleep -Milliseconds 500
    Assert ([SettingsNative]::Text((Control $window $PAGE.Appearance 1303)) -eq '9') 'Cancelled font size must not return'
    Assert ([SettingsNative]::Text((Control $window $PAGE.Date 1202)) -eq '%y%V.%u') 'Saved format must be shown after reopening'
    Assert ([SettingsNative]::Text((Control $window 0 1101)) -like 'ElevenClock-rs keeps running*') 'Welcome text must only show once'
    Assert ([SettingsNative]::GetForegroundWindow() -eq $window) 'The second launch must bring Settings to the front'
    $steps.cancel = 'Cancel discarded the draft, the clock kept running and a second launch refocused Settings'

    # 8b. Support (R11): status, a bounded Refresh status, the previewed report, Copy
    #     diagnostics to the real clipboard, redaction and the bounded local log.
    Foreground $window
    1..3 | ForEach-Object { Press 0x22 0x11 }   # Ctrl+PageDown: Appearance, Startup, Support
    $status = [SettingsNative]::Text((Control $window $PAGE.Support 1501))
    Assert ($status -like 'ElevenClock-rs version *') "Support status must lead with the version: $status"
    Assert ($status -match 'Windows 10\.0\.\d+') "Support status must show the Windows build: $status"
    Assert ($status -match 'Status: ') "Support status must say where replacement stands: $status"
    $report = [SettingsNative]::Text((Control $window $PAGE.Support 1503))
    Assert ($report -like 'ElevenClock-rs diagnostics*') "Report preview missing: $report"
    foreach ($field in 'App version:', 'App commit:', 'Windows:', 'Architecture:', 'Clock replacement:', 'Taskbar discovery:', 'Taskbars:') {
        Assert ($report.Contains($field)) "Report is missing '$field'"
    }
    foreach ($private in @($env:USERNAME, $env:USERPROFILE, 'C:\Users')) {
        Assert (-not $report.ToLowerInvariant().Contains($private.ToLowerInvariant())) "Report leaks '$private'"
    }
    # Layout at the current scale: every Support control lies inside its page and no two
    # overlap, so nothing is clipped or covered at any DPI (checked at 100-200% in CI).
    $pageRect = New-Object SettingsNative+Rect
    [SettingsNative]::GetWindowRect((Control $window 0 $PAGE.Support), [ref]$pageRect) | Out-Null
    $rects = @{}
    foreach ($id in 1501, 1502, 1503, 1504, 1505) {
        $r = New-Object SettingsNative+Rect
        [SettingsNative]::GetWindowRect((Control $window $PAGE.Support $id), [ref]$r) | Out-Null
        Assert (($r.Left -ge $pageRect.Left) -and ($r.Top -ge $pageRect.Top) -and ($r.Right -le $pageRect.Right) -and ($r.Bottom -le $pageRect.Bottom)) "Support control $id lies outside its page"
        $rects[$id] = $r
    }
    foreach ($a in $rects.Keys) { foreach ($b in $rects.Keys) {
        if ($a -lt $b) {
            $x = $rects[$a]; $y = $rects[$b]
            $overlap = ($x.Left -lt $y.Right) -and ($y.Left -lt $x.Right) -and ($x.Top -lt $y.Bottom) -and ($y.Top -lt $x.Bottom)
            Assert (-not $overlap) "Support controls $a and $b overlap"
        }
    } }
    # What a screen reader is given: a name, a role and keyboard focusability. Edits are
    # named by the label before them; this is the data Narrator speaks, not its speech.
    $names = [ordered]@{}
    foreach ($entry in @(@(1501, 'status', 'Status*'), @(1502, 'refresh', 'Refresh status'), @(1503, 'report', 'Report text*'), @(1504, 'copy', 'Copy diagnostics'))) {
        $accessible = [SettingsNative]::Accessible((Control $window $PAGE.Support $entry[0]))
        $names[$entry[1]] = $accessible.accName(0)
        Assert ($names[$entry[1]] -like $entry[2]) "Support $($entry[1]) accessible name was '$($names[$entry[1]])', expected '$($entry[2])'"
        Assert ((([int]$accessible.accState(0)) -band 0x100000) -ne 0) "Support $($entry[1]) is not keyboard focusable"
    }
    $steps.supportAccessibleNames = ($names.GetEnumerator() | ForEach-Object { "$($_.Key)='$($_.Value)'" }) -join '; '
    # Refresh status answers within the bound and never leaves the dialog busy.
    $feedback = Control $window $PAGE.Support 1505
    # Keyboard only: Tab walks status, Refresh, report and Copy; Alt+F presses Refresh.
    $supportOrder = 1501, 1502, 1503, 1504
    $actual = @()
    1..$supportOrder.Count | ForEach-Object { Press 9; $actual += Focused-Id $window }
    Assert (($actual -join ',') -eq ($supportOrder -join ',')) "Support tab order was $($actual -join ',') but expected $($supportOrder -join ',')"
    [SettingsNative]::WatchAlerts([uint32]$app.Id)
    $alertsBefore = [SettingsNative]::AlertsFrom($feedback)
    Press 70 18
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while (([SettingsNative]::Text($feedback) -ne 'Status refreshed.') -and [DateTime]::UtcNow -lt $deadline) { Pump 200 }
    Assert ([SettingsNative]::Text($feedback) -eq 'Status refreshed.') "Refresh status did not finish: '$([SettingsNative]::Text($feedback))'"
    Pump 300
    # One alert when the check starts and one when it ends, both from the result line.
    Assert (([SettingsNative]::AlertsFrom($feedback) - $alertsBefore) -ge 2) 'Refresh status must announce that it started and finished'
    # Copy diagnostics writes exactly the previewed text, only because it was clicked.
    $clipboardBefore = Get-Clipboard -Raw
    $sentinel = "R11 sentinel $([Guid]::NewGuid())"
    Set-Clipboard -Value $sentinel
    Start-Sleep -Milliseconds 300
    Assert ((Get-Clipboard -Raw).Trim() -eq $sentinel) 'Opening and refreshing Support must not touch the clipboard'
    # Contention: a separate process holds the clipboard open. Copy must fail quickly
    # with an accessible message, leave the old contents alone and keep the window alive.
    $flag = Join-Path $env:TEMP "r11-clipboard-held-$([Guid]::NewGuid()).flag"
    $holderScript = Join-Path $env:TEMP "r11-clipboard-holder-$([Guid]::NewGuid()).ps1"
    Set-Content -Path $holderScript -Encoding UTF8 -Value @'
param([string]$Flag)
Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr owner); [DllImport("user32.dll")] public static extern bool CloseClipboard();' -Name Clip -Namespace Holder
if (-not [Holder.Clip]::OpenClipboard([IntPtr]::Zero)) { exit 2 }
Set-Content -Path $Flag -Value 'held'
Start-Sleep -Seconds 5
[Holder.Clip]::CloseClipboard() | Out-Null
'@
    $holder = Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $holderScript, $flag -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while (-not (Test-Path $flag) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
        Assert (Test-Path $flag) 'The clipboard holder process did not start'
        $held = [Diagnostics.Stopwatch]::StartNew()
        Click (Control $window $PAGE.Support 1504)   # includes a 500 ms settle
        $busy = [SettingsNative]::Text($feedback)
        Assert ($busy -like 'Diagnostics were not copied. Another program is using the clipboard*') "A held clipboard must be reported: '$busy'"
        Assert ($busy -like '*Your clipboard was not changed.') "A held clipboard must not be cleared: '$busy'"
        Assert ($held.Elapsed.TotalSeconds -lt 3) 'Copy blocked the window while the clipboard was held'
        Assert ([SettingsNative]::IsWindowEnabled((Control $window $PAGE.Support 1504))) 'The window must stay usable while the clipboard is held'
        $holder.WaitForExit(15000) | Out-Null
        Assert ((Get-Clipboard -Raw).Trim() -eq $sentinel) 'The old clipboard contents must survive a failed copy'
    } finally {
        if (-not $holder.HasExited) { $holder.Kill() }
        Remove-Item $flag, $holderScript -Force -ErrorAction SilentlyContinue
    }
    $report = [SettingsNative]::Text((Control $window $PAGE.Support 1503))
    $alertsBefore = [SettingsNative]::AlertsFrom($feedback)
    Click (Control $window $PAGE.Support 1504)
    Pump 300
    Assert (([SettingsNative]::AlertsFrom($feedback) - $alertsBefore) -ge 1) 'The copy result must be announced'
    Assert ([SettingsNative]::Text($feedback) -eq 'Diagnostics copied to the clipboard.') "Copy did not report success: '$([SettingsNative]::Text($feedback))'"
    $copied = (Get-Clipboard -Raw).Replace("`r`n", "`n")
    Assert ($copied.TrimEnd() -eq $report.TrimEnd()) 'The clipboard must hold exactly the previewed report'
    if ($null -ne $clipboardBefore) { Set-Clipboard -Value $clipboardBefore } else { Set-Clipboard -Value $null }
    # The bounded local log exists, names the start and holds no personal data.
    $log = Join-Path $env:LOCALAPPDATA 'ElevenClock-rs\logs\elevenclock-rs.log'
    Assert (Test-Path $log) 'The local log was not created'
    $logText = Get-Content $log -Raw
    Assert ($logText -like '*ElevenClock-rs * started*') 'The log must record the start'
    Assert (-not $logText.ToLowerInvariant().Contains($env:USERNAME.ToLowerInvariant())) 'The log must not contain the user name'
    Assert ((Get-ChildItem (Split-Path $log) -Filter 'elevenclock-rs*.log' | Measure-Object -Property Length -Maximum).Maximum -le 1MB) 'A log file exceeds 1 MiB'
    Capture-Window $window 'support.png'
    [IO.File]::WriteAllText((Join-Path (Resolve-Path $OutputDirectory) 'support-report.txt'), $report)
    Press 0x22 0x11                # Ctrl+PageDown: back to Date and time
    $steps.support = 'Support status, previewed report (no user name or paths), bounded Refresh status, exact clipboard copy and bounded log verified'

    # 9. Reset stages defaults; nothing is saved until Apply.
    Click (Control $window 0 1104)
    Assert ([SettingsNative]::Text((Control $window $PAGE.Date 1202)) -eq '%H:%M' + "`n" + '%g%V.%u') 'Reset must stage the default format'
    Assert ((Get-Content $preferences -Raw) -like '*%y%V.%u*') 'Reset must not write preferences'
    Assert ([SettingsNative]::IsWindowEnabled((Control $window 0 1105))) 'Reset must leave Apply available'
    $steps.reset = 'Reset staged the defaults without saving'

    # 10. Capture, then close with the requested gesture.
    $rect = New-Object SettingsNative+Rect
    if (![SettingsNative]::GetWindowRect($window, [ref]$rect)) { throw 'Could not measure Settings window' }
    $bitmap = [Drawing.Bitmap]::new(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try { if (![SettingsNative]::PrintWindow($window, $dc, 2)) { throw 'Native dialog capture failed' } }
            finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path (Resolve-Path $OutputDirectory) 'settings.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    $os = Get-CimInstance Win32_OperatingSystem
    [ordered]@{
        os = $os.Caption
        version = $os.Version
        build = $os.BuildNumber
        dpi = $dpi
        title = 'ElevenClock-rs Settings'
        tabOrder = $expected
        steps = $steps
        closeGesture = $CloseGesture
        capture = 'PrintWindow of the actual native dialog'
        interactiveWindows11Acceptance = 'not performed by this CI harness'
        commit = $env:GITHUB_SHA
    } | ConvertTo-Json -Depth 4 | Tee-Object -FilePath (Join-Path $OutputDirectory 'context.json')
    Foreground $window
    switch ($CloseGesture) {
        'Window' { [SettingsNative]::SendMessageW($window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null } # WM_CLOSE
        'Escape' { [SettingsNative]::KeyStroke(27, 0) }
        'Enter' { [SettingsNative]::KeyStroke(13, 0) } # OK is the default button: it applies the staged reset, then closes
        'Mnemonic' { [SettingsNative]::KeyStroke(67, 18) } # Alt+C: Cancel
    }
    # Closing Settings closes only the window: the clock keeps running (R08).
    Wait-Settings $false 'Settings to close'
    Start-Sleep -Milliseconds 500
    $app.Refresh()
    if ($app.HasExited) { throw 'Closing Settings stopped the application; it must keep running' }
    $saved = Get-Content $preferences -Raw
    if ($CloseGesture -eq 'Enter') {
        Assert (($saved -like '*%g%V.%u*') -and ($saved -notlike '*%y%V.%u*')) 'OK must apply the staged reset before closing'
    } else {
        Assert ($saved -like '*%y%V.%u*') 'Closing without OK must discard the staged reset'
    }

    # 11. A second launch signals the existing process, shows Settings there and exits.
    $again = Start-Process $Executable -PassThru
    if (!$again.WaitForExit(15000)) { $again.Kill(); throw 'Second launch did not exit' }
    if ($again.ExitCode -ne 0) { throw "Second launch exit code $($again.ExitCode)" }
    Wait-Settings $true 'second launch to show Settings in the existing process'
    $windows = @(Get-Process -Name elevenclock-rs -ErrorAction SilentlyContinue | Where-Object { [SettingsNative]::Settings($_.Id) -ne [IntPtr]::Zero })
    if ($windows.Count -ne 1) { throw "Expected one Settings window, found $($windows.Count)" }

    # 12. Exit stops the application and releases everything it started.
    $exit = [SettingsNative]::GetDlgItem([SettingsNative]::Settings($app.Id), 1003)
    if ($exit -eq [IntPtr]::Zero) { throw 'Exit button not found' }
    [SettingsNative]::SendMessageW($exit, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (!$app.WaitForExit(10000)) { throw 'Exit did not stop the application' }
    if ($app.ExitCode -ne 0) { throw 'Application reported failure on Exit' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 100
        $left = @(Get-Process -Name elevenclock-rs -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $baseline })
    } while ($left.Count -gt 0 -and [DateTime]::UtcNow -lt $deadline)
    if ($left.Count -gt 0) { throw "Processes remained after Exit: $($left.Id -join ',')" }
} finally {
    $app.Refresh()
    if (!$app.HasExited) { $app.Kill() }
    $app.Dispose()
    if ($null -ne $backup) {
        New-Item -ItemType Directory -Path (Split-Path $preferences) -Force | Out-Null
        [IO.File]::WriteAllBytes($preferences, $backup)
    } elseif (Test-Path $preferences) { Remove-Item $preferences -Force }
}
