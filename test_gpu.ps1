$nvidiaPath = "nvidia-smi"

function Parse-Float($value, $default) {
    if ($null -eq $value) { return [float]$default }
    $v = $value.ToString().Trim()
    if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]' -or $v -eq '[Not Supported]' -or $v -eq 'Not Supported') { return [float]$default }
    $out = 0.0
    if ([double]::TryParse($v, [ref]$out)) { return [float]$out }
    return [float]$default
}

function Parse-UInt64($value, $default) {
    if ($null -eq $value) { return [uint64]$default }
    $v = $value.ToString().Trim()
    if ($v -eq '' -or $v -eq 'N/A' -or $v -eq '[N/A]' -or $v -eq '[Not Supported]' -or $v -eq 'Not Supported') { return [uint64]$default }
    $out = 0.0
    if ([double]::TryParse($v, [ref]$out)) { return [uint64]$out }
    return [uint64]$default
}

# Get standard nvidia-smi output for power draw fallback
$standardOutput = & $nvidiaPath
$cudaVersion = "N/A"

if ($standardOutput) {
    # Extract CUDA version
    $line = $standardOutput | Where-Object { $_ -match 'CUDA Version' } | Select-Object -First 1
    if ($line -match 'CUDA Version:\s*([0-9\.]+)') {
        $cudaVersion = $Matches[1]
    }
}

# Query all GPUs and parse power from standard output if needed
$raw = & $nvidiaPath --query-gpu=index,name,temperature.gpu,utilization.gpu,utilization.memory,memory.used,memory.total,power.draw,power.limit,fan.speed,clocks.current.graphics,clocks.current.memory,driver_version --format=csv,noheader,nounits
$lines = $raw -split "`n" | Where-Object { $_ -match '\S' }

$rows = foreach ($line in $lines) {
    $parts = $line.Split(',') | ForEach-Object { $_.Trim() }
    if ($parts.Count -lt 13) { continue }

    $gpuIndex = Parse-UInt64 $parts[0] 0
    $powerDraw = Parse-Float $parts[7] 0.0
    $powerLimit = Parse-Float $parts[8] 0.0

    Write-Host "Before fallback: GPU=$gpuIndex PowerDraw=$powerDraw PowerLimit=$powerLimit"

    # If power values are 0 or N/A, try extracting from standard output
    if ($powerDraw -eq 0.0 -or $powerLimit -eq 0.0) {
        # Find the line with GPU index
        $gpuPattern = "^\|\s*$gpuIndex\s+"
        $gpuLineIndex = -1
        for ($i = 0; $i -lt $standardOutput.Count; $i++) {
            if ($standardOutput[$i] -match $gpuPattern) {
                $gpuLineIndex = $i
                break
            }
        }

        # Power info is in the next line
        if ($gpuLineIndex -ge 0 -and ($gpuLineIndex + 1) -lt $standardOutput.Count) {
            $powerLine = $standardOutput[$gpuLineIndex + 1]
            Write-Host "Power Line: $powerLine"
            if ($powerLine -match '(\d+(?:\.\d+)?)W\s*/\s*(\d+(?:\.\d+)?)W') {
                if ($powerDraw -eq 0.0) { $powerDraw = [float]$Matches[1] }
                if ($powerLimit -eq 0.0) { $powerLimit = [float]$Matches[2] }
                Write-Host "After fallback: PowerDraw=$powerDraw PowerLimit=$powerLimit"
            }
        }
    }

    [PSCustomObject]@{
        Name = $parts[1]
        GpuIndex = [uint32]$gpuIndex
        Temperature = Parse-Float $parts[2] 0.0
        UtilizationGpu = Parse-Float $parts[3] 0.0
        UtilizationMemory = Parse-Float $parts[4] 0.0
        MemoryUsed = (Parse-UInt64 $parts[5] 0) * 1MB
        MemoryTotal = (Parse-UInt64 $parts[6] 0) * 1MB
        PowerDraw = $powerDraw
        PowerLimit = $powerLimit
        FanSpeed = Parse-Float $parts[9] -1.0
        ClockGraphics = [uint32](Parse-UInt64 $parts[10] 0)
        ClockMemory = [uint32](Parse-UInt64 $parts[11] 0)
        DriverVersion = $parts[12]
        CudaVersion = $cudaVersion
    }
}

$best = $rows | Sort-Object -Property MemoryTotal -Descending | Select-Object -First 1
$best | ConvertTo-Json
