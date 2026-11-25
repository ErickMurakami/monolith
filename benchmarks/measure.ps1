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
$samplesTaken = for ($i = 0; $i -lt $Samples; $i++) {
    $memory = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory
    $processor = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'"
    $app = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "Name='monolith' OR Name LIKE 'monolith#%'")
    if ($Scenario -eq 'baseline' -and $app.Count) { throw 'Close Monolith before collecting the baseline.' }
    if ($Scenario -ne 'baseline' -and !$app.Count) { throw 'Start Monolith before collecting this scenario.' }
    [ordered]@{
        timestamp = [DateTimeOffset]::UtcNow.ToString('o')
        available_mib = $memory.AvailableMBytes
        system_commit_bytes = $memory.CommittedBytes
        cpu_percent = $processor.PercentProcessorTime
        monolith = @($app | ForEach-Object {
            [ordered]@{
                pid = $_.IDProcess
                working_set_bytes = $_.WorkingSet
                private_working_set_bytes = $_.WorkingSetPrivate
                private_bytes = $_.PrivateBytes
                cpu_percent_one_core = $_.PercentProcessorTime
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
