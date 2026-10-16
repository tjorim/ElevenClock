# Development-only packaging driver. Requires native Windows MSVC + SDK, Python
# 3.11+, and Inno Setup 6. It never publishes or installs prerequisites.
param(
    [string]$OutputDirectory = 'dist/candidate',
    [string]$Python = 'python',
    [string]$Iscc = '',
    [ValidateSet('Development', 'Distribution')][string]$Mode = 'Development'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$oldFlags = $env:RUSTFLAGS
try {
    if ($env:OS -ne 'Windows_NT') { throw 'A native Windows build host is required' }
    if (!$Iscc) {
        $Iscc = @("$env:LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe", 'C:/Program Files (x86)/Inno Setup 6/ISCC.exe') |
            Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
    if (!$Iscc -or !(Test-Path -LiteralPath $Iscc)) { throw 'Inno Setup 6 compiler is required; no automatic prerequisite installation' }
    if (Test-Path -LiteralPath $OutputDirectory) { throw 'Choose a new output directory to avoid stale artifacts' }
    if ($Mode -eq 'Distribution') {
        & $Python scripts/rust-release.py check-gates
        if ($LASTEXITCODE -ne 0) { throw 'Distribution gates are not passed' }
    }
    # Keep the toolchain choice out of the app and source developer defaults.
    # A target-qualified build leaves host proc macros dynamically linked.
    if ($env:CARGO_ENCODED_RUSTFLAGS) { throw 'CARGO_ENCODED_RUSTFLAGS must be unset' }
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    & cargo build --manifest-path rust/Cargo.toml -p clock-app --release --target x86_64-pc-windows-msvc --target-dir rust/target/packaging --locked
    if ($LASTEXITCODE -ne 0) { throw 'Locked release build failed' }
    $exe = 'rust/target/packaging/x86_64-pc-windows-msvc/release/elevenclock-rs.exe'
    if ((Get-AuthenticodeSignature -LiteralPath $exe).Status -ne 'NotSigned') { throw 'Candidate signing status differs from the unsigned package notice' }
    $sdk = Get-ChildItem 'C:/Program Files (x86)/Windows Kits/10/bin/*/x64/mt.exe' | Sort-Object FullName -Descending | Select-Object -First 1
    if (!$sdk) { throw 'Windows SDK manifest tool unavailable' }
    $manifest = Join-Path $root 'rust/target/packaging/verified-app.manifest'
    & $sdk.FullName "-inputresource:$exe;#1" "-out:$manifest"
    if ($LASTEXITCODE -ne 0) { throw 'Missing embedded manifest' }
    [xml]$xml = Get-Content -LiteralPath $manifest -Raw
    $level = $xml.SelectSingleNode("//*[local-name()='requestedExecutionLevel']")
    if (!$level -or $level.level -ne 'asInvoker' -or $level.uiAccess -ne 'false') { throw 'Expected unelevated asInvoker manifest' }
    & $Python scripts/rust-release.py prepare --output $OutputDirectory --mode $Mode
    if ($LASTEXITCODE -ne 0) { throw 'Release assembly failed' }
    $output = (Resolve-Path -LiteralPath $OutputDirectory).Path
    $package = Get-Content -LiteralPath "$output/package.json" -Raw | ConvertFrom-Json
    & $Iscc "/DPayloadDir=$output/payload" "/DPackageVersion=$($package.version)" "/DArtifactName=$($package.name)" "/DArtifactDir=$output" packaging/windows/ElevenClock-rs.iss
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    if ((Get-AuthenticodeSignature -LiteralPath "$output/$($package.name)-setup.exe").Status -ne 'NotSigned') { throw 'Installer signing status differs from the unsigned notice' }
    & $Python scripts/rust-release.py finish --output $output
    if ($LASTEXITCODE -ne 0) { throw 'Artifact checksum generation failed' }
    & $Python scripts/rust-release.py verify --output $output
    if ($LASTEXITCODE -ne 0) { throw 'Artifact integrity verification failed' }
} finally {
    $env:RUSTFLAGS = $oldFlags
    Pop-Location
}
