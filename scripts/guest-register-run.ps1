$ErrorActionPreference='Stop'
$folder='C:\RustDeskAgentLab'
$action=New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument '-NoProfile -ExecutionPolicy Bypass -File C:\RustDeskAgentLab\guest-full-product-test.ps1'
$principal=New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
$settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 30) -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName RustDeskAgentLabDriver -Action $action -Principal $principal -Settings $settings -Force | Out-Null
Start-ScheduledTask -TaskName RustDeskAgentLabDriver
