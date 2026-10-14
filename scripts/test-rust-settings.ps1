# Development-only native smoke harness; never part of the installed app.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/release/clock-app.exe',
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
public static class SettingsNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct GuiInfo {
        public int Size, Flags;
        public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        public Rect CaretRect;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Keyboard { public ushort Key, Scan; public uint Flags, Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(8)] public Keyboard Keyboard;
    }
    [DllImport("user32.dll")] public static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
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
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetNextDlgTabItem(IntPtr dialog, IntPtr control, bool previous);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr control);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
}
'@
[SettingsNative]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
# Processes of this executable that exist before the run (other tests, helpers).
$baseline = @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$app = Start-Process $Executable -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
        if ($app.HasExited) { throw 'Prototype exited before opening Settings' }
    } while ($app.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $deadline)
    if ($app.MainWindowTitle -ne 'ElevenClock Settings') { throw 'Expected native Settings window' }
    $window = $app.MainWindowHandle
    $dpi = [SettingsNative]::GetDpiForWindow($window)
    if ($ExpectedDpi -ne 0 -and $dpi -ne $ExpectedDpi) { throw "Actual window DPI $dpi differs from requested $ExpectedDpi" }
    $first = [SettingsNative]::GetNextDlgTabItem($window, [IntPtr]::Zero, $false)
    $second = [SettingsNative]::GetNextDlgTabItem($window, $first, $false)
    $third = [SettingsNative]::GetNextDlgTabItem($window, $second, $false)
    $wrap = [SettingsNative]::GetNextDlgTabItem($window, $third, $false)
    if ([SettingsNative]::GetDlgCtrlID($first) -ne 1001) { throw 'First tab stop must be Appearance' }
    if ([SettingsNative]::GetDlgCtrlID($second) -ne 1003) { throw 'Second tab stop must be Exit' }
    if ([SettingsNative]::GetDlgCtrlID($third) -ne 2) { throw 'Third tab stop must be Close' }
    if ($wrap -ne $first) { throw 'Unexpected tab stop or broken tab cycle' }
    # WM_NEXTDLGCTL exercises real forward/reverse dialog focus navigation.
    # This is message-based evidence, not an interactive keyboard acceptance run.
    [SettingsNative]::SendMessageW($window, 0x0028, $first, [IntPtr]1) | Out-Null
    [uint32]$processId = 0
    $thread = [SettingsNative]::GetWindowThreadProcessId($window, [ref]$processId)
    $info = New-Object SettingsNative+GuiInfo
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
    [SettingsNative]::SendMessageW($window, 0x0028, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (![SettingsNative]::GetGUIThreadInfo($thread, [ref]$info) -or $info.Focus -ne $second) { throw 'Forward focus did not reach Exit' }
    [SettingsNative]::SendMessageW($window, 0x0028, [IntPtr]1, [IntPtr]::Zero) | Out-Null
    if (![SettingsNative]::GetGUIThreadInfo($thread, [ref]$info) -or $info.Focus -ne $first) { throw 'Reverse focus did not reach Appearance' }
    # Input goes through the actual Windows input queue and modal dialog manager.
    [SettingsNative]::SetForegroundWindow($window) | Out-Null
    if ([SettingsNative]::GetForegroundWindow() -ne $window) { throw 'Keyboard test cannot activate the actual dialog' }
    function Assert-Focus([IntPtr]$Expected, [string]$Gesture) {
        $deadline = [DateTime]::UtcNow.AddSeconds(3)
        do {
            Start-Sleep -Milliseconds 50
            if ([SettingsNative]::GetGUIThreadInfo($thread, [ref]$info) -and $info.Focus -eq $Expected) { return }
        } while ([DateTime]::UtcNow -lt $deadline)
        throw "$Gesture did not focus the expected control"
    }
    [SettingsNative]::KeyStroke(9, 0)
    Assert-Focus $second 'Tab'
    [SettingsNative]::KeyStroke(9, 16)
    Assert-Focus $first 'Shift+Tab'
    [SettingsNative]::KeyStroke(9, 0)
    Assert-Focus $second 'Tab'
    [SettingsNative]::KeyStroke(65, 18)
    Assert-Focus $first 'Alt+A'
    # Exercise the theme selector with real arrow input and retain its limitation.
    [SettingsNative]::KeyStroke(40, 0)
    Start-Sleep -Milliseconds 100
    if ([SettingsNative]::SendMessageW($first, 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -ne 1) { throw 'Down arrow did not select Light' }
    [SettingsNative]::KeyStroke(40, 0)
    Start-Sleep -Milliseconds 100
    if ([SettingsNative]::SendMessageW($first, 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -ne 2) { throw 'Down arrow did not select Dark' }
    [SettingsNative]::KeyStroke(38, 0)
    [SettingsNative]::KeyStroke(38, 0)
    Start-Sleep -Milliseconds 100
    if ([SettingsNative]::SendMessageW($first, 0x0147, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -ne 0) { throw 'Up arrow did not restore System' }
    $rect = New-Object SettingsNative+Rect
    if (![SettingsNative]::GetWindowRect($window, [ref]$rect)) { throw 'Could not measure Settings window' }
    $bitmap = [Drawing.Bitmap]::new(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try {
                if (![SettingsNative]::PrintWindow($window, $dc, 2)) { throw 'Native dialog capture failed' }
            } finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path (Resolve-Path $OutputDirectory) 'settings.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    $os = Get-CimInstance Win32_OperatingSystem
    [ordered]@{
        os = $os.Caption
        version = $os.Version
        build = $os.BuildNumber
        dpi = [SettingsNative]::GetDpiForWindow($window)
        title = $app.MainWindowTitle
        tabOrder = @(1001, 1003, 2)
        forwardAndReverseDialogFocus = 'passed'
        queuedKeyboardInput = 'Tab, Shift+Tab, Alt+A and theme-selection arrows passed'
        closeGesture = $CloseGesture
        capture = 'PrintWindow of the actual native dialog'
        interactiveWindows11Acceptance = 'not performed by this Server CI harness'
        commit = $env:GITHUB_SHA
    } | ConvertTo-Json | Tee-Object -FilePath (Join-Path $OutputDirectory 'context.json')
    function Wait-Settings([bool]$Open, [string]$What) {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            Start-Sleep -Milliseconds 100
            $app.Refresh()
            if ($app.HasExited) { throw "Application exited while waiting for: $What" }
            $isOpen = $app.MainWindowHandle -ne 0 -and $app.MainWindowTitle -eq 'ElevenClock Settings'
        } while ($isOpen -ne $Open -and [DateTime]::UtcNow -lt $deadline)
        if ($isOpen -ne $Open) { throw "Timed out waiting for: $What" }
    }
    switch ($CloseGesture) {
        'Window' { if (!$app.CloseMainWindow()) { throw 'Could not request native window close' } }
        'Escape' { [SettingsNative]::KeyStroke(27, 0) }
        'Enter' { [SettingsNative]::KeyStroke(13, 0) }
        'Mnemonic' { [SettingsNative]::KeyStroke(67, 18) }
    }
    # Closing Settings closes only the window: the clock keeps running (R08).
    Wait-Settings $false 'Settings to close'
    Start-Sleep -Milliseconds 500
    $app.Refresh()
    if ($app.HasExited) { throw 'Closing Settings stopped the application; it must keep running' }
    # A second launch signals the existing process, shows Settings there and exits.
    $again = Start-Process $Executable -PassThru
    if (!$again.WaitForExit(15000)) { $again.Kill(); throw 'Second launch did not exit' }
    if ($again.ExitCode -ne 0) { throw "Second launch exit code $($again.ExitCode)" }
    Wait-Settings $true 'second launch to show Settings in the existing process'
    $windows = @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -eq 'ElevenClock Settings' })
    if ($windows.Count -ne 1) { throw "Expected one Settings window, found $($windows.Count)" }
    # Exit stops the application and releases everything it started.
    $exit = [SettingsNative]::GetDlgItem($app.MainWindowHandle, 1003)
    if ($exit -eq [IntPtr]::Zero) { throw 'Exit button not found' }
    [SettingsNative]::SendMessageW($exit, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (!$app.WaitForExit(10000)) { throw 'Exit did not stop the application' }
    if ($app.ExitCode -ne 0) { throw 'Application reported failure on Exit' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 100
        $left = @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $baseline })
    } while ($left.Count -gt 0 -and [DateTime]::UtcNow -lt $deadline)
    if ($left.Count -gt 0) { throw "Processes remained after Exit: $($left.Id -join ',')" }
} finally {
    $app.Refresh()
    if (!$app.HasExited) { $app.Kill() }
    $app.Dispose()
}
