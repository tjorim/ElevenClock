# Development-only native smoke harness; never part of the installed app.
param(
    [string]$Executable = 'rust/target/x86_64-pc-windows-msvc/release/clock-app.exe',
    [string]$OutputDirectory = 'native-evidence'
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
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetNextDlgTabItem(IntPtr dialog, IntPtr control, bool previous);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr control);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
}
'@
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
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
    $first = [SettingsNative]::GetNextDlgTabItem($window, [IntPtr]::Zero, $false)
    $second = [SettingsNative]::GetNextDlgTabItem($window, $first, $false)
    $wrap = [SettingsNative]::GetNextDlgTabItem($window, $second, $false)
    if ([SettingsNative]::GetDlgCtrlID($first) -ne 1001) { throw 'First tab stop must be Appearance' }
    if ([SettingsNative]::GetDlgCtrlID($second) -ne 2) { throw 'Second tab stop must be Close' }
    if ($wrap -ne $first) { throw 'Unexpected tab stop or broken tab cycle' }
    # WM_NEXTDLGCTL exercises real forward/reverse dialog focus navigation.
    # This is message-based evidence, not an interactive keyboard acceptance run.
    [SettingsNative]::SendMessageW($window, 0x0028, $first, [IntPtr]1) | Out-Null
    [uint32]$processId = 0
    $thread = [SettingsNative]::GetWindowThreadProcessId($window, [ref]$processId)
    $info = New-Object SettingsNative+GuiInfo
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
    [SettingsNative]::SendMessageW($window, 0x0028, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (![SettingsNative]::GetGUIThreadInfo($thread, [ref]$info) -or $info.Focus -ne $second) { throw 'Forward focus did not reach Close' }
    [SettingsNative]::SendMessageW($window, 0x0028, [IntPtr]1, [IntPtr]::Zero) | Out-Null
    if (![SettingsNative]::GetGUIThreadInfo($thread, [ref]$info) -or $info.Focus -ne $first) { throw 'Reverse focus did not reach Appearance' }
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
        tabOrder = @(1001, 2)
        forwardAndReverseDialogFocus = 'passed'
        capture = 'PrintWindow of the actual native dialog'
        interactiveKeyboardDpiThemeAcceptance = 'not performed by this harness'
        commit = $env:GITHUB_SHA
    } | ConvertTo-Json | Tee-Object -FilePath (Join-Path $OutputDirectory 'context.json')
    if (!$app.CloseMainWindow()) { throw 'Could not request native window close' }
    if (!$app.WaitForExit(5000)) { throw 'Prototype did not close' }
    if ($app.ExitCode -ne 0) { throw 'Prototype reported failure' }
} finally {
    $app.Refresh()
    if (!$app.HasExited) { $app.Kill() }
    $app.Dispose()
}
