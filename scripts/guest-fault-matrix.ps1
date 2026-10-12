$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -notlike 'RDLAB-*'){throw 'Only lab guests may run this test'}
$folder='C:\RustDeskAgentLab'
$main='C:\Program Files\RustDesk\RustDesk.exe'
$fallback='C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe'
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$marker='C:\ProgramData\RustDesk\ManagementAgent\state\service-recovery-enabled'
$msi=Join-Path $folder 'new-fallback-standard.msi'
$result=[ordered]@{schema=2;computer=$env:COMPUTERNAME;status='FAIL';cases=@();checked_at=[DateTime]::UtcNow.ToString('o')}
$mainBackup=$main+'.fault-backup'
$fallbackBackup=$fallback+'.fault-backup'
$createdFallback=$false;$ownedMarker=$false;$mainMoved=$false;$fallbackMoved=$false
function State {
 $service=Get-CimInstance Win32_Service -Filter "Name='RustDesk'" -ErrorAction SilentlyContinue
 $agentService=Get-CimInstance Win32_Service -Filter "Name='RustDeskManagementAgent'" -ErrorAction SilentlyContinue
 $state=Get-Content -LiteralPath 'C:\ProgramData\RustDesk\ManagementAgent\state\agent.json' -Raw -ErrorAction SilentlyContinue | ConvertFrom-Json
 @{main_service=@{state=$service.State;path=$service.PathName;pid=$service.ProcessId};agent_service=@{state=$agentService.State;pid=$agentService.ProcessId};main_present=(Test-Path $main);fallback_present=(Test-Path $fallback);recovery_at=$state.last_service_recovery_at;recovery_error=$state.service_recovery_error;failures=$state.service_recovery_failures}
}
function AddCase($id,$status,$data){$result.cases+=@{id=$id;status=$status;data=$data}}
function Wait-Main([int]$seconds=240){$deadline=(Get-Date).AddSeconds($seconds);do{Start-Sleep -Seconds 3;$s=Get-CimInstance Win32_Service -Filter "Name='RustDesk'" -ErrorAction SilentlyContinue;$p=@(Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'" -ErrorAction SilentlyContinue|Where-Object {$_.ExecutablePath -eq $main})}while(($null -eq $s -or $s.State -ne 'Running' -or $p.Count -eq 0) -and (Get-Date)-lt $deadline);return @{service=$s.State;pid=$s.ProcessId;process_count=$p.Count;path=$s.PathName}}
function Config-Main([string]$path){$binary='"'+$path+'" --service';$p=Start-Process -FilePath "$env:SystemRoot\System32\sc.exe" -ArgumentList @('config','RustDesk','binPath=',$binary,'start=','auto','obj=','LocalSystem') -Wait -PassThru -WindowStyle Hidden;if($p.ExitCode -ne 0){throw "sc config failed $($p.ExitCode)"}}
function Install-NewPackage {$args='/i "'+$msi+'" /qn /norestart REBOOT=ReallySuppress LAUNCH_TRAY_APP=N DEFENDER_EXCLUSIONS=0 /L*v "'+(Join-Path $folder 'fault-upgrade.log')+'"';$p=Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList $args -PassThru;if(!$p.WaitForExit(600000)){throw 'fault upgrade timeout'};return $p.ExitCode}
try {
 $code=Install-NewPackage;AddCase 'fault-matrix-upgrade' $(if($code -in @(0,3010)){'PASS'}else{'FAIL'}) @{exit=$code;state=(State)}
 if(!(Test-Path $marker)){New-Item -ItemType File -Path $marker -Force|Out-Null;$ownedMarker=$true}
 if(!(Test-Path $fallback)){New-Item -ItemType Directory -Path (Split-Path $fallback) -Force|Out-Null;Copy-Item $main $fallback;$createdFallback=$true}
 Start-Service RustDesk -ErrorAction SilentlyContinue;Start-Sleep -Seconds 5
 Stop-Service RustDesk -Force; $normal=Wait-Main
 AddCase 'main-service-normal-stop' ($normal.process_count -gt 0) $normal
 Start-Service RustDesk -ErrorAction SilentlyContinue;Start-Sleep -Seconds 5
 foreach($p in @(Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'"|Where-Object {$_.ExecutablePath -eq $main})){Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue};$killed=Wait-Main
 AddCase 'main-process-kill' ($killed.process_count -gt 0) $killed
 Stop-Service RustDesk -Force;Config-Main 'C:\RustDesk\missing\RustDesk.exe';$bad=Wait-Main
 AddCase 'wrong-service-binpath-repaired-to-normal' ($bad.process_count -gt 0 -and (Get-CimInstance Win32_Service -Filter "Name='RustDesk'").PathName -match 'Program Files\\RustDesk\\RustDesk.exe') $bad
 Config-Main $main;Start-Service RustDesk -ErrorAction SilentlyContinue;Start-Sleep -Seconds 5
 Stop-Service RustDesk -Force;Move-Item $main $mainBackup;$mainMoved=$true;Config-Main 'C:\RustDesk\missing\RustDesk.exe';$fb=Wait-Main
 AddCase 'normal-missing-fallback-client-started' ($fb.process_count -gt 0 -and (Get-CimInstance Win32_Service -Filter "Name='RustDesk'").PathName -match 'Client\\standard\\RustDesk\\RustDesk.exe') $fb
 Stop-Service RustDesk -Force;Config-Main 'C:\RustDesk\missing\RustDesk.exe';Move-Item $fallback $fallbackBackup;$fallbackMoved=$true;$both=Wait-Main 70
 AddCase 'normal-and-fallback-missing-bounded-failure' ($both.process_count -eq 0) @{observed=$both;expected='agent remains running and records bounded recovery error; no untrusted executable is started'}
 $result.status=if(@($result.cases|Where-Object {$_.status -eq 'FAIL'}).Count){'FAIL'}else{'PASS'}
}catch{$result.error=$_.Exception.Message}
finally {
 try{if($mainMoved -and (Test-Path $mainBackup)){Move-Item $mainBackup $main -Force}}catch{$result.restore_main_error=$_.Exception.Message}
 try{if($fallbackMoved -and (Test-Path $fallbackBackup)){Move-Item $fallbackBackup $fallback -Force}}catch{$result.restore_fallback_error=$_.Exception.Message}
 try{if($createdFallback -and (Test-Path $fallback)){Remove-Item $fallback -Force}}catch{}
 try{Config-Main $main}catch{$result.restore_service_error=$_.Exception.Message}
 try{Start-Service RustDesk -ErrorAction SilentlyContinue}catch{}
 if($ownedMarker){Remove-Item $marker -Force -ErrorAction SilentlyContinue}
 $result.final=State;$result.checked_at=[DateTime]::UtcNow.ToString('o');$result|ConvertTo-Json -Depth 12|Set-Content (Join-Path $folder 'fault-matrix-result.json') -Encoding utf8
}
if($result.status -ne 'PASS'){exit 1};exit 0
