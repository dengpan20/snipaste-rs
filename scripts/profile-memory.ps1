param(
    [string]$Executable = (Join-Path $PSScriptRoot '../target/x86_64-pc-windows-gnu/release/snipaste-rs.exe'),
    [string]$ReportDirectory = (Join-Path $PSScriptRoot '../target/memory-after')
)

$ErrorActionPreference = 'Stop'
$profileExe = (Resolve-Path -LiteralPath $Executable).Path
New-Item -ItemType Directory -Path $ReportDirectory -Force | Out-Null
$profileDir = (Resolve-Path -LiteralPath $ReportDirectory).Path
$profileVars = @('SNIPASTE_MEMORY_PROFILE_DIR', 'SNIPASTE_SMOKE_MS', 'SNIPASTE_VERIFY_DIR', 'RUST_LOG')
$profileSaved = @{}
foreach ($key in $profileVars) { $profileSaved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process') }
try {
    $env:SNIPASTE_MEMORY_PROFILE_DIR = $profileDir
    $env:RUST_LOG = 'info'
    Remove-Item Env:SNIPASTE_SMOKE_MS -ErrorAction SilentlyContinue
    Remove-Item Env:SNIPASTE_VERIFY_DIR -ErrorAction SilentlyContinue
    $profileStart = Get-Date
    $profileProcess = Start-Process -FilePath $profileExe -WindowStyle Hidden -Wait -PassThru `
        -RedirectStandardError (Join-Path $profileDir 'run.log') `
        -RedirectStandardOutput (Join-Path $profileDir 'stdout.log')
    if ($profileProcess.ExitCode -ne 0) { throw "内存验收进程失败：$($profileProcess.ExitCode)" }
    if ((Get-Item -LiteralPath (Join-Path $profileDir 'memory.csv')).LastWriteTime -lt $profileStart) { throw '未生成本次内存验收报告' }
    $profileRows = @(Import-Csv -LiteralPath (Join-Path $profileDir 'memory.csv'))
    if ($profileRows.Count -ne 13 -or $profileRows[-1].stage -ne 'idle-3') { throw '内存验收未完成，请查看 run.log' }
    $profileRows | Select-Object stage, `
        @{ Name = '工作集MiB'; Expression = { [Math]::Round([double]$_.resident_bytes / 1MB, 1) } }, `
        @{ Name = '私有提交MiB'; Expression = { [Math]::Round([double]$_.private_bytes / 1MB, 1) } }, `
        capture_ms, gpu, overlay | Format-Table -AutoSize
} finally {
    foreach ($key in $profileVars) { [Environment]::SetEnvironmentVariable($key, $profileSaved[$key], 'Process') }
}
