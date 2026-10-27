# Run management-agent terminal tests on win10-gs.
param(
  [string]$Filter = 'terminal::',
  [string]$Repo = 'D:\github\rd-client',
  [string]$LogDir = 'D:\tmp\rdtest\agent-build'
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$log = Join-Path $LogDir ("terminal-test-{0}.log" -f $stamp)
$exitFile = Join-Path $LogDir 'terminal-test-exit.txt'

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
  $env:Path = (Join-Path $toolchain 'bin') + ';' + $env:Path
  $vcvars = 'C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat'
  Set-Location -LiteralPath $Repo
  $cmd = 'call "' + $vcvars + '" && cargo test --locked --bin rustdesk-management-agent -- ' + $Filter + ' --nocapture'
  Write-Log ("cmd={0}" -f $cmd)
  $stdoutLog = Join-Path $LogDir ("terminal-test-stdout-{0}.log" -f $stamp)
  $stderrLog = Join-Path $LogDir ("terminal-test-stderr-{0}.log" -f $stamp)
  $p = Start-Process -FilePath 'cmd.exe' -ArgumentList @('/c', $cmd) `
    -WorkingDirectory $Repo -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $stdoutLog `
    -RedirectStandardError $stderrLog
  $code = $p.ExitCode
  Set-Content -LiteralPath $exitFile -Value $code -Encoding ASCII
  Write-Log ("exit={0}" -f $code)
  if (Test-Path -LiteralPath $stdoutLog) {
    Get-Content -LiteralPath $stdoutLog -Tail 80 | ForEach-Object { Write-Log $_ }
  }
  exit $code
}
catch {
  Write-Log ("ERROR: " + $_.Exception.Message)
  Set-Content -LiteralPath $exitFile -Value 1 -Encoding ASCII
  exit 1
}
