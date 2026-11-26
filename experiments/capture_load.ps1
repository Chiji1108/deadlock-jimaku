# Deadlock のウィンドウを取り込み続けたときの負荷を測る（DESIGN.md 18.6 節）。
# 測り方ごとに `capture_load` の例を動かし、その間のゲーム、OBS、画面の合成（dwm）の CPU と GPU を出す。
#
# usage: powershell -File experiments/capture_load.ps1 [-Seconds 30] [-Modes none,idle,full,region] [-Out <フォルダ>] [-Rate 3]
#   先に cargo build -p overlays-core --release --example capture_load をしておく
#   CPU は 1 コアに対する割合、GPU はプロセスが使っているエンジンの使用率の合計（どちらも測っている間の平均）
#
# ゲームの FPS は、PresentMon（winget install Intel.PresentMon.Console。管理者で動かす）を先に動かして測る:
#   presentmon --process_name deadlock.exe --output_file <CSV> --timed <秒数> --terminate_after_timed --no_console_stats --date_time
#   CSV の MsBetweenPresents を、下に出る時刻で測り方ごとに分ける（CSV の時刻は時差がずれて出ることがある）
param(
    [int]$Seconds = 30,
    [string[]]$Modes = @('none', 'idle', 'full', 'region'),
    [string]$Out = "$PSScriptRoot\results\capture_load",
    [int]$Rate = 3
)

$exe = "$PSScriptRoot\..\target\release\examples\capture_load.exe"
if (-not (Test-Path $exe)) { throw "先に capture_load の例をビルドする: $exe" }
New-Item -ItemType Directory -Force $Out | Out-Null
$watched = @('deadlock', 'obs64', 'dwm')

function Get-CpuTimes {
    $times = @{}
    foreach ($name in $watched) {
        foreach ($found in Get-Process $name -ErrorAction SilentlyContinue) {
            $times[$name] += $found.TotalProcessorTime.TotalSeconds
        }
    }
    $times
}

foreach ($mode in $Modes) {
    $ids = @{}
    foreach ($name in $watched) {
        Get-Process $name -ErrorAction SilentlyContinue | ForEach-Object { $ids["$($_.Id)"] = $name }
    }
    $before = Get-CpuTimes
    $started = Get-Date
    $log = "$Out\$mode.txt"
    $run = Start-Process $exe -ArgumentList $mode, $Seconds, "`"$Out`"", $Rate -NoNewWindow -PassThru -RedirectStandardOutput $log
    $ids["$($run.Id)"] = 'capture_load'

    # GPU の使用率を、プロセスごとに足す
    $gpu = @{}
    $samples = 0
    while (-not $run.HasExited -and ((Get-Date) - $started).TotalSeconds -lt $Seconds) {
        $counters = (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction SilentlyContinue).CounterSamples
        $samples += 1
        foreach ($counter in $counters) {
            if ($counter.InstanceName -match '^pid_(\d+)_' -and $ids.ContainsKey($Matches[1])) {
                $gpu[$ids[$Matches[1]]] += $counter.CookedValue
            }
        }
    }
    $after = Get-CpuTimes
    $elapsed = ((Get-Date) - $started).TotalSeconds
    $run.WaitForExit()

    # 始めと終わりの時刻は、外で測った FPS（PresentMon の CSV）を測り方ごとに分けるのに使う
    '{0:HH:mm:ss.fff}〜{1:HH:mm:ss.fff}' -f $started, $started.AddSeconds($elapsed)
    Get-Content $log -Encoding UTF8
    foreach ($name in $watched + 'capture_load') {
        $cpu = if ($before.ContainsKey($name) -and $after.ContainsKey($name)) {
            '{0,6:N1}%' -f (($after[$name] - $before[$name]) / $elapsed * 100)
        } else { '      -' }
        $used = if ($samples -gt 0 -and $gpu.ContainsKey($name)) { '{0,5:N1}%' -f ($gpu[$name] / $samples) } else { '    -' }
        if ($before.ContainsKey($name) -or $gpu.ContainsKey($name)) {
            '  {0,-12} CPU {1}　GPU {2}' -f $name, $cpu, $used
        }
    }
}
