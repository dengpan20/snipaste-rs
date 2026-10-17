param(
    [string]$Target = $(if ($env:CARGO_BUILD_TARGET) { $env:CARGO_BUILD_TARGET } else { 'x86_64-pc-windows-msvc' }),
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    if ($Target -notin @('x86_64-pc-windows-msvc', 'x86_64-pc-windows-gnu')) {
        throw 'Windows 发布包只支持 x64 MSVC / GNU 目标。'
    }
    $manifest = Get-Content -Raw -LiteralPath 'Cargo.toml'
    $releaseVersion = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $releaseVersion) { throw '无法读取版本号。' }
    $sourceCommit = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw '无法读取 Git 提交。' }
    $changes = git status --porcelain
    if ($changes) { throw '请先提交源代码，再生成带有准确提交信息的发布包。' }
    if (-not $SkipBuild) {
        cargo build --locked --release -p snipaste-app --target $Target
        if ($LASTEXITCODE -ne 0) { throw 'release 构建失败。' }
    }
    $executable = Join-Path $projectRoot "target/$Target/release/snipaste-rs.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw '找不到 release 程序。' }
    $stage = Join-Path $projectRoot ('target/package-windows/' + [guid]::NewGuid().ToString('N'))
    $output = Join-Path $projectRoot "dist/release-$releaseVersion"
    New-Item -ItemType Directory -Force -Path $stage, $output | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $stage 'snipaste-rs.exe')
    $utf8 = New-Object System.Text.UTF8Encoding $false
    $instructions = (Get-Content -Raw -Encoding UTF8 -LiteralPath 'packaging/windows/使用说明.txt').Replace('@VERSION@', $releaseVersion)
    [IO.File]::WriteAllText((Join-Path $stage '使用说明.txt'), $instructions, $utf8)
    Copy-Item -LiteralPath "docs/releases/$releaseVersion.md" -Destination (Join-Path $stage 'RELEASE-NOTES.md')
    $metadata = [ordered]@{
        name = '映贴'; version = $releaseVersion; target = $Target
        source_commit = $sourceCommit.Trim(); built_at_utc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
        build_profile = 'release'; code_signature = 'unsigned'
    } | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $stage 'release.json'), $metadata + "`n", $utf8)
    $checksums = foreach ($name in @('snipaste-rs.exe', '使用说明.txt', 'RELEASE-NOTES.md', 'release.json')) {
        (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage $name)).Hash.ToLowerInvariant() + '  ' + $name
    }
    [IO.File]::WriteAllText((Join-Path $stage 'SHA256SUMS.txt'), ($checksums -join "`n") + "`n", $utf8)
    $archive = Join-Path $output "yingtie-$releaseVersion-win64.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -Force
    Copy-Item -LiteralPath $executable -Destination (Join-Path $output "yingtie-$releaseVersion.exe") -Force
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        if ($zip.Entries.Count -ne 5) { throw '压缩包内容数量不符。' }
        foreach ($entry in $zip.Entries) {
            if ($entry.Length -eq 0) { throw "压缩包中存在空文件：$($entry.FullName)" }
        }
    } finally { $zip.Dispose() }
    Get-Item -LiteralPath $archive, (Join-Path $output "yingtie-$releaseVersion.exe") |
        Select-Object FullName, Length
} finally { Pop-Location }
