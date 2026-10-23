$ErrorActionPreference='Stop'
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME){throw 'Only the declared lab host may run this test: set RUSTDESK_LAB_HOST to this COMPUTERNAME'}
$principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Administrator required'}
$folder='C:\RustDeskAgentLab'
$result=[ordered]@{schema=2;computer=$env:COMPUTERNAME;status='FAIL';cases=@();checked_at=[DateTime]::UtcNow.ToString('o')}
function State {
 $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,StartName,StartMode,PathName,ProcessId)
 $files=@(); foreach($p in @('C:\Program Files\RustDesk\RustDesk.exe','C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe')){if(Test-Path -LiteralPath $p){$files+=@{path=$p;sha256=(Get-FileHash $p -Algorithm SHA256).Hash}}}
 @{services=$services;files=$files;agent_state=(Test-Path 'C:\ProgramData\RustDesk\ManagementAgent\state\agent.json')}
}
function AddCase($id,$status,$data){$result.cases+=@{id=$id;status=$status;data=$data}}
function Install($name,$path,$consent){$log=Join-Path $folder ($name+'.log');$args='/i "'+$path+'" /qn /norestart REBOOT=ReallySuppress LAUNCH_TRAY_APP=N DEFENDER_EXCLUSIONS='+$consent+' /L*v "'+$log+'"';$p=Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList $args -PassThru;if(!$p.WaitForExit(600000)){throw "$name timeout"};$p.ExitCode}
try {
 $before=State
 $legacy=Join-Path $folder 'legacy-standard.msi'; $new=Join-Path $folder 'new-defender-standard.msi'
 $legacyCode=Install 'legacy-install' $legacy '0'; $afterLegacy=State
 AddCase 'install-legacy-standard-msi' $(if($legacyCode -in @(0,3010) -and @($afterLegacy.services|Where-Object {$_.State -eq 'Running'}).Count -eq 2){'PASS'}else{'FAIL'}) @{exit=$legacyCode;state=$afterLegacy}
 $newCode=Install 'new-upgrade-defender-msi' $new '1'; $afterNew=State
 $pref=Get-MpPreference -ErrorAction SilentlyContinue
 $paths=@('C:\Program Files\RustDesk\RustDesk.exe','C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe','C:\Program Files\RustDesk','C:\ProgramData\RustDesk\ManagementAgent','C:\ProgramData\RustDesk\Client')
 $missing=@($paths|Where-Object {@($pref.ExclusionPath)-notcontains $_})
 $rules=@{}; foreach($n in @('RustDesk Service','RustDesk Management Agent Service')){$rules[$n]=@((& netsh.exe advfirewall firewall show rule name="$n" verbose)|Select-String -Pattern 'Enabled|Program|Direction').Count}
 $last=''; $key=Get-ItemProperty -Path 'HKLM:\SOFTWARE\RustDesk\Defender' -ErrorAction SilentlyContinue;if($key){$last=$key.LastResult}
 AddCase 'upgrade-new-defender-msi' $(if($newCode -in @(0,3010) -and $missing.Count -eq 0 -and @($afterNew.services|Where-Object {$_.State -eq 'Running'}).Count -eq 2){'PASS'}else{'FAIL'}) @{exit=$newCode;state=$afterNew;missing_exclusions=$missing;firewall=$rules;last_result=$last}
 Stop-Service RustDesk -Force -ErrorAction Stop
 $deadline=(Get-Date).AddSeconds(180); do{Start-Sleep 3;$svc=(Get-Service RustDesk -ErrorAction SilentlyContinue).Status}while($svc -ne 'Running' -and (Get-Date)-lt $deadline)
 AddCase 'agent-recovers-main-service-after-stop' $(if($svc -eq 'Running'){'PASS'}else{'FAIL'}) @{service=$svc}
 Start-Service RustDesk -ErrorAction SilentlyContinue; Start-Sleep 5
 $main='C:\Program Files\RustDesk\RustDesk.exe'
 foreach($p in @(Get-CimInstance Win32_Process -Filter "Name='rustdesk.exe'"|Where-Object {$_.ExecutablePath -eq $main})){Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue}
 $deadline=(Get-Date).AddSeconds(180); do{Start-Sleep 3;$proc=@(Get-CimInstance Win32_Process -Filter "Name='rustdesk.exe'"|Where-Object {$_.ExecutablePath -eq $main})}while($proc.Count -eq 0 -and (Get-Date)-lt $deadline)
 AddCase 'agent-recovers-main-process-after-kill' $(if($proc.Count -gt 0){'PASS'}else{'FAIL'}) @{process_count=$proc.Count}
 $result.status=if(@($result.cases|Where-Object {$_.status -eq 'FAIL'}).Count){'FAIL'}else{'PASS'}
} catch {$result.error=$_.Exception.Message}
finally{$result.final=State;$result.checked_at=[DateTime]::UtcNow.ToString('o');$result|ConvertTo-Json -Depth 12|Set-Content (Join-Path $folder 'full-product-test-result.json') -Encoding utf8}
if($result.status -ne 'PASS'){exit 1};exit 0
