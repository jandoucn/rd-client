param([ValidateSet('present','missing')][string]$Scenario='present')
$ErrorActionPreference='Stop'
if((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win11'))){throw 'Win11 only'}
$name="RustDeskShortcut-Machine-$Scenario"
if(Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue){throw 'task already exists'}
& "$env:SystemRoot\System32\icacls.exe" C:\Users\Public\shortcut-machine /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' | Out-Null
if($LASTEXITCODE -ne 0){throw 'failed to protect task directory'}
$action=New-ScheduledTaskAction -Execute C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe -Argument "-NoProfile -ExecutionPolicy Bypass -File C:\Users\Public\shortcut-machine\worker.ps1 -Scenario $Scenario"
Register-ScheduledTask -TaskName $name -Action $action -Principal (New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest) | Out-Null
Start-ScheduledTask -TaskName $name
