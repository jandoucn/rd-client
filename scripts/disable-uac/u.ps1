# Disable UAC completely (Windows 10/11, machine-wide).
# Usage:
#   Set-ExecutionPolicy Bypass -Scope Process -Force; irm https://shell.olii.cc/u.ps1 | iex
# Local:
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\u.ps1

$ErrorActionPreference = 'Stop'
$ScriptUrl = 'https://shell.olii.cc/u.ps1'
$RegPath = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
$Desired = [ordered]@{
  EnableLUA                  = 0
  ConsentPromptBehaviorAdmin = 0
  PromptOnSecureDesktop      = 0
  FilterAdministratorToken   = 0
}

function Test-IsAdmin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $p = New-Object Security.Principal.WindowsPrincipal $id
  return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Start-Elevated {
  # irm|iex has no $PSCommandPath; re-fetch from URL under admin.
  $cmd = if ($PSCommandPath) {
    "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
  } else {
    "-NoProfile -ExecutionPolicy Bypass -Command `"irm '$ScriptUrl' | iex`""
  }
  Write-Host 'Requesting admin rights (last UAC prompt)...' -ForegroundColor Yellow
  Start-Process powershell.exe -Verb RunAs -ArgumentList $cmd | Out-Null
}

function Get-UacValues {
  $props = Get-ItemProperty -Path $RegPath -ErrorAction SilentlyContinue
  $out = [ordered]@{}
  foreach ($n in $Desired.Keys) {
    $out[$n] = if ($props -and ($props.PSObject.Properties.Name -contains $n)) { [int]$props.$n } else { $null }
  }
  return $out
}

function Test-Disabled([System.Collections.IDictionary]$cur) {
  foreach ($n in $Desired.Keys) {
    if ($cur[$n] -ne $Desired[$n]) { return $false }
  }
  return $true
}

function Show-UacValues([string]$title, [System.Collections.IDictionary]$values) {
  Write-Host $title -ForegroundColor Cyan
  foreach ($n in $values.Keys) {
    $v = if ($null -eq $values[$n]) { '(not set)' } else { $values[$n] }
    Write-Host ("  {0,-28} = {1}" -f $n, $v)
  }
}

function Set-UacDisabled {
  if (-not (Test-Path $RegPath)) {
    New-Item -Path $RegPath -Force | Out-Null
  }
  foreach ($n in $Desired.Keys) {
    New-ItemProperty -Path $RegPath -Name $n -Value $Desired[$n] -PropertyType DWord -Force | Out-Null
  }
}

try {
  if (-not (Test-IsAdmin)) {
    Start-Elevated
    return
  }

  $before = Get-UacValues
  Show-UacValues 'Current UAC settings:' $before

  if (Test-Disabled $before) {
    Write-Host 'UAC already disabled. No reboot needed for registry values.' -ForegroundColor Green
    Write-Host 'If apps still prompt, reboot once manually.' -ForegroundColor DarkGray
    return
  }

  Set-UacDisabled

  $after = Get-UacValues
  Show-UacValues 'New UAC settings:' $after

  if (-not (Test-Disabled $after)) {
    throw 'Write verification failed. Group Policy may be overriding these keys.'
  }

  Write-Host 'UAC disabled (no auto reboot).' -ForegroundColor Green
  Write-Host 'EnableLUA takes full effect after a manual reboot.' -ForegroundColor Yellow
  Write-Host 'Reboot when ready:  Restart-Computer' -ForegroundColor DarkGray
}
catch {
  Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
  if ($PSCommandPath) { Read-Host 'Press Enter to exit' | Out-Null }
  exit 1
}
