# Development harness for an explicitly disposable Windows account. Never shipped.
# This writes real HKCU startup values and installs/uninstalls the product.
param(
    [string]$PackageDirectory = 'dist/candidate',
    [string]$OutputDirectory = 'native-evidence/packaging',
    [string]$PreviousInstaller = '',
    [switch]$DisposableAccount
)
$ErrorActionPreference = 'Stop'
if (!$DisposableAccount) { throw 'Use only in a disposable account, with -DisposableAccount' }
$packageRoot = (Resolve-Path -LiteralPath $PackageDirectory).Path
$package = Get-Content -LiteralPath "$packageRoot/package.json" -Raw | ConvertFrom-Json
$installer = Join-Path $packageRoot "$($package.name)-setup.exe"
$install = Join-Path $env:LOCALAPPDATA 'Programs/ElevenClock-rs'
$private = Join-Path $env:LOCALAPPDATA 'ElevenClock-rs'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'ElevenClock-rs.lnk'
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$value = 'ElevenClockRust'
function Assert($condition, [string]$message) { if (!$condition) { throw $message } }
function Read-Run([string]$name) { (Get-ItemProperty -LiteralPath $run -Name $name -ErrorAction SilentlyContinue).$name }
Assert (!(Test-Path $install) -and !(Test-Path $private) -and !(Test-Path $shortcut)) 'Account already has an installation or private data'
Assert (!(Get-Process -Name elevenclock-rs,ElevenClock -ErrorAction SilentlyContinue)) 'Exit all Rust and legacy overlay apps before testing'
Assert ($null -eq (Read-Run $value) -and $null -eq (Read-Run 'elevenClock')) 'Account already owns startup registration'
Assert ($null -eq (Get-ItemProperty -LiteralPath $approved -Name $value -ErrorAction SilentlyContinue)) 'Account already owns StartupApproved state'
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$evidence = (Resolve-Path -LiteralPath $OutputDirectory).Path
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$results = [ordered]@{
    os = [Environment]::OSVersion.Version.ToString()
    artifact = $package.name
    administratorGroup = ($identity.Groups.Value -contains 'S-1-5-32-544')
    elevated = ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    cleanRuntimeMachine = $false  # Hosted images have developer runtimes installed.
    previousReleaseRollback = 'not run: no earlier installer supplied'
    status = 'failed or interrupted; inspect installer logs'
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class PackageWindow {
  delegate bool Callback(IntPtr window, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumWindows(Callback callback, IntPtr data);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int max);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint msg, IntPtr w, IntPtr l);
  public static IntPtr Settings(int process) {
    IntPtr result = IntPtr.Zero;
    EnumWindows((window, data) => {
      uint pid; GetWindowThreadProcessId(window, out pid);
      var title = new StringBuilder(256); GetWindowText(window, title, 256);
      if (pid == process && title.ToString() == "ElevenClock-rs Settings") result = window;
      return true;
    }, IntPtr.Zero);
    return result;
  }
}
'@
function Invoke-Setup([string]$file, [string]$label, [string]$directory = '') {
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$evidence/$label.log`"")
    if ($directory) { $arguments += "/DIR=`"$directory`"" }
    $p = Start-Process -FilePath $file -ArgumentList $arguments -PassThru
    Assert ($p.WaitForExit(120000)) "Installer timed out: $label"
    Assert ($p.ExitCode -eq 0) "Installer failed: $label ($($p.ExitCode))"
}
function Uninstall([string]$directory, [string]$label) {
    Invoke-Setup (Join-Path $directory 'unins000.exe') $label
    Assert (!(Test-Path (Join-Path $directory 'elevenclock-rs.exe'))) 'Uninstall left the executable'
    Assert (!(Test-Path $shortcut)) 'Uninstall left the Start menu shortcut'
}
function Wait-App {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $apps = @(Get-Process -Name elevenclock-rs -ErrorAction SilentlyContinue | Where-Object { [PackageWindow]::Settings($_.Id) -ne [IntPtr]::Zero })
        if ($apps.Count -eq 1) { return $apps[0] }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Interactive launch did not open one Settings window'
}
function Exit-App($app) {
    $window = [PackageWindow]::Settings($app.Id)
    Assert ($window -ne [IntPtr]::Zero) 'No Settings window for Exit'
    [PackageWindow]::SendMessage($window, 0x111, [IntPtr]1003, [IntPtr]::Zero) | Out-Null
    Assert ($app.WaitForExit(20000)) 'App did not exit through Settings'
}
try {
    # Verify every downloaded artifact before any installation/launch.
    foreach ($line in Get-Content "$packageRoot/SHA256SUMS.txt") {
        $hash, $relative = $line -split '  ', 2
        Assert ((Get-FileHash -LiteralPath (Join-Path $packageRoot $relative) -Algorithm SHA256).Hash.ToLowerInvariant() -eq $hash) "Checksum mismatch: $relative"
    }
    $results.checksums = 'passed'
    New-Item -Path $run -Force | Out-Null
    New-Item -Path $approved -Force | Out-Null
    New-ItemProperty -Path $run -Name 'elevenClock' -Value 'legacy-sentinel-do-not-launch' -PropertyType String | Out-Null
    Invoke-Setup $installer 'install'
    $exe = Join-Path $install 'elevenclock-rs.exe'
    Assert (Test-Path $exe) 'Default per-user executable missing'
    Assert ($null -eq (Read-Run $value)) 'Installer enabled startup'
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
    Assert ($link.TargetPath -eq $exe -and !$link.Arguments) 'Wrong Start menu target or arguments'
    Start-Process -FilePath $shortcut
    $app = Wait-App
    Exit-App $app
    $results.startMenuLaunch = 'passed: Settings opens and Exit completes'
    # Exercise the existing complete real Settings/Support/startup journey.
    & "$PSScriptRoot/test-rust-settings.ps1" -Executable $exe -OutputDirectory "$evidence/settings"
    $results.settingsSupportJourney = 'passed'
    $installedHash = (Get-FileHash $exe -Algorithm SHA256).Hash
    New-Item -ItemType Directory -Path $private -Force | Out-Null
    $sentinel = Join-Path $private 'packaging-preserve-sentinel.txt'
    Set-Content $sentinel 'keep-private-data'
    $unowned = Join-Path $install 'unowned-sentinel.txt'
    Set-Content $unowned 'keep-unowned-file'
    Invoke-Setup $installer 'reinstall'
    Assert ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $installedHash) 'Reinstall changed the candidate'
    $results.reinstall = 'passed (same candidate; not version-update evidence)'
    # Only the installed executable owns the registration deleted on uninstall.
    Set-ItemProperty $run $value ('"' + $exe + '" --startup')
    New-ItemProperty $approved $value -PropertyType Binary -Value ([byte[]](3,0,0,0,0,0,0,0,0,0,0,0)) -Force | Out-Null
    Uninstall $install 'uninstall-owned'
    Assert ($null -eq (Read-Run $value)) 'Owned startup value survived uninstall'
    Assert ($null -eq (Get-ItemProperty $approved -Name $value -ErrorAction SilentlyContinue)) 'Owned approval survived uninstall'
    Assert ((Get-Content $sentinel) -eq 'keep-private-data') 'Private data removed'
    Assert ((Get-Content $unowned) -eq 'keep-unowned-file') 'Unowned installation file removed'
    Assert ((Read-Run 'elevenClock') -eq 'legacy-sentinel-do-not-launch') 'Legacy startup changed'
    $results.uninstallOwnership = 'passed: private/unowned files and legacy startup preserved'
    # Quoted non-ASCII path plus portable startup ownership and its Windows flag.
    $testSuffix = [char]0x00E9
    $spacedInstall = Join-Path $env:LOCALAPPDATA "Programs/ElevenClock-rs test $testSuffix"
    Invoke-Setup $installer 'install-unicode' $spacedInstall
    $portable = Join-Path $packageRoot "portable smoke $testSuffix"
    Expand-Archive -LiteralPath "$packageRoot/$($package.name)-portable.zip" -DestinationPath $portable
    $portableExe = Join-Path $portable 'elevenclock-rs.exe'
    Set-ItemProperty $run $value ('"' + $portableExe + '" --startup')
    New-ItemProperty $approved $value -PropertyType Binary -Value ([byte[]](3,0,0,0,0,0,0,0,0,0,0,0)) -Force | Out-Null
    Uninstall $spacedInstall 'uninstall-portable-owner'
    Assert ((Read-Run $value) -eq ('"' + $portableExe + '" --startup')) 'Portable startup was removed'
    Assert ($null -ne (Get-ItemProperty $approved -Name $value -ErrorAction SilentlyContinue)) 'Portable approval flag removed'
    Remove-ItemProperty $run $value
    Remove-ItemProperty $approved $value
    Start-Process -FilePath $portableExe
    $app = Wait-App
    Exit-App $app
    $quiet = Start-Process -FilePath $portableExe -ArgumentList '--startup' -PassThru
    Start-Sleep -Seconds 3
    Assert (!$quiet.HasExited -and [PackageWindow]::Settings($quiet.Id) -eq [IntPtr]::Zero) 'Startup launch opened Settings or failed'
    Start-Process -FilePath $portableExe
    $app = Wait-App
    Exit-App $app
    $results.portableAndQuietLaunch = 'passed (marker launch; actual Windows logon not tested)'
    $results.quotedUnicodePath = 'passed'
    Invoke-Setup $installer 'install-malformed-startup'
    Set-ItemProperty $run $value 'unrecognised-command'
    New-ItemProperty $approved $value -PropertyType Binary -Value ([byte[]](3,0,0,0,0,0,0,0,0,0,0,0)) -Force | Out-Null
    Uninstall $install 'uninstall-malformed-startup'
    Assert ((Read-Run $value) -eq 'unrecognised-command') 'Malformed startup value was removed'
    Assert ($null -ne (Get-ItemProperty $approved -Name $value -ErrorAction SilentlyContinue)) 'Unowned approval flag removed'
    Remove-ItemProperty $run $value
    Remove-ItemProperty $approved $value
    $results.malformedStartupOwnership = 'passed'
    if ($PreviousInstaller) {
        Invoke-Setup (Resolve-Path $PreviousInstaller).Path 'previous-install'
        $oldHash = (Get-FileHash $exe -Algorithm SHA256).Hash
        Assert ($oldHash -ne $installedHash) 'Previous installer must contain a different executable'
        Invoke-Setup $installer 'update-from-previous'
        Assert ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $installedHash) 'Update did not install candidate'
        Uninstall $install 'remove-before-rollback'
        Invoke-Setup (Resolve-Path $PreviousInstaller).Path 'rollback'
        Assert ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $oldHash) 'Rollback did not restore earlier executable'
        Start-Process -FilePath $shortcut
        $app = Wait-App
        Exit-App $app
        Uninstall $install 'remove-rollback'
        $results.previousReleaseRollback = 'passed: earlier hash restored and Settings/Exit works'
    }
    $results.status = 'passed automated cases; see unverified clean-machine/account/rollback gates'
} finally {
    # On failure keep installation and logs for diagnosis; delete only test-owned
    # startup values so this disposable account cannot log into a stale command.
    foreach ($name in @($value, 'elevenClock')) {
        Remove-ItemProperty -LiteralPath $run -Name $name -ErrorAction SilentlyContinue
    }
    Remove-ItemProperty -LiteralPath $approved -Name $value -ErrorAction SilentlyContinue
    $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$evidence/results.json" -Encoding UTF8
}
