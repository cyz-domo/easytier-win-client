# Monitor real-time CPU and Memory usage of EasyTier GUI & WebView2 processes.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\monitor-gui-cpu.ps1 [-IntervalSec 1]

param(
    [int]$IntervalSec = 1
)

$Host.UI.RawUI.WindowTitle = "EasyTier GUI & WebView2 资源监控"
$numCores = [System.Environment]::ProcessorCount

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " EasyTier GUI & WebView2 实时 CPU/内存监控工具" -ForegroundColor Cyan
Write-Host " 逻辑核心数: $numCores | 采样间隔: ${IntervalSec}s | 按 Ctrl+C 退出" -ForegroundColor DarkGray
Write-Host "==========================================================" -ForegroundColor Cyan

function Get-ProcessClassification($proc) {
    try {
        $cmd = (Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)" -ErrorAction SilentlyContinue).CommandLine
        if ($proc.ProcessName -eq "easytier-win-client") {
            return "EasyTier GUI 主进程"
        }
        if ($proc.ProcessName -eq "easytier-core") {
            return "EasyTier Core 内核"
        }
        if ($proc.ProcessName -eq "msedgewebview2") {
            if ($cmd -match "--type=gpu-process") { return "WebView2 GPU进程" }
            if ($cmd -match "--type=renderer") { return "WebView2 渲染进程" }
            if ($cmd -match "--type=utility") { return "WebView2 工具进程" }
            if ($cmd -match "--type=crashpad-handler") { return "WebView2 Crashpad" }
            return "WebView2 浏览器主控"
        }
    } catch {
        # ignore
    }
    return $proc.ProcessName
}

$previousSamples = @{}

while ($true) {
    # Find all related processes
    $targetProcesses = Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $_.ProcessName -in @("easytier-win-client", "msedgewebview2", "easytier-core")
    }

    # Filter msedgewebview2 to only those belonging to easytier-win-client if GUI is running
    $guiProc = $targetProcesses | Where-Object { $_.ProcessName -eq "easytier-win-client" }
    if ($guiProc) {
        $guiPid = $guiProc.Id
        $childPids = (Get-CimInstance Win32_Process -Filter "ParentProcessId = $guiPid" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty ProcessId)
        # Also grab grandchildren (renderer / gpu spawned by webview2 browser process)
        $allDescendantPids = [System.Collections.Generic.HashSet[int]]::new()
        foreach ($cp in $childPids) {
            [void]$allDescendantPids.Add($cp)
            $grand = (Get-CimInstance Win32_Process -Filter "ParentProcessId = $cp" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty ProcessId)
            foreach ($gp in $grand) { [void]$allDescendantPids.Add($gp) }
        }

        $relevantProcesses = $targetProcesses | Where-Object {
            $_.Id -eq $guiPid -or $allDescendantPids.Contains($_.Id)
        }
    } else {
        $relevantProcesses = $targetProcesses
    }

    $now = [DateTime]::UtcNow
    $currentSamples = @{}
    $results = @()
    $totalCpu = 0.0
    $totalMemMb = 0.0

    foreach ($p in $relevantProcesses) {
        try {
            $pId = $p.Id
            $cpuTime = $p.TotalProcessorTime.TotalMilliseconds
            $memMb = [Math]::Round($p.WorkingSet64 / 1MB, 1)

            $currentSamples[$pId] = @{
                Time = $now
                CpuTime = $cpuTime
            }

            $cpuPercent = 0.0
            if ($previousSamples.ContainsKey($pId)) {
                $prev = $previousSamples[$pId]
                $timeDeltaMs = ($now - $prev.Time).TotalMilliseconds
                $cpuDeltaMs = $cpuTime - $prev.CpuTime
                if ($timeDeltaMs -gt 0) {
                    $cpuPercent = [Math]::Round(($cpuDeltaMs / ($timeDeltaMs * $numCores)) * 100, 2)
                    if ($cpuPercent -lt 0) { $cpuPercent = 0.0 }
                }
            }

            $role = Get-ProcessClassification $p
            $results += [PSCustomObject]@{
                PID = $pId
                Role = $role
                ProcessName = $p.ProcessName
                CPU_Pct = $cpuPercent
                Mem_MB = $memMb
            }
            $totalCpu += $cpuPercent
            $totalMemMb += $memMb
        } catch {
            # Process may have exited
        }
    }

    $previousSamples = $currentSamples

    # Clear and render
    Clear-Host
    $ts = Get-Date -Format "HH:mm:ss"
    Write-Host "[$ts] EasyTier 进程资源监控 (每 ${IntervalSec}s 刷新 | 逻辑核心: $numCores)" -ForegroundColor Cyan
    Write-Host ("-" * 72)

    if ($results.Count -eq 0) {
        Write-Host "未检测到正在运行的 easytier-win-client / WebView2 / easytier-core 进程。" -ForegroundColor Yellow
    } else {
        # Format table header
        Write-Host ("{0,-8} {1,-24} {2,-12} {3,8} {4,10}" -f "PID", "角色类型", "进程名", "CPU %", "内存(MB)") -ForegroundColor White
        Write-Host ("{0,-8} {1,-24} {2,-12} {3,8} {4,10}" -f "----", "--------", "------", "-----", "--------") -ForegroundColor DarkGray

        foreach ($row in ($results | Sort-Object -Property CPU_Pct -Descending)) {
            $cpuStr = "{0:N1}%" -f $row.CPU_Pct
            $memStr = "{0:N1} MB" -f $row.Mem_MB

            $color = "Green"
            if ($row.CPU_Pct -ge 10.0) { $color = "Red" }
            elseif ($row.CPU_Pct -ge 3.0) { $color = "Yellow" }

            Write-Host ("{0,-8} {1,-24} {2,-12} " -f $row.PID, $row.Role, $row.ProcessName) -NoNewline
            Write-Host ("{0,8} " -f $cpuStr) -ForegroundColor $color -NoNewline
            Write-Host ("{0,10}" -f $memStr)
        }

        Write-Host ("-" * 72)
        $totCpuStr = "{0:N1}%" -f $totalCpu
        $totMemStr = "{0:N1} MB" -f $totalMemMb
        Write-Host ("{0,-8} {1,-24} {2,-12} {3,8} {4,10}" -f "", "总计 (EasyTier 进程树)", "", $totCpuStr, $totMemStr) -ForegroundColor Cyan
    }

    Start-Sleep -Seconds $IntervalSec
}
