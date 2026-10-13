# Development-only UI feasibility checks on an ephemeral Windows CI desktop.
# Uses supported high-contrast APIs and Windows Settings UI for display scaling.
# Does not use private DPI APIs, alter the product or ship with the executable.
param([string]$OutputDirectory = 'native-evidence/matrix')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class UiEnvironment {
    [StructLayout(LayoutKind.Sequential)]
    public struct HighContrast { public uint Size, Flags; public IntPtr Scheme; }
    [DllImport("user32.dll", EntryPoint="SystemParametersInfoW", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern bool Parameters(uint action, uint param, ref HighContrast value, uint flags);
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
$results = [ordered]@{highContrast = 'not run'; scaling = 'not run'; displayControls = @()}
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
    $results | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $OutputDirectory 'environment.json')
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
try {
    Start-Process 'ms-settings:display'
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Milliseconds 300
        $settings = $root.FindFirst([Windows.Automation.TreeScope]::Children, $windowCondition)
    } while (!$settings -and [DateTime]::UtcNow -lt $deadline)
    if (!$settings) { throw 'Windows Settings display page unavailable on this runner' }
    $comboCondition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ComboBox)
    $combos = $settings.FindAll([Windows.Automation.TreeScope]::Descendants, $comboCondition)
    $results.displayControls = @($combos | ForEach-Object { [ordered]@{name = $_.Current.Name; id = $_.Current.AutomationId} })
    foreach ($combo in $combos) {
        if ($combo.Current.Name -match 'Scale|size of text' -or $combo.Current.AutomationId -match 'Scale|Dpi') { $scale = $combo; break }
    }
    if (!$scale) { throw 'Display Scale selector unavailable on this runner' }
    $selection = $scale.GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern)
    $selected = $selection.GetCurrentSelection()
    if ($selected.Count -ne 1) { throw 'Cannot record original display scale' }
    $originalScale = $selected[0].Current.Name
    function Set-Scale([string]$Label) {
        $scale.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
        Start-Sleep -Milliseconds 300
        $items = $settings.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ListItem))
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
    if ($scale -and $originalScale) { Set-Scale $originalScale }
    if ($settings) { $settings.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close() }
    $results | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $OutputDirectory 'environment.json')
}
