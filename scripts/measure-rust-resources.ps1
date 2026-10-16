# Development-only resource sampler for R13; never part of the installed app.
#
# Samples every elevenclock-rs process (the app and any helper) at a fixed interval and
# reports, per the acceptance contract: total CPU as CPU-time delta / (wall time x logical
# processors), private working set, handles and threads. Timer/discovery rates are not
# observable from outside the process; record them from the R11 in-app diagnostics instead.
# It only reads public process counters and never starts, stops or controls the app.
param(
    [int]$DurationSeconds = 600,
    [int]$IntervalSeconds = 10,
    [string]$Label = 'unlabelled',
    [string]$OutputDirectory = 'native-evidence/resources'
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$logical = [Environment]::ProcessorCount
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$csv = Join-Path $OutputDirectory "$stamp-$Label.csv"

function Snapshot {
    $processes = @(Get-Process -Name 'elevenclock-rs*' -ErrorAction SilentlyContinue)
    $cpu = 0.0; $private = 0L; $handles = 0; $threads = 0
    foreach ($process in $processes) {
        $cpu += $process.TotalProcessorTime.TotalSeconds
        $handles += $process.HandleCount
        $threads += $process.Threads.Count
        # Private working set (the contract's metric), not private bytes/commit.
        $counter = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess = $($process.Id)"
        $private += [int64]$counter.WorkingSetPrivate
    }
    [pscustomobject]@{
        Time = [DateTime]::UtcNow; Processes = $processes.Count; Cpu = $cpu
        PrivateBytes = $private; Handles = $handles; Threads = $threads
    }
}

$first = Snapshot
if ($first.Processes -eq 0) { throw 'No elevenclock-rs process is running; start the app first.' }
$rows = @()
$previous = $first
$deadline = $first.Time.AddSeconds($DurationSeconds)
while ($previous.Time -lt $deadline) {
    Start-Sleep -Seconds $IntervalSeconds
    $now = Snapshot
    if ($now.Processes -eq 0) { throw 'The app exited during measurement.' }
    $wall = ($now.Time - $previous.Time).TotalSeconds
    $rows += [pscustomobject]@{
        UtcTime = $now.Time.ToString('o'); Processes = $now.Processes
        CpuPercentOfMachine = [math]::Round(100 * ($now.Cpu - $previous.Cpu) / ($wall * $logical), 4)
        PrivateMiB = [math]::Round($now.PrivateBytes / 1MB, 2)
        Handles = $now.Handles; Threads = $now.Threads
    }
    $previous = $now
}
$rows | Export-Csv -Path $csv -NoTypeInformation

$wallTotal = ($previous.Time - $first.Time).TotalSeconds
$summary = [ordered]@{
    label = $Label; csv = $csv; logicalProcessors = $logical
    wallSeconds = [math]::Round($wallTotal, 1); processesSeen = $first.Processes
    cpuPercentOfMachine = [math]::Round(100 * ($previous.Cpu - $first.Cpu) / ($wallTotal * $logical), 4)
    privateMiBStart = [math]::Round($first.PrivateBytes / 1MB, 2)
    privateMiBEnd = [math]::Round($previous.PrivateBytes / 1MB, 2)
    privateMiBMax = ($rows | Measure-Object PrivateMiB -Maximum).Maximum
    handlesStart = $first.Handles; handlesEnd = $previous.Handles
    threadsStart = $first.Threads; threadsEnd = $previous.Threads
    targets = 'CPU < 0.2% of machine; private working set < 40 MiB'
}
$summary | ConvertTo-Json | Tee-Object -FilePath (Join-Path $OutputDirectory "$stamp-$Label.json")
