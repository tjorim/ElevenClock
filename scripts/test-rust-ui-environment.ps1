# Development-only UI feasibility checks on an ephemeral Windows CI desktop.
# Uses supported high-contrast APIs and Windows Settings UI for display scaling.
# Does not use private DPI APIs, alter the product or ship with the executable.
param([string]$OutputDirectory = 'native-evidence/matrix')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName Accessibility
Add-Type -ReferencedAssemblies @('System.dll', [Accessibility.IAccessible].Assembly.Location) @'
using System;
using System.Runtime.InteropServices;
public static class UiEnvironment {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    public struct DisplayMode {
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string Device;
        public ushort SpecVersion, DriverVersion, Size, Extra;
        public uint Fields;
        public int X, Y;
        public uint Orientation, FixedOutput;
        public short Color, Duplex, YResolution, TTOption, Collate;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string Form;
        public ushort LogPixels;
        public uint Bits, Width, Height, Flags, Frequency, IcmMethod, IcmIntent,
            Media, Dither, Reserved1, Reserved2, PanningWidth, PanningHeight;
    }
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern bool EnumDisplaySettingsW(string device, int index, ref DisplayMode mode);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int ChangeDisplaySettingsExW(string device, ref DisplayMode mode, IntPtr window, uint flags, IntPtr parameter);
    public static DisplayMode ReadMode(int index) {
        var mode = new DisplayMode {Size = (ushort)Marshal.SizeOf(typeof(DisplayMode))};
        if (!EnumDisplaySettingsW(null, index, ref mode)) throw new InvalidOperationException("Display mode unavailable");
        return mode;
    }
    public static DisplayMode[] Modes() {
        var modes = new System.Collections.Generic.List<DisplayMode>();
        for (int index=0; index<512; index++) {
            var mode = new DisplayMode {Size = (ushort)Marshal.SizeOf(typeof(DisplayMode))};
            if (!EnumDisplaySettingsW(null, index, ref mode)) break;
            modes.Add(mode);
        }
        return modes.ToArray();
    }
    public static void ApplyMode(DisplayMode mode) {
        mode.Fields = 0x00180000; // DM_PELSWIDTH | DM_PELSHEIGHT
        int result = ChangeDisplaySettingsExW(null, ref mode, IntPtr.Zero, 0, IntPtr.Zero);
        if (result != 0) throw new InvalidOperationException("Actual display mode change failed: " + result);
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Mouse { public int X, Y; public uint Data, Flags, Time; public IntPtr Extra; }
    [StructLayout(LayoutKind.Explicit, Size=40)]
    public struct Input { [FieldOffset(0)] public uint Type; [FieldOffset(8)] public Mouse Mouse; }
    [DllImport("user32.dll")] public static extern uint SendInput(uint count, Input[] values, int size);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    public static void DoubleClick(int x, int y) {
        if (!Environment.Is64BitProcess) throw new InvalidOperationException("Harness expects x64");
        var values = new [] {
            new Input {Mouse = new Mouse {X=(int)(x*65535.0/(GetSystemMetrics(0)-1)), Y=(int)(y*65535.0/(GetSystemMetrics(1)-1)), Flags=0x8001}},
            new Input {Mouse = new Mouse {Flags=2}}, new Input {Mouse = new Mouse {Flags=4}},
            new Input {Mouse = new Mouse {Flags=2}}, new Input {Mouse = new Mouse {Flags=4}}
        };
        if (SendInput(5, values, Marshal.SizeOf(typeof(Input))) != 5) {
            SendInput(1, new [] {new Input {Mouse = new Mouse {Flags=4}}}, Marshal.SizeOf(typeof(Input)));
            throw new InvalidOperationException("Windows mouse input injection failed");
        }
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct HighContrast { public uint Size, Flags; public IntPtr Scheme; }
    [DllImport("user32.dll", EntryPoint="SystemParametersInfoW", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool Parameters(uint action, uint param, ref HighContrast value, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dialog, int id);
    [DllImport("oleacc.dll")] public static extern int AccessibleObjectFromWindow(IntPtr window, uint objectId, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out Accessibility.IAccessible accessible);
    public static Accessibility.IAccessible Accessible(IntPtr window) {
        Guid iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        Accessibility.IAccessible accessible;
        Marshal.ThrowExceptionForHR(AccessibleObjectFromWindow(window, 0xfffffffc, ref iid, out accessible));
        if (accessible == null) throw new InvalidOperationException("Native accessibility object unavailable");
        return accessible;
    }
    [DllImport("user32.dll")] public static extern uint GetSysColor(int index);
    public static HighContrast Read() {
        var value = new HighContrast {Size = (uint)Marshal.SizeOf(typeof(HighContrast))};
        if (!Parameters(0x42, value.Size, ref value, 0)) throw new System.ComponentModel.Win32Exception();
        return value;
    }
    public static void Apply(uint flags, string scheme) {
        var value = new HighContrast {Size = (uint)Marshal.SizeOf(typeof(HighContrast)), Flags = flags,
            Scheme = Marshal.StringToHGlobalUni(scheme)};
        try { if (!Parameters(0x43, value.Size, ref value, 2)) throw new System.ComponentModel.Win32Exception(); }
        finally { Marshal.FreeHGlobal(value.Scheme); }
    }
}
'@
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$original = [UiEnvironment]::Read()
$originalScheme = [Runtime.InteropServices.Marshal]::PtrToStringUni($original.Scheme)
$results = [ordered]@{highContrast = 'not run'; scaling = 'not run'; displayControls = @(); scaleAttempts = @(); displayModes = @()}
function Run-Prototype([string]$Case, [int]$Dpi = 0, [string]$Close = 'Window') {
    & pwsh -NoProfile -File scripts/test-rust-settings.ps1 -OutputDirectory (Join-Path $OutputDirectory $Case) -ExpectedDpi $Dpi -CloseGesture $Close
    if ($LASTEXITCODE -ne 0) { throw "Native prototype failed in $Case" }
}
try {
    [UiEnvironment]::Apply(($original.Flags -bor 1), 'High Contrast Black')
    Start-Sleep -Milliseconds 800
    $active = [UiEnvironment]::Read()
    if (($active.Flags -band 1) -eq 0) { throw 'Windows did not enable high contrast' }
    Run-Prototype 'high-contrast' 0 'Mnemonic'
    $results.highContrast = [ordered]@{
        state = 'enabled and captured'
        flags = $active.Flags
        scheme = [Runtime.InteropServices.Marshal]::PtrToStringUni($active.Scheme)
        windowColor = [UiEnvironment]::GetSysColor(5)
        windowTextColor = [UiEnvironment]::GetSysColor(8)
        buttonFaceColor = [UiEnvironment]::GetSysColor(15)
        buttonTextColor = [UiEnvironment]::GetSysColor(18)
    }
} finally {
    [UiEnvironment]::Apply($original.Flags, $originalScheme)
    $results | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'environment.json')
}
# Verify distinct ordinary dialog exit gestures before changing display settings.
Run-Prototype 'keyboard-escape' 0 'Escape'
Run-Prototype 'keyboard-enter' 0 'Enter'
# UI Automation changes the real Windows Settings scale selection. The child
# harness independently verifies GetDpiForWindow; a UI selection alone never passes.
$root = [Windows.Automation.AutomationElement]::RootElement
$windowCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, 'Settings')
$settings = $null
$scale = $null
$originalScale = $null
$originalMode = [UiEnvironment]::ReadMode(-1)
$modeChanged = $false
try {
    $modes = [UiEnvironment]::Modes()
    $results.displayModes = @($modes | ForEach-Object { [ordered]@{width=$_.Width; height=$_.Height; bits=$_.Bits; frequency=$_.Frequency} })
    # The initial 1024x768 mode limits Settings to 125%. Pick an actual driver
    # mode with sufficient logical desktop area at 200%, rather than private DPI APIs.
    $largeMode = $modes | Where-Object { $_.Width -ge 1600 -and $_.Height -ge 1200 -and $_.Bits -eq $originalMode.Bits } | Sort-Object { [long]$_.Width * $_.Height } | Select-Object -First 1
    if (!$largeMode) { throw 'Display driver exposes no mode large enough for the 200% scaling test' }
    [UiEnvironment]::ApplyMode($largeMode)
    $modeChanged = $true
    Start-Sleep -Seconds 1
    $actualMode = [UiEnvironment]::ReadMode(-1)
    if ($actualMode.Width -ne $largeMode.Width -or $actualMode.Height -ne $largeMode.Height) { throw 'Driver did not apply the requested display mode' }
    $results.activeDisplayMode = [ordered]@{width=$actualMode.Width; height=$actualMode.Height}

    Start-Process 'ms-settings:display'
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 300
        $settings = $root.FindFirst([Windows.Automation.TreeScope]::Children, $windowCondition)
    } while (!$settings -and [DateTime]::UtcNow -lt $deadline)
    if (!$settings) { throw 'Windows Settings display page unavailable on this runner' }
    $comboCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ComboBox)
    # Window creation precedes loading the asynchronous Display Settings page.
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $combos = $settings.FindAll([Windows.Automation.TreeScope]::Descendants, $comboCondition)
        foreach ($combo in $combos) {
            if ($combo.Current.Name -match 'Scale|size of text' -or $combo.Current.AutomationId -match 'Scale|Dpi') { $scale = $combo; break }
        }
        if (!$scale) { Start-Sleep -Milliseconds 300 }
    } while (!$scale -and [DateTime]::UtcNow -lt $deadline)
    $results.displayControls = @($combos | ForEach-Object { [ordered]@{name = $_.Current.Name; id = $_.Current.AutomationId} })
    if (!$scale) { throw 'Display Scale selector unavailable on this runner' }
    $selection = $scale.GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern)
    $selected = $selection.Current.GetSelection()
    if ($selected.Count -ne 1) { throw 'Cannot record original display scale' }
    $originalScale = $selected[0].Current.Name
    $scaleId = $scale.Current.AutomationId
    function Set-Scale([string]$Label) {
        # Reacquire provider elements after display changes and activate Settings
        # after the prototype's keyboard/close tests return focus to the runner.
        $settings = [Windows.Automation.AutomationElement]::RootElement.FindFirst([Windows.Automation.TreeScope]::Children, $windowCondition)
        if (!$settings) { throw 'Settings window disappeared during scale test' }
        $scale = $settings.FindFirst([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $scaleId))
        [UiEnvironment]::SetForegroundWindow([IntPtr]($settings.Current.NativeWindowHandle)) | Out-Null
        $scale.SetFocus()
        $scale.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
        Start-Sleep -Milliseconds 300
        $items = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ListItem))
        $results.scaleAttempts += [ordered]@{requested = $Label; available = @($items | ForEach-Object { $_.Current.Name })}
        $target = $null
        foreach ($item in $items) {
            if ($item.Current.Name -eq $Label -or $item.Current.Name -like "$Label *") { $target = $item; break }
        }
        if (!$target) { throw "Actual display scale option $Label unavailable" }
        $target.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
        Start-Sleep -Seconds 1
    }
    foreach ($dpi in @(96, 120, 144, 192)) {
        $percent = [int]($dpi * 100 / 96)
        Set-Scale "$percent%"
        Run-Prototype "scale-$percent" $dpi
    }
    $results.scaling = 'Actual window DPI verified at 100/125/150/200 percent'
} catch {
    $results.scaling = 'Unavailable or failed: ' + $_.Exception.Message
    throw
} finally {
    try {
        if ($scale -and $originalScale) { Set-Scale $originalScale }
    } finally {
        if ($modeChanged) { [UiEnvironment]::ApplyMode($originalMode) }
    }
    if ($settings) { $settings.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close() }
    $results | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'environment.json')
}

# Exercise the literal executable double-click path through Explorer, then query
# the native accessible labels. This remains a developer-only desktop harness.
$explorer = $null
$opened = $null
try {
    $before = @(Get-Process -Name clock-app -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
    $folder = (Resolve-Path 'rust/target/x86_64-pc-windows-msvc/release').Path
    Start-Process explorer.exe -ArgumentList ('"' + $folder + '"')
    $cabinet = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ClassNameProperty, 'CabinetWClass')
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 300
        $windows = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $cabinet)
        foreach ($window in $windows) { if ($window.Current.Name -like '*release*') { $explorer = $window; break } }
    } while (!$explorer -and [DateTime]::UtcNow -lt $deadline)
    if (!$explorer) { throw 'Could not open the executable folder in Explorer' }
    $handle = [IntPtr]($explorer.Current.NativeWindowHandle)
    $explorer.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).SetWindowVisualState([Windows.Automation.WindowVisualState]::Normal)
    [UiEnvironment]::SetForegroundWindow($handle) | Out-Null
    $file = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $items = $explorer.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ListItem))
        foreach ($item in $items) { if ($item.Current.Name -in @('clock-app', 'clock-app.exe')) { $file = $item; break } }
        if (!$file) { Start-Sleep -Milliseconds 200 }
    } while (!$file -and [DateTime]::UtcNow -lt $deadline)
    if (!$file) { throw 'Executable file item did not load in Explorer' }
    if ($file.Current.IsOffscreen) { $file.GetCurrentPattern([Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView(); Start-Sleep -Milliseconds 200 }
    if ($file.Current.IsOffscreen) { throw 'Executable file item is not visible in Explorer' }
    if ([UiEnvironment]::GetForegroundWindow() -ne $handle) { throw 'Explorer could not be activated for the mouse test' }
    $bounds = $file.Current.BoundingRectangle
    [UiEnvironment]::DoubleClick([int]($bounds.Left + [Math]::Min(40, $bounds.Width/2)), [int]($bounds.Top + $bounds.Height/2))
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 200
        $opened = Get-Process -Name clock-app -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $before } | Select-Object -First 1
        if ($opened) { $opened.Refresh() }
    } while ((!$opened -or $opened.MainWindowHandle -eq 0) -and [DateTime]::UtcNow -lt $deadline)
    if (!$opened -or $opened.MainWindowTitle -ne 'ElevenClock Settings') { throw 'Explorer double-click did not open native Settings' }
    if (!$opened.WaitForInputIdle(5000)) { throw 'Prototype did not reach its input loop after double-click' }
    # Classic controls expose their default IAccessible objects through Oleacc.
    # Query those directly: the legacy .NET UIA client omits the classic-control
    # proxies on this runner, while native Settings/Explorer providers still work.
    $combo = [UiEnvironment]::Accessible([UiEnvironment]::GetDlgItem($opened.MainWindowHandle, 1001))
    $button = [UiEnvironment]::Accessible([UiEnvironment]::GetDlgItem($opened.MainWindowHandle, 2))
    $name = $combo.accName(0)
    $closeName = $button.accName(0)
    $focusable = (([int]$combo.accState(0)) -band 0x100000) -ne 0
    $results.accessibleLabels = [ordered]@{provider='Native IAccessible (Oleacc standard-control proxy)'; appearance=$name; appearanceFocusable=$focusable; close=$closeName; role=$combo.accRole(0)}
    if ($name -notlike 'Appearance*' -or !$focusable) { throw "Native Appearance label/focus failed: name='$name', focusable=$focusable" }
    if ($closeName -ne 'Close') { throw "Native Close label failed: '$closeName'" }
    $results.explorerDoubleClick = 'Real queued left-button double-click opened the native executable'
    if (!$opened.CloseMainWindow() -or !$opened.WaitForExit(5000) -or $opened.ExitCode -ne 0) { throw 'Double-clicked prototype did not close normally' }
} finally {
    if ($opened) { $opened.Refresh(); if (!$opened.HasExited) { $opened.Kill() }; $opened.Dispose() }
    if ($explorer) { $explorer.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close() }
    $results | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'environment.json')
}
