$ErrorActionPreference='Stop'
$folder='C:\RustDeskAgentLab'
$msi=Join-Path $folder 'new-marker-fallback-standard.msi'
$log=Join-Path $folder 'new-marker-fallback-install.log'
$arguments='/i "'+$msi+'" /qn /norestart REBOOT=ReallySuppress LAUNCH_TRAY_APP=N DEFENDER_EXCLUSIONS=0 /L*v "'+$log+'"'
$process=Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList $arguments -PassThru
if(!$process.WaitForExit(600000)){throw 'installer timeout'}
$marker='C:\ProgramData\RustDesk\ManagementAgent\state\service-recovery-enabled'
$deadline=(Get-Date).AddSeconds(30)
do{Start-Sleep -Seconds 2}while(!(Test-Path -LiteralPath $marker) -and (Get-Date)-lt $deadline)
@{installer_exit=$process.ExitCode;marker=(Test-Path -LiteralPath $marker);services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,PathName,StartName);checked_at=[DateTime]::UtcNow.ToString('o')}|ConvertTo-Json -Depth 6|Set-Content (Join-Path $folder 'new-marker-fallback-install-result.json') -Encoding utf8
if($process.ExitCode -notin @(0,3010) -or !(Test-Path -LiteralPath $marker)){exit 1};exit 0
