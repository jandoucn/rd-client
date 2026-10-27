# Safely replace the installed management-agent on win10-gs.
# Handles System file attribute and SCM failure actions.
#
# Usage (admin):
#   powershell -NoProfile -ExecutionPolicy Bypass -File replace-agent.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File replace-agent.ps1 -Source D:\github\rd-client\target\release\rustdesk-management-agent.exe
param(
  [string]$Source = 'D:\github\rd-client\target\debug\rustdesk-management-agent.exe',
  [string]$InstallDir = 'C:\ProgramData\RustDesk\ManagementAgent',
  [string]$ServiceName = 'RustDeskManagementAgent',
  [string]$BackupDir = 'D:\tmp\rdtest\agent-backup'
)

$ErrorActionPreference = 'Stop'
$dest = Join-Path $InstallDir 'rustdesk-management-agent.exe'
if (-not (Test-Path -LiteralPath $Source)) {
  throw ("source missing: {0}" -f $Source)
}
New-Item -ItemType Directory -Force -Path $BackupDir | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backup = Join-Path $BackupDir ("rustdesk-management-agent-{0}.exe" -f $stamp)

Write-Host ("source={0}" -f $Source)
Write-Host ("dest={0}" -f $dest)
Write-Host ("backup={0}" -f $backup)

if (Test-Path -LiteralPath $dest) {
  Copy-Item -LiteralPath $dest -Destination $backup -Force
  $bhash = (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash.ToLower()
  Write-Host ("backed up sha256={0}" -f $bhash)
}

# Clear failure actions so stop is not undone by SCM restart in 5s.
sc.exe failure $ServiceName reset= 0 actions= '//' | Out-Null
Stop-Service -Name $ServiceName -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
Get-Process -Name 'rustdesk-management-agent' -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -and ($_.Path -ieq $dest) } |
  ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 1

# System-attribute files cannot be overwritten in place: rename first, then copy.
$old = Join-Path $InstallDir ("rustdesk-management-agent.old-{0}.exe" -f $stamp)
if (Test-Path -LiteralPath $dest) {
  Rename-Item -LiteralPath $dest -NewName (Split-Path $old -Leaf) -Force
}
Copy-Item -LiteralPath $Source -Destination $dest -Force
if (Test-Path -LiteralPath $old) {
  Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
}

sc.exe failure $ServiceName reset= 0 actions= 'restart/5000/restart/5000/restart/5000' | Out-Null
Start-Service -Name $ServiceName
Start-Sleep -Seconds 2
$svc = Get-Service -Name $ServiceName
$sha = (Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLower()
Write-Host ("service={0} sha256={1}" -f $svc.Status, $sha)
if ($svc.Status -ne 'Running') {
  throw 'service not running after replace'
}
