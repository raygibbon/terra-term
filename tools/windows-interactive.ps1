param([ValidateSet('WindowsTerminal','ConsoleHost')][string]$HostName = 'WindowsTerminal')
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --examples
if ($LASTEXITCODE -ne 0) { throw 'Example build failed.' }
$report = Join-Path (Get-Location) "target/manual-$HostName.json"
$env:TERRA_TERM_INPUT_LOG = Join-Path (Get-Location) "target/manual-$HostName-input.log"
$results = [ordered]@{ Host = $HostName; WT_SESSION = $env:WT_SESSION; Date = (Get-Date -Format o); Observations = @() }
$results | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 $report
foreach ($example in @('input','unicode','colors','resize','lifecycle')) {
    Write-Host "terra-term manual check: $example"
    if ($example -eq 'input') {
        Write-Host 'Test Ctrl+A-Z (Ctrl+[ quits), Ctrl+backslash, Ctrl+], Ctrl+^, Ctrl+_, Ctrl+C, Ctrl+Break, Shift/Alt, F1-F24, Unicode, mouse/buttons/drag/wheels and resize. Esc exits. Events are logged.'
    }
    Write-Host 'Note the original screen: TERRA-TERM-RESTORE-MARKER. Check it returns when the example exits.'
    & "target/debug/examples/$example.exe"
    $exitCode = $LASTEXITCODE
    $observation = Read-Host "Record actual observations, failures, and untested items for $example (exit $exitCode)"
    $results.Observations += [ordered]@{ Example = $example; ExitCode = $exitCode; Observation = $observation }
    $results | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8 $report
}
Write-Host "Manual observations saved to $report"
