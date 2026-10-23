param([ValidateSet('Prepare','Install','Cycle')][string]$Mode)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'WIN10_SYSTEM_REQUIRED: set RUSTDESK_LAB_HOST to this COMPUTERNAME (RUSTDESK_LAB_OS unset or win10) and run as SYSTEM'}
$lab='C:\RustDeskAgentLab';$prefix="$lab\bootstrap-v213-$Mode"
if(Test-Path "$prefix.result.json"){throw 'RUN_USED'}
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$boot='C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe'
$manifest='C:\ProgramData\RustDesk\RecoveryBootstrap\bootstrap.json'
$state='C:\ProgramData\RustDesk\ManagementAgent\state'
$budget='C:\ProgramData\RustDesk\RecoveryBootstrap\budget.json'
$name='RustDeskManagementAgent';$task='RustDeskManagementAgentRecovery'
$channel='Microsoft-Windows-TaskScheduler/Operational'
$r=@{mode=$Mode;run_id='v213';state='unknown';errors=@();scope='non-test Debug local MSI production code, not official Release';remote_control='not_tested';started=[DateTime]::UtcNow.ToString('o')}
$clock=[Diagnostics.Stopwatch]::StartNew()
function Assert([bool]$Value,[string]$Message){if(!$Value){throw $Message}}
function Hash([string]$Path){[string](Get-FileHash -LiteralPath $Path).Hash}
function Json([string]$Path){[IO.File]::ReadAllText($Path)|ConvertFrom-Json}
function Save($Value,[string]$Path){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 16))}
function Services { @(Get-CimInstance Win32_Service|Where-Object Name -in @('RustDesk','RustDeskManagementAgent')|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name) }
function Health([string]$Label,[int64]$Build){
    $p=Start-Process $agent -ArgumentList 'status --json --edition standard --package-kind msi' -PassThru -RedirectStandardOutput "$prefix.$Label.status.json" -RedirectStandardError "$prefix.$Label.stderr.txt"
    $null=$p.Handle;Assert ($p.WaitForExit(30000)) 'STATUS_TIMEOUT';$p.WaitForExit();Assert ($p.ExitCode -eq 0) 'STATUS_EXIT'
    $s=Json "$prefix.$Label.status.json"
    Assert ($s.application.state -eq 'ready' -and $s.agent.build_seq -eq $Build -and $s.application.build_seq -eq $Build) 'HEALTH_BUILD'
    Assert ($s.agent_installation_id -eq '76b922ac75fb14f19783bb6d48c45653' -and $s.package_identity.edition -eq 'standard' -and $s.package_identity.kind -eq 'msi') 'INSTALLATION_IDENTITY'
    $services=Services
    Assert ($services.Count -eq 2 -and @($services|Where-Object {$_.State -ne 'Running' -or $_.StartMode -ne 'Auto' -or $_.StartName -ne 'LocalSystem'}).Count -eq 0) 'SERVICES_UNHEALTHY'
    $processes=@($services|ForEach-Object {$p=Get-Process -Id $_.ProcessId;@{name=[string]$_.Name;pid=$p.Id;created=[string]$p.StartTime.ToUniversalTime().ToFileTimeUtc();path=$p.Path}})
    $hashes=@{agent=(Hash $agent);bootstrap=(Hash $boot);dll=(Hash 'C:\Program Files\RustDesk\librustdesk.dll');manifest=(Hash $manifest)}
    Assert ($hashes.agent -eq $hashes.bootstrap) 'AGENT_BOOTSTRAP_MISMATCH'
    $m=Json $manifest;Assert ($m.agent_sha256 -eq $hashes.agent -and $m.edition -eq 'standard' -and $m.package_kind -eq 'msi') 'MANIFEST_MISMATCH'
    @{status=$s;services=$services;processes=$processes;hashes=$hashes;manifest=$m;task=(Export-ScheduledTask $task);budget=if(Test-Path $budget){Json $budget}else{$null}}
}
function Backup-Service([string]$Label){
    $reg=Get-Item "HKLM:\SYSTEM\CurrentControlSet\Services\$name";$values=@{}
    foreach($key in $reg.GetValueNames()){$values[$key]=$reg.GetValue($key)}
    & "$env:SystemRoot\System32\reg.exe" export "HKLM\SYSTEM\CurrentControlSet\Services\$name" "$prefix.$Label.scm.reg" /y|Out-Null;Assert ($LASTEXITCODE -eq 0) 'SCM_EXPORT'
    $sddl=((& "$env:SystemRoot\System32\sc.exe" sdshow $name)|Where-Object {$_ -match '^[ODGS]:'}) -join '';Assert ($LASTEXITCODE -eq 0 -and $sddl.Length -gt 0) 'SDDL_EXPORT'
    $details=@{};foreach($op in @('qc','qdescription','qfailure','qfailureflag','qsidtype','qprivs','qtriggerinfo')){$text=& "$env:SystemRoot\System32\sc.exe" $op $name;Assert ($LASTEXITCODE -eq 0) 'SCM_QUERY';$details[$op]=$text -join "`r`n"}
    @{registry=$values;sddl=$sddl;details=$details}
}
function Last-Record { (Get-WinEvent -LogName $channel -MaxEvents 1).RecordId }
function Events([long]$Floor){
    try{$list=@(Get-WinEvent -LogName $channel -FilterXPath "*[System[EventRecordID > $Floor]]")}catch{if($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*'){return @()};throw}
    @($list|ForEach-Object {[xml]$x=$_.ToXml();$d=@{};foreach($v in $x.SelectNodes('//*[local-name()="EventData"]/*[local-name()="Data"]')){$d[$v.GetAttribute('Name')]=$v.InnerText};@{id=$_.Id;record_id=$_.RecordId;data=$d;xml=$_.ToXml()}}|Where-Object {$_.data['TaskName'] -eq "\$task"}|Sort-Object record_id)
}
function Completed-Periodic([object[]]$Events){
    $triggers=@($Events|Where-Object id -eq 107|Sort-Object record_id -Descending)
    foreach($trigger in @($triggers|Select-Object -First 1)){
        $id=$trigger.data['InstanceId'];if(!$id){$id=$trigger.data['TaskInstanceId']}
        if(!$id){continue}
        $same=@($Events|Where-Object {$_.data['InstanceId'] -eq $id -or $_.data['TaskInstanceId'] -eq $id})
        if(@($same|Where-Object id -eq 329).Count){throw 'PERIODIC_TASK_329'}
        $done=@($same|Where-Object id -eq 201)
        if($done.Count){Assert ($done[0].data['ResultCode'] -eq '0') 'PERIODIC_TASK_FAILED';return @{instance=$id;trigger_record=$trigger.record_id;complete_record=$done[0].record_id}}
    }
    return $null
}
Add-Type @'
using System;using System.Runtime.InteropServices;
public static class AgentNative213 {
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenSCManager(string m,string d,uint a);
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenService(IntPtr m,string n,uint a);
 [DllImport("advapi32.dll",SetLastError=true)] static extern bool DeleteService(IntPtr s);
 [DllImport("advapi32.dll")] static extern bool CloseServiceHandle(IntPtr s);
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateService(IntPtr m,string n,string dn,uint access,uint type,uint start,uint err,string path,string group,IntPtr tag,string deps,string account,string pw);
 public static int Observe(){IntPtr m=OpenSCManager(null,null,1);if(m==IntPtr.Zero)return Marshal.GetLastWin32Error();try{IntPtr s=OpenService(m,"RustDeskManagementAgent",1);if(s==IntPtr.Zero)return Marshal.GetLastWin32Error();CloseServiceHandle(s);return 0;}finally{CloseServiceHandle(m);}}
 public static void Delete(){IntPtr m=OpenSCManager(null,null,1);if(m==IntPtr.Zero)throw new Exception("SCM "+Marshal.GetLastWin32Error());try{IntPtr s=OpenService(m,"RustDeskManagementAgent",0x10000);if(s==IntPtr.Zero)throw new Exception("OpenDelete "+Marshal.GetLastWin32Error());try{if(!DeleteService(s))throw new Exception("Delete "+Marshal.GetLastWin32Error());}finally{CloseServiceHandle(s);}}finally{CloseServiceHandle(m);}}
 public static void Restore(string path,string display,uint type,uint start,uint err){IntPtr m=OpenSCManager(null,null,3);if(m==IntPtr.Zero)throw new Exception("SCMCreate "+Marshal.GetLastWin32Error());try{IntPtr s=CreateService(m,"RustDeskManagementAgent",display,0xF01FF,type,start,err,path,null,IntPtr.Zero,null,null,null);if(s==IntPtr.Zero)throw new Exception("CreateRestore "+Marshal.GetLastWin32Error());CloseServiceHandle(s);}finally{CloseServiceHandle(m);}}
}
'@
$ownedMarker=$false;$deleted=$false;$oldLog=$null;$before=$null
try{
    if($Mode -eq 'Prepare'){
        $r.health=Health 'baseline' 2026100502;$r.scm=Backup-Service 'baseline'
        $r.rescue_present=Test-Path 'C:\ProgramData\RustDesk\ManagementAgent\rescue'
        $r.rescue_inventory=@();if($r.rescue_present){$r.rescue_inventory=@(Get-ChildItem 'C:\ProgramData\RustDesk\ManagementAgent\rescue' -Force|Select-Object Name,Mode)}
        foreach($m in @('disable-agent','disable-recovery','maintenance.json')){Assert (!(Test-Path "$state\$m")) 'EXISTING_LOCAL_GATE'}
        $r.state='prepared'
    }elseif($Mode -eq 'Install'){
        $before=Json "$lab\bootstrap-v213-Prepare.result.json";Assert ($before.state -eq 'prepared') 'BASELINE_NOT_PREPARED'
        $artifact=Json "$lab\bootstrap-v213-artifacts.json"
        $msi="$lab\agent-standard-v213.msi";Assert ((Hash $msi) -eq $artifact.package.sha256) 'MSI_HASH'
        $r.before=$before.health;$r.package_hash=Hash $msi
        $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "/i $msi /qn /norestart /L*v $prefix.msi.log" -PassThru
        $null=$p.Handle;$r.installer=@{pid=$p.Id;created=[string]$p.StartTime.ToUniversalTime().ToFileTimeUtc()}
        Assert ($p.WaitForExit(900000)) 'MSI_TIMEOUT';$p.WaitForExit();$r.installer.exit=$p.ExitCode;Assert ($p.ExitCode -eq 0) 'MSI_INSTALL_FAILED'
        $r.after=Health 'after' 2026100504
        Assert ($r.after.hashes.agent -eq $artifact.agent.sha256 -and $r.after.hashes.dll -eq $artifact.dll.sha256) 'NEW_PAYLOAD_NOT_INSTALLED'
        Assert ($r.after.task -ceq $before.health.task) 'ORIGINAL_TASK_CHANGED'
        foreach($key in @('client_id','client_uuid','agent_installation_id')){Assert ($r.after.status.$key -eq $before.health.status.$key) 'UPGRADE_IDENTITY_CHANGED'}
        $r.state='pass'
    }else{
        $install=Json "$lab\bootstrap-v213-Install.result.json";Assert ($install.state -eq 'pass') 'UPGRADE_NOT_PASSED'
        $r.before=Health 'cycle-before' 2026100504;$before=Backup-Service 'cycle';$r.scm=$before
        Assert ($r.before.hashes.agent -eq $install.after.hashes.agent) 'AGENT_DRIFT'
        foreach($m in @('disable-agent','disable-recovery','maintenance.json')){Assert (!(Test-Path "$state\$m")) 'EXISTING_LOCAL_GATE'}
        [xml]$xml=$r.before.task;$ns=New-Object Xml.XmlNamespaceManager($xml.NameTable);$ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
        Assert ($xml.SelectSingleNode('//t:Repetition/t:Interval',$ns).InnerText -eq 'PT5M') 'PERIOD_CHANGED'
        Assert ($xml.SelectSingleNode('//t:Exec/t:Command',$ns).InnerText.Trim('"') -eq $boot) 'TASK_ACTION_CHANGED'
        $oldLog=(Get-WinEvent -ListLog $channel).IsEnabled;$r.original_log_enabled=$oldLog
        if(!$oldLog){& "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:true;Assert ($LASTEXITCODE -eq 0) 'LOG_ENABLE'}
        $marker="$state\disable-agent";$f=[IO.File]::Open($marker,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
        try{$b=[Text.Encoding]::ASCII.GetBytes('bootstrap-v213-owned-disable');$f.Write($b,0,$b.Length);$f.Flush($true)}finally{$f.Dispose()};$ownedMarker=$true
        $r.budget_original_text=if(Test-Path $budget){[IO.File]::ReadAllText($budget)}else{$null}
        Stop-Service $name;(Get-Service $name).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
        [AgentNative213]::Delete();$deleted=$true;$wait=[Diagnostics.Stopwatch]::StartNew()
        do{$code=[AgentNative213]::Observe();if($code -eq 1060){break};Assert ($code -in @(0,1072)) 'ABSENCE_UNKNOWN';Start-Sleep -Milliseconds 250}while($wait.Elapsed.TotalSeconds -lt 30)
        Assert ($code -eq 1060) 'DELETE_NOT_ABSENT';$r.absence_code=$code
        $floor=Last-Record;$r.disabled_floor=$floor;$w=[Diagnostics.Stopwatch]::StartNew();$seen=$null
        do{
            Assert ([AgentNative213]::Observe() -eq 1060) 'DISABLE_GATE_BYPASSED'
            $seen=Completed-Periodic @(Events $floor)
            Save @{stage='disabled_wait';elapsed=$w.Elapsed.TotalSeconds;service_code=1060} "$prefix.progress.json"
            if($seen){break};Start-Sleep -Seconds 2
        }while($w.Elapsed.TotalSeconds -lt 660)
        Assert ($null -ne $seen) 'DISABLED_PERIOD_NOT_OBSERVED';$r.disabled_period=$seen;$r.disabled_seconds=$w.Elapsed.TotalSeconds
        $r.disabled_events=@(Events $floor)
        Assert (([IO.File]::ReadAllText($marker)) -eq 'bootstrap-v213-owned-disable') 'MARKER_CHANGED'
        $actualBudget=if(Test-Path $budget){[IO.File]::ReadAllText($budget)}else{$null};Assert ($actualBudget -ceq $r.budget_original_text) 'DISABLED_BUDGET_CHANGED'
        Remove-Item -LiteralPath $marker;$ownedMarker=$false
        $floor=Last-Record;$r.recovery_floor=$floor;$r.system_floor=(Get-WinEvent -LogName System -MaxEvents 1).RecordId;$w=[Diagnostics.Stopwatch]::StartNew();$ready=$false
        $priorFailures=if($r.budget_original_text){(ConvertFrom-Json -InputObject $r.budget_original_text).failures}else{0}
        $deadline=660
        if($r.budget_original_text){$remaining=[math]::Max(0,($r.budget_original_text|ConvertFrom-Json).next_at-[DateTimeOffset]::UtcNow.ToUnixTimeSeconds());$deadline=[math]::Max(660,$remaining+660)}
        $r.recovery_window_seconds=$deadline
        do{
            $code=[AgentNative213]::Observe();Assert ($code -in @(0,1060)) 'RECOVERY_OBSERVATION_UNKNOWN'
            $s=Get-Service $name -ErrorAction SilentlyContinue
            $ev=@(Events $floor);$done=Completed-Periodic $ev
            if($s -and $s.Status -eq 'Running' -and $done -and (Test-Path $budget)){
                $b=Json $budget;if($b.failures -eq $priorFailures+1){$ready=$true;break}
            }
            Save @{stage='recovery_wait';elapsed=$w.Elapsed.TotalSeconds;service_code=$code;window=$deadline} "$prefix.progress.json"
            Start-Sleep -Seconds 2
        }while($w.Elapsed.TotalSeconds -lt $deadline)
        Assert $ready 'PRODUCTION_PERIODIC_REPAIR_TIMEOUT';$r.recovery_seconds=$w.Elapsed.TotalSeconds;$r.recovery_events=$ev;$r.recovery_period=$done;$r.budget_after=Json $budget
        $r.service_created_events=@(Get-WinEvent -LogName System -FilterXPath "*[System[EventID=7045 and EventRecordID > $($r.system_floor)]]"|ForEach-Object {$_.ToXml()}|Where-Object {$_ -like '*RustDesk Management Agent*' -and $_ -like '*rustdesk-management-agent.exe*'})
        Assert ($r.service_created_events.Count -eq 1) 'SERVICE_CREATION_EVENT_NOT_UNIQUE'
        $r.after=Health 'cycle-after' 2026100504
        Assert ($r.after.task -ceq $r.before.task) 'TASK_DEFINITION_CHANGED'
        foreach($key in @('client_id','client_uuid','agent_installation_id')){Assert ($r.after.status.$key -eq $r.before.status.$key) 'REPAIR_IDENTITY_CHANGED'}
        foreach($key in @('agent','bootstrap','dll','manifest')){Assert ($r.after.hashes[$key] -eq $r.before.hashes[$key]) 'REPAIR_HASH_CHANGED'}
        Assert ($r.after.status.application.pid -eq $r.before.status.application.pid -and $r.after.status.application.process_created_at -eq $r.before.status.application.process_created_at) 'MAIN_PROCESS_CHANGED'
        $r.scm_after=Backup-Service 'after';Assert ($r.scm_after.sddl -ceq $before.sddl) 'SERVICE_SDDL_CHANGED'
        foreach($key in @('Type','Start','ErrorControl','ObjectName','DisplayName','Description')){Assert ($r.scm_after.registry[$key] -eq $before.registry[$key]) 'SCM_CONTRACT_CHANGED'}
        $r.state='pass'
    }
}catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
finally{
    if($Mode -eq 'Cycle'){
        if($ownedMarker -and (Test-Path "$state\disable-agent") -and [IO.File]::ReadAllText("$state\disable-agent") -eq 'bootstrap-v213-owned-disable'){Remove-Item -LiteralPath "$state\disable-agent";$ownedMarker=$false}
        if($r.state -ne 'pass' -and $deleted){
            try{
                $code=[AgentNative213]::Observe();if($code -eq 1060){$reg=$before.registry;[AgentNative213]::Restore($reg.ImagePath,$reg.DisplayName,$reg.Type,$reg.Start,$reg.ErrorControl);& "$env:SystemRoot\System32\reg.exe" import "$prefix.cycle.scm.reg"|Out-Null;Assert ($LASTEXITCODE -eq 0) 'RESTORE_REG';& "$env:SystemRoot\System32\sc.exe" sdset $name $before.sddl|Out-Null;Assert ($LASTEXITCODE -eq 0) 'RESTORE_SDDL'}elseif($code -ne 0){throw 'RESTORE_ABSENCE_UNKNOWN'}
                Start-Service $name;(Get-Service $name).WaitForStatus('Running',[TimeSpan]::FromSeconds(30));$r.recovery_fallback=Health 'failure-restored' 2026100504;$r.manually_restored_on_failure=$true
            }catch{$r.restore_error=$_.Exception.Message}
        }
        if($null -ne $oldLog -and !$oldLog){& "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:false;$r.log_restored=($LASTEXITCODE -eq 0)}
    }
    $r.elapsed_seconds=$clock.Elapsed.TotalSeconds;$r.finished=[DateTime]::UtcNow.ToString('o');Save $r "$prefix.result.json"
    if($r.state -in @('pass','prepared')){Unregister-ScheduledTask "RustDeskAgentLabBootstrapProd-v213-$Mode" -Confirm:$false}
}
