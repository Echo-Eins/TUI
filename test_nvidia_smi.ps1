# Diagnostic script to check nvidia-smi output
Write-Host "=== Test 1: Query GPU data ===" -ForegroundColor Cyan
$raw = nvidia-smi --query-gpu=name,pci.bus_id,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version --format=csv,noheader,nounits
Write-Host "Raw output:"
$raw
Write-Host ""

Write-Host "=== Test 2: Parse CSV ===" -ForegroundColor Cyan
$parts = $raw.Split(',') | ForEach-Object { $_.Trim() }
Write-Host "Total parts: $($parts.Count)"
for ($i = 0; $i -lt $parts.Count; $i++) {
    Write-Host "[$i] = '$($parts[$i])'"
}
Write-Host ""

Write-Host "=== Test 3: Standard nvidia-smi output ===" -ForegroundColor Cyan
nvidia-smi
Write-Host ""

Write-Host "=== Test 4: GPU Process Memory (WMI) ===" -ForegroundColor Cyan
$engine = Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -ErrorAction SilentlyContinue
if ($engine) {
    Write-Host "Found $($engine.Count) GPU engine entries"
    $engine | Select-Object -First 5 | Format-Table Name, UtilizationPercentage
} else {
    Write-Host "No GPU engine data found via WMI"
}
