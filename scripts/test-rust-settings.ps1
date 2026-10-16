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
Add-Type @'
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
