param(
    [ValidateRange(2, 120)][int]$Samples = 6,
    [ValidateRange(1, 30)][int]$IntervalSeconds = 2,
    [ValidateSet('baseline', 'launcher', 'resident', 'dock')][string]$Scenario = 'baseline',
    [string]$Output = 'artifacts/benchmark.json'
)

$ErrorActionPreference = 'Stop'
$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor
$windows = Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Windows NT/CurrentVersion'
$gpuClass = 'Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine'

function Get-GpuUsage {
    try {
        $contexts = @(Get-CimInstance -ClassName $gpuClass -Property Name,UtilizationPercentage)
    } catch [Microsoft.Management.Infrastructure.CimException] {
        return @{ system = $null; processes = $null }
    }
    $engines = @{}
    $engineProcesses = @{}
    foreach ($context in $contexts) {
        if ($context.Name -match '^pid_(\d+)_(luid_.+?_phys_\d+_eng_\d+)_engtype_') {
            $process = [uint32]$Matches[1]
            $engine = $Matches[2]
            $load = [double]$context.UtilizationPercentage
            $engines[$engine] += $load
            if (!$engineProcesses.ContainsKey($process)) { $engineProcesses[$process] = @{} }
            $engineProcesses[$process][$engine] += $load
        }
    }
    if (!$engines.Count) { return @{ system = $null; processes = $null } }
    $processes = @{}
    foreach ($process in $engineProcesses.Keys) {
        $processes[$process] = [Math]::Min(100, ($engineProcesses[$process].Values | Measure-Object -Maximum).Maximum)
    }
    return @{
        system = [Math]::Min(100, ($engines.Values | Measure-Object -Maximum).Maximum)
        processes = $processes
    }
}

$samplesTaken = for ($i = 0; $i -lt $Samples; $i++) {
    $memory = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory
    $processor = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'"
    $app = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "Name='monolith' OR Name LIKE 'monolith#%'")
    if ($Scenario -eq 'baseline' -and $app.Count) { throw 'Close Monolith before collecting the baseline.' }
    if ($Scenario -ne 'baseline' -and !$app.Count) { throw 'Start Monolith before collecting this scenario.' }
    $gpu = Get-GpuUsage
    [ordered]@{
        timestamp = [DateTimeOffset]::UtcNow.ToString('o')
        available_mib = $memory.AvailableMBytes
        system_commit_bytes = $memory.CommittedBytes
        cpu_percent = $processor.PercentProcessorTime
        gpu_busiest_engine_percent = $gpu.system
        monolith = @($app | ForEach-Object {
            [ordered]@{
                pid = $_.IDProcess
                working_set_bytes = $_.WorkingSet
                private_working_set_bytes = $_.WorkingSetPrivate
                private_bytes = $_.PrivateBytes
                cpu_percent_one_core = $_.PercentProcessorTime
                gpu_busiest_engine_percent = if ($null -ne $gpu.processes) {
                    [double]$gpu.processes[[uint32]$_.IDProcess]
                } else {
                    $null
                }
                io_bytes_per_second = $_.IODataBytesPersec
            }
        })
    }
    if ($i -lt ($Samples - 1)) { Start-Sleep -Seconds $IntervalSeconds }
}
$result = [ordered]@{
    scenario = $Scenario
    windows = "$($windows.DisplayVersion) $($os.BuildNumber).$($windows.UBR)"
    cpu = @($cpu.Name)
    logical_processors = ($cpu.NumberOfLogicalProcessors | Measure-Object -Sum).Sum
    physical_memory_mib = [math]::Round($os.TotalVisibleMemorySize / 1024)
    revision = (git rev-parse --verify --quiet HEAD)
    interval_seconds = $IntervalSeconds
    samples = @($samplesTaken)
}
$parent = Split-Path -Parent $Output
if ($parent) { New-Item -ItemType Directory -Force $parent | Out-Null }
$result | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 $Output
