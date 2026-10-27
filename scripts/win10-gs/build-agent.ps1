# Build rustdesk-management-agent on win10-gs.
# Must run under a shell that already has (or will call) vcvars64.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-agent.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File build-agent.ps1 -Profile release -Filter "terminal::"
param(
  [ValidateSet('debug', 'release')]
  [string]$Profile = 'debug',
  [string]$Filter = '',
  [string]$Repo = 'D:\github\rd-client',
  [string]$LogDir = 'D:\tmp\rdtest\agent-build'
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$log = Join-Path $LogDir ("build-agent-{0}-{1}.log" -f $Profile, $stamp)
$exitFile = Join-Path $LogDir ("build-agent-{0}-exit.txt" -f $Profile)

function Write-Log([string]$msg) {
  $line = "[{0}] {1}" -f (Get-Date -Format 'HH:mm:ss'), $msg
  Add-Content -LiteralPath $log -Value $line -Encoding UTF8
  Write-Host $line
}

try {
  $toolchain = 'D:\build-env\rustup\toolchains\1.90.0-x86_64-pc-windows-msvc'
  $env:RUSTUP_HOME = 'D:\build-env\rustup'
  $env:CARGO_HOME = 'D:\build-env\cargo'
  $env:RUSTC = Join-Path $toolchain 'bin\rustc.exe'
  $env:RUSTUP_TOOLCHAIN = '1.90.0-x86_64-pc-windows-msvc'
  $env:VCPKG_ROOT = 'D:\build-env\vcpkg-agent'
  $env:LIBCLANG_PATH = 'D:\build-env\libclang-agent\clang\native'
  $env:PUB_CACHE = 'C:\Users\Administrator\AppData\Local\Pub\Cache'
  $env:NUGET_PACKAGES = 'C:\Users\Administrator\.nuget\packages'
  # Put toolchain bin first; do NOT prepend cargo\bin proxies.
  $env:Path = (Join-Path $toolchain 'bin') + ';' + $env:Path

  $vcvars = 'C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat'
  if (-not (Test-Path -LiteralPath $vcvars)) {
    throw ("vcvars64.bat missing: {0}" -f $vcvars)
  }

  Set-Location -LiteralPath $Repo
  Write-Log ("repo={0} profile={1} filter={2}" -f $Repo, $Profile, $Filter)
  Write-Log ("rustc={0}" -f $env:RUSTC)

  $cargoArgs = @('build', '--locked', '--bin', 'rustdesk-management-agent')
  if ($Profile -eq 'release') {
    $cargoArgs += '--release'
  }
  $argLine = ($cargoArgs | ForEach-Object {
      if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }) -join ' '
  $cmd = 'call "' + $vcvars + '" && cargo ' + $argLine
  Write-Log ("cmd={0}" -f $cmd)

  $stdoutLog = Join-Path $LogDir ("build-agent-stdout-{0}.log" -f $stamp)
  $stderrLog = Join-Path $LogDir ("build-agent-stderr-{0}.log" -f $stamp)
  $p = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/c', $cmd) `
    -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $stdoutLog `
    -RedirectStandardError $stderrLog
  $code = $p.ExitCode
  Set-Content -LiteralPath $exitFile -Value $code -Encoding ASCII
  Write-Log ("exit={0} log={1}" -f $code, $log)
  if ($code -ne 0) {
    Write-Log '--- stderr tail ---'
    if (Test-Path -LiteralPath $stderrLog) {
      Get-Content -LiteralPath $stderrLog -Tail 80 | ForEach-Object { Write-Log $_ }
    }
    exit $code
  }

  if ($Filter) {
    $testArgs = @('test', '--locked', '--bin', 'rustdesk-management-agent', '--', $Filter, '--nocapture')
    if ($Profile -eq 'release') {
      $testArgs = @('test', '--locked', '--release', '--bin', 'rustdesk-management-agent', '--', $Filter, '--nocapture')
    }
    $testLine = ($testArgs | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
      }) -join ' '
    $tcmd = 'call "' + $vcvars + '" && cargo ' + $testLine
    Write-Log ("test cmd={0}" -f $tcmd)
    $tOut = Join-Path $LogDir ("test-agent-stdout-{0}.log" -f $stamp)
    $tErr = Join-Path $LogDir ("test-agent-stderr-{0}.log" -f $stamp)
    $tp = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/c', $tcmd) `
      -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru `
      -RedirectStandardOutput $tOut `
      -RedirectStandardError $tErr
    $tcode = $tp.ExitCode
    Set-Content -LiteralPath (Join-Path $LogDir 'test-agent-exit.txt') -Value $tcode -Encoding ASCII
    Write-Log ("test exit={0}" -f $tcode)
    if ($tcode -ne 0) {
      if (Test-Path -LiteralPath $tOut) {
        Get-Content -LiteralPath $tOut -Tail 100 | ForEach-Object { Write-Log $_ }
      }
      exit $tcode
    }
  }

  if ($Profile -eq 'release') {
    $bin = Join-Path $Repo 'target\release\rustdesk-management-agent.exe'
  } else {
    $bin = Join-Path $Repo 'target\debug\rustdesk-management-agent.exe'
  }
  if (-not (Test-Path -LiteralPath $bin)) {
    throw ("binary missing after build: {0}" -f $bin)
  }
  $hash = (Get-FileHash -LiteralPath $bin -Algorithm SHA256).Hash.ToLower()
  Write-Log ("binary={0} sha256={1}" -f $bin, $hash)
  Set-Content -LiteralPath (Join-Path $LogDir 'last-binary.txt') -Value ("{0}`n{1}" -f $bin, $hash) -Encoding ASCII
  exit 0
}
catch {
  Write-Log ("ERROR: " + $_.Exception.Message)
  Set-Content -LiteralPath $exitFile -Value 1 -Encoding ASCII
  exit 1
}
