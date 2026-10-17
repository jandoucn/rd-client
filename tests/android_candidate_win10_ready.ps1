param(
    [Parameter(Mandatory = $true)][string]$RepoRoot,
    [Parameter(Mandatory = $true)][string]$OverlayZip,
    [string]$FlutterBat,
    [string]$OutputDirectory,
    [switch]$VerifyOnly
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Read-ZipText($Archive, [string]$Name) {
    $entry = $Archive.GetEntry($Name)
    if ($null -eq $entry) { throw "Missing archive entry: $Name" }
    $reader = [System.IO.StreamReader]::new($entry.Open())
    try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
}
function Hash-Bytes([byte[]]$Bytes) {
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $algorithm.Dispose() }
}

$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
$OverlayZip = (Resolve-Path -LiteralPath $OverlayZip).Path
$archive = [System.IO.Compression.ZipFile]::OpenRead($OverlayZip)
try {
    $receipt = (Read-ZipText $archive 'overlay-receipt.json') | ConvertFrom-Json
    $manifestText = Read-ZipText $archive 'tests/android_candidate_testmanifest.json'
    $manifest = $manifestText | ConvertFrom-Json
    if ($receipt.sources.Count -ne 11 -or $manifest.tests.Count -ne 5 -or
        $manifest.expected_total_cases -ne 93) { throw 'Unexpected candidate manifest' }
    foreach ($source in $receipt.sources) {
        $path = Join-Path $RepoRoot $source.path
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $source.sha256) {
            throw "Working-tree source drift: $($source.path)"
        }
        $entry = $archive.GetEntry($source.path)
        if ($null -eq $entry) { throw "Missing source: $($source.path)" }
        $stream = $entry.Open()
        $buffer = [System.IO.MemoryStream]::new()
        try {
            $stream.CopyTo($buffer)
            if ((Hash-Bytes $buffer.ToArray()) -ne $source.sha256) { throw "Archive source drift: $($source.path)" }
        } finally { $stream.Dispose(); $buffer.Dispose() }
    }
    foreach ($dependency in $receipt.dependencies_not_in_archive) {
        $text = [System.IO.File]::ReadAllText((Join-Path $RepoRoot $dependency.path)).Replace("`r`n", "`n")
        if ((Hash-Bytes ([System.Text.Encoding]::UTF8.GetBytes($text))) -ne $dependency.normalized_sha256) {
            throw "Dependency drift (will not resolve or modify): $($dependency.path)"
        }
    }
} finally { $archive.Dispose() }

$overlayHash = (Get-FileHash -LiteralPath $OverlayZip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($VerifyOnly) {
    [pscustomobject]@{ source_files = 11; test_files = 5; static_expected_cases = 93;
        overlay_sha256 = $overlayHash; tests_executed = $false } | ConvertTo-Json
    exit 0
}
if (-not $FlutterBat -or -not (Test-Path -LiteralPath $FlutterBat -PathType Leaf)) {
    throw 'Supply the existing Flutter SDK flutter.bat path; no installation is performed'
}
if (-not $OutputDirectory -or -not [System.IO.Path]::IsPathRooted($OutputDirectory)) {
    throw 'Supply an absolute fresh output directory'
}
if ((Test-Path -LiteralPath $OutputDirectory) -or
    -not (Test-Path -LiteralPath (Split-Path -Parent $OutputDirectory) -PathType Container)) {
    throw 'Output directory must be absent and its parent must exist'
}
if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot 'flutter/.dart_tool/package_config.json'))) {
    throw 'Use the prepared checkout with dependencies already available; pub get is not allowed'
}
[void][System.IO.Directory]::CreateDirectory($OutputDirectory)
$stdout = Join-Path $OutputDirectory 'tests.jsonl'
$stderr = Join-Path $OutputDirectory 'stderr.log'
$arguments = @('--no-version-check', 'test', '--no-pub', '--reporter=json', '--concurrency=1')
$arguments += @($manifest.tests | ForEach-Object { $_.path.Substring('flutter/'.Length) })
$started = [DateTime]::UtcNow.ToString('o')
$exitCode = -1
Push-Location (Join-Path $RepoRoot 'flutter')
try {
    & $FlutterBat @arguments 1> $stdout 2> $stderr
    $exitCode = $LASTEXITCODE
} finally { Pop-Location }

$events = @(Get-Content -LiteralPath $stdout | ForEach-Object {
    try { $_ | ConvertFrom-Json } catch { }
})
$suites = @{}
$tests = @{}
foreach ($event in $events) {
    if ($event.type -eq 'suite') { $suites[$event.suite.id] = $event.suite.path.Replace('\', '/') }
    if ($event.type -eq 'testStart') { $tests[$event.test.id] = $event.test }
}
$completed = @($events | Where-Object { $_.type -eq 'testDone' -and -not $_.hidden })
$counts = @()
$countsMatch = $true
foreach ($expected in $manifest.tests) {
    $actual = @($completed | Where-Object {
        $test = $tests[$_.testID]
        $null -ne $test -and $suites[$test.suiteID].EndsWith($expected.path.Substring('flutter/'.Length))
    }).Count
    if ($actual -ne $expected.expected_cases) { $countsMatch = $false }
    $counts += [pscustomobject]@{ path = $expected.path; expected = $expected.expected_cases; actual = $actual }
}
$failed = @($completed | Where-Object { $_.result -ne 'success' -or $_.skipped }).Count
$done = @($events | Where-Object { $_.type -eq 'done' -and $_.success -eq $true }).Count -eq 1
$passed = $exitCode -eq 0 -and $done -and $countsMatch -and $failed -eq 0 -and $completed.Count -eq 93
$result = [pscustomobject]@{
    machine = $env:COMPUTERNAME; started_utc = $started; ended_utc = [DateTime]::UtcNow.ToString('o');
    flutter = $FlutterBat; arguments = $arguments; overlay_sha256 = $overlayHash;
    exit_code = $exitCode; tests_executed = $true; cases = $counts; accepted = $passed
}
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Encoding UTF8
$result | ConvertTo-Json -Depth 8
if (-not $passed) { exit 1 }
