param(
    [string]$ReportPath = (Join-Path $PSScriptRoot '../target/windows-console-host.txt')
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$ReportPath = [System.IO.Path]::GetFullPath($ReportPath)
'RUNNING' | Set-Content -Encoding utf8 $ReportPath
$artifacts = & cargo test --lib --no-run --message-format=json
if ($LASTEXITCODE -ne 0) { throw 'Could not build the native console tests.' }
$binary = $artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.profile.test -and $_.executable } |
    Select-Object -First 1 -ExpandProperty executable
if (-not $binary) { throw 'No library test executable found.' }
$env:TERRA_TERM_CONSOLE_TEST = 'attached'
$env:TERRA_TERM_CONSOLE_REPORT = $ReportPath
[ordered]@{
    Date = (Get-Date -Format o)
    Windows = [Environment]::OSVersion.VersionString
    Architecture = $env:PROCESSOR_ARCHITECTURE
    Shell = "PowerShell $($PSVersionTable.PSVersion)"
    WindowsTerminalSession = $env:WT_SESSION
    TestExecutable = $binary
} | ConvertTo-Json | Set-Content -Encoding utf8 "$ReportPath.environment.json"
& $binary --exact platform::windows::tests::native_console --nocapture 2> "$ReportPath.stderr.txt"
$testExit = $LASTEXITCODE
if ($testExit -ne 0) { throw "Native console checks failed; see $ReportPath.stderr.txt" }
Write-Host "Native console checks passed. Report: $ReportPath"
exit $testExit
