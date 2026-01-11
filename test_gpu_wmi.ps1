# Diagnostic script to check WMI GPU process data
Write-Host "=== Test 1: GPU Process Memory ===" -ForegroundColor Cyan
$items = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -ErrorAction SilentlyContinue
if ($items) {
    Write-Host "Found $($items.Count) GPU process memory entries"
    $items | Select-Object -First 10 | Format-Table Name, DedicatedUsage, SharedUsage
} else {
    Write-Host "No GPU process memory data found"
}
Write-Host ""

Write-Host "=== Test 2: GPU Engine Utilization ===" -ForegroundColor Cyan
$engine = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue
if ($engine) {
    Write-Host "Found $($engine.Count) GPU engine entries"
    $engine | Where-Object { $_.UtilizationPercentage -gt 0 } | Select-Object -First 10 | Format-Table Name, UtilizationPercentage
} else {
    Write-Host "No GPU engine data found"
}
Write-Host ""

Write-Host "=== Test 3: Parse GPU Process with Usage ===" -ForegroundColor Cyan
$byPid = @{}
foreach ($item in $items) {
    if ($item.Name -match '^pid_(\d+)_') {
        $pid = [int]$matches[1]
        if (-not $byPid.ContainsKey($pid)) {
            $byPid[$pid] = [uint64]0
        }
        $byPid[$pid] += [uint64]$item.DedicatedUsage
    }
}

$gpuByPid = @{}
$typeByPid = @{}
if ($engine) {
    foreach ($item in $engine) {
        if ($item.Name -match '^pid_(\d+)_') {
            $pid = [int]$matches[1]
            $util = [float]$item.UtilizationPercentage
            if (-not $gpuByPid.ContainsKey($pid)) { $gpuByPid[$pid] = 0.0 }
            $gpuByPid[$pid] += $util

            $etype = "Unknown"
            if ($item.Name -match 'engtype_3D' -or $item.Name -match 'engtype_Graphics') {
                $etype = "Graphics"
            } elseif ($item.Name -match 'engtype_Compute') {
                $etype = "Compute"
            } elseif ($item.Name -match 'engtype_Copy') {
                $etype = "Copy"
            }
            $typeByPid[$pid] = $etype
        }
    }
}

Write-Host "PIDs with GPU usage:"
$gpuByPid.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 10 | ForEach-Object {
    $pid = $_.Key
    $usage = $_.Value
    $vram = if ($byPid.ContainsKey($pid)) { [math]::Round($byPid[$pid] / 1MB, 2) } else { 0 }
    $type = if ($typeByPid.ContainsKey($pid)) { $typeByPid[$pid] } else { "Unknown" }
    $proc = Get-Process -Id $pid -ErrorAction SilentlyContinue
    $name = if ($proc) { $proc.ProcessName } else { "Unknown" }
    Write-Host "  PID $pid ($name): GPU=$($usage)%, VRAM=$($vram)MB, Type=$type"
}
