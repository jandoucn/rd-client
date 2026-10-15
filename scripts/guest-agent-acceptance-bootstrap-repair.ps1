param([ValidateSet('Prepare','Repair')][string]$Mode,
    [ValidateSet('v210','v211')][string]$Round='v210',
    [ValidatePattern('^[a-fA-F0-9]{64}$')][string]$TestSha256='374ddb43a901e1579c6cd0c20848f9c07e13f043fe0044b0e4a6ca139715a8fc')
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if($env:COMPUTERNAME -ne 'RDLAB-WIN10' -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18' -or (Get-CimInstance Win32_ComputerSystemProduct).UUID -ne 'a6c9edff-6159-468f-bdba-08720a153424'){throw 'WIN10_SYSTEM_REQUIRED'}
$lab='C:\RustDeskAgentLab';$prefix="$lab\bootstrap-$Round-repair"
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$bootstrap='C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe'
$state='C:\ProgramData\RustDesk\ManagementAgent\state'
$budget='C:\ProgramData\RustDesk\RecoveryBootstrap\budget.json'
$testExe="$lab\bootstrap-$Round-agent.exe"
$serviceName='RustDeskManagementAgent'
$script:checks=0
function Assert([bool]$Value,[string]$Message){$script:checks++;if(!$Value){throw $Message}}
function Hash([string]$Path){[string](Get-FileHash -LiteralPath $Path).Hash}
function Save($Value,[string]$Path){
    $f=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try{$b=[Text.Encoding]::UTF8.GetBytes(($Value|ConvertTo-Json -Depth 12));$f.Write($b,0,$b.Length);$f.Flush($true)}finally{$f.Dispose()}
}
function Health([string]$Label){
    $p=Start-Process $agent -ArgumentList 'status --json --edition standard --package-kind msi' -PassThru -RedirectStandardOutput "$prefix.$Label.status.json" -RedirectStandardError "$prefix.$Label.stderr.txt"
    $null=$p.Handle;Assert ($p.WaitForExit(30000)) 'STATUS_TIMEOUT';$p.WaitForExit();Assert ($p.ExitCode -eq 0) 'STATUS_EXIT'
    $s=[IO.File]::ReadAllText("$prefix.$Label.status.json")|ConvertFrom-Json
    Assert ($s.application.state -eq 'ready' -and $s.agent.build_seq -eq 2026100502 -and $s.application.build_seq -eq 2026100502) 'HEALTH_BUILD'
    Assert ($s.agent_installation_id -eq '76b922ac75fb14f19783bb6d48c45653' -and $s.package_identity.edition -eq 'standard') 'IDENTITY_EDITION'
    $services=@(Get-CimInstance Win32_Service|Where-Object Name -in @('RustDesk','RustDeskManagementAgent')|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
    Assert ($services.Count -eq 2 -and @($services|Where-Object {$_.State -ne 'Running' -or $_.StartMode -ne 'Auto' -or $_.StartName -ne 'LocalSystem'}).Count -eq 0) 'SERVICE_BASELINE'
    $hashes=@{agent=(Hash $agent);bootstrap=(Hash $bootstrap);dll=(Hash 'C:\Program Files\RustDesk\librustdesk.dll');manifest=(Hash 'C:\ProgramData\RustDesk\RecoveryBootstrap\bootstrap.json')}
    Assert ($hashes.agent -eq '00866e366c7cc222b39b4a4b34d8ab3938dd1c7dfb23cc00b76982ecdd155c1b' -and $hashes.bootstrap -eq $hashes.agent -and $hashes.dll -eq 'f131d0a1857a177e61a82bf978a973fec8e64edbd592311a8f08d180b53d1bb5') 'INSTALLED_HASH'
    @{status=$s;services=$services;hashes=$hashes;task=(Export-ScheduledTask RustDeskManagementAgentRecovery)}
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class FixedAgentScm {
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenSCManager(string m,string d,uint a);
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr OpenService(IntPtr m,string n,uint a);
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateService(IntPtr m,string n,string display,uint access,uint type,uint start,uint error,string path,string group,IntPtr tag,string deps,string account,string password);
 [DllImport("advapi32.dll",SetLastError=true)] static extern bool DeleteService(IntPtr s);
 [DllImport("advapi32.dll")] static extern bool CloseServiceHandle(IntPtr s);
 public static int Observe() {IntPtr m=OpenSCManager(null,null,1);if(m==IntPtr.Zero)return Marshal.GetLastWin32Error();try{IntPtr s=OpenService(m,"RustDeskManagementAgent",1);if(s==IntPtr.Zero)return Marshal.GetLastWin32Error();CloseServiceHandle(s);return 0;}finally{CloseServiceHandle(m);}}
 public static void Delete() {IntPtr m=OpenSCManager(null,null,1);if(m==IntPtr.Zero)throw new Exception("SCM "+Marshal.GetLastWin32Error());try{IntPtr s=OpenService(m,"RustDeskManagementAgent",0x10000);if(s==IntPtr.Zero)throw new Exception("OpenDelete "+Marshal.GetLastWin32Error());try{if(!DeleteService(s))throw new Exception("Delete "+Marshal.GetLastWin32Error());}finally{CloseServiceHandle(s);}}finally{CloseServiceHandle(m);}}
 public static void Restore(string path,string display,uint type,uint start,uint error,string deps) {IntPtr m=OpenSCManager(null,null,3);if(m==IntPtr.Zero)throw new Exception("SCMCreate "+Marshal.GetLastWin32Error());try{IntPtr s=CreateService(m,"RustDeskManagementAgent",display,0xF01FF,type,start,error,path,null,IntPtr.Zero,deps,null,null);if(s==IntPtr.Zero)throw new Exception("CreateRestore "+Marshal.GetLastWin32Error());CloseServiceHandle(s);}finally{CloseServiceHandle(m);}}
}
'@
function Wait-Absent {
    $w=[Diagnostics.Stopwatch]::StartNew()
    do{$code=[FixedAgentScm]::Observe();if($code -eq 1060){return};if($code -notin @(0,1072)){throw "ABSENCE_UNKNOWN_$code"};Start-Sleep -Milliseconds 250}while($w.Elapsed.TotalSeconds -lt 30)
    throw 'SERVICE_DELETE_TIMEOUT'
}
function Invoke-Bootstrap([string]$Label){
    $env:RUSTDESK_BOOTSTRAP_REPAIR_LAB='v210'
    $test='management_agent::bootstrap::tests::fixed_installed_bootstrap_repair_entry'
    $p=Start-Process $testExe -ArgumentList "--ignored --exact $test --nocapture --test-threads=1" -PassThru -RedirectStandardOutput "$prefix.$Label.test.stdout.txt" -RedirectStandardError "$prefix.$Label.test.stderr.txt"
    $null=$p.Handle;$watch=[Diagnostics.Stopwatch]::StartNew()
    Assert ($p.WaitForExit(60000)) 'BOOTSTRAP_TIMEOUT';$p.WaitForExit()
    $proof=@{label=$Label;pid=$p.Id;exit=$p.ExitCode;elapsed_ms=$watch.ElapsedMilliseconds}
    Save $proof "$prefix.$Label.test.json"
    Assert ($p.ExitCode -eq 0) "BOOTSTRAP_FAILED_$Label"
    Assert ([IO.File]::ReadAllText("$prefix.$Label.test.stdout.txt") -match 'test result: ok\. 1 passed; 0 failed; 0 ignored;') 'ZERO_TEST'
}
if($Mode -eq 'Prepare'){
    $r=@{state='unknown';errors=@()}
    try{
        $r.health=Health 'baseline'
        foreach($marker in @('disable-agent','disable-recovery','maintenance.json')){Assert (!(Test-Path "$state\$marker")) 'EXISTING_LOCAL_GATE'}
        $reg=Get-Item 'HKLM:\SYSTEM\CurrentControlSet\Services\RustDeskManagementAgent'
        $r.registry=@{};foreach($name in $reg.GetValueNames()){$r.registry[$name]=$reg.GetValue($name)}
        $r.sddl=((& "$env:SystemRoot\System32\sc.exe" sdshow $serviceName)|Where-Object {$_ -match '^[ODGS]:'}) -join ''
        Assert ($LASTEXITCODE -eq 0 -and $r.sddl.Length -gt 0) 'SECURITY_EXPORT'
        & "$env:SystemRoot\System32\reg.exe" export 'HKLM\SYSTEM\CurrentControlSet\Services\RustDeskManagementAgent' "$prefix.scm.reg" /y|Out-Null
        Assert ($LASTEXITCODE -eq 0) 'REGISTRY_EXPORT'
        foreach($op in @('qc','qdescription','qfailure','qfailureflag','qsidtype','qprivs','qtriggerinfo')){
            $data=& "$env:SystemRoot\System32\sc.exe" $op $serviceName
            Assert ($LASTEXITCODE -eq 0) "SCM_EXPORT_$op"
            [IO.File]::WriteAllText("$prefix.$op.txt",($data -join "`r`n"))
        }
        $r.budget_before=if(Test-Path $budget){[IO.File]::ReadAllText($budget)}else{$null}
        if($r.budget_before){Assert (([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()) -ge ($r.budget_before|ConvertFrom-Json).next_at) 'BUDGET_NOT_DUE'}
        $r.test_sha256=Hash $testExe;Assert ($r.test_sha256 -eq $TestSha256) 'TEST_HASH'
        $r.state='prepared'
    }catch{$r.errors+=$_.Exception.Message}
    Save $r "$prefix.prepare.json"
}else{
    $r=@{state='unknown';errors=@();deleted=$false;restored_on_failure=$false}
    $before=[IO.File]::ReadAllText("$prefix.prepare.json")|ConvertFrom-Json
    Assert ($before.state -eq 'prepared') 'PREPARATION_FAILED'
    $ownedMarkers=New-Object System.Collections.ArrayList
    try{
        Assert ((Hash $testExe) -eq $before.test_sha256) 'TEST_EXE_CHANGED'
        # Stop with Disabled first so the installed periodic bootstrap cannot race the planned deletion.
        Set-Service $serviceName -StartupType Disabled
        Stop-Service $serviceName
        (Get-Service $serviceName).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
        Invoke-Bootstrap 'disabled'
        Assert ((Get-Service $serviceName).Status -eq 'Stopped') 'DISABLED_STARTED'
        Assert ((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\RustDeskManagementAgent').Start -eq 4) 'DISABLED_CHANGED'
        [FixedAgentScm]::Delete();Wait-Absent;$r.deleted=$true
        Save @{code=1060;when=[DateTime]::UtcNow.ToString('o')} "$prefix.absent.json"
        foreach($name in @('disable-agent','disable-recovery','maintenance.json')){
            $path="$state\$name";$f=[IO.File]::Open($path,[IO.FileMode]::CreateNew);$f.Dispose();[void]$ownedMarkers.Add($path)
            Invoke-Bootstrap $name
            Assert ([FixedAgentScm]::Observe() -eq 1060) "LOCAL_GATE_RECREATED_$name"
            Remove-Item -LiteralPath $path;[void]$ownedMarkers.Remove($path)
        }
        Invoke-Bootstrap 'repair'
        Assert ([FixedAgentScm]::Observe() -eq 0) 'SERVICE_NOT_RECREATED'
        $r.after=Health 'after'
        foreach($field in @('client_id','client_uuid','agent_installation_id')){Assert ($r.after.status.$field -eq $before.health.status.$field) 'IDENTITY_CHANGED'}
        Assert ($r.after.task -ceq $before.health.task) 'TASK_CHANGED'
        foreach($key in $before.health.hashes.PSObject.Properties.Name){Assert ($r.after.hashes[$key] -eq $before.health.hashes.$key) 'HASH_CHANGED'}
        $current=Get-CimInstance Win32_Service|Where-Object Name -eq $serviceName
        Assert ($current.PathName -like '*--normal-install-path "C:\Program Files\RustDesk"*') 'QUOTING_CHANGED'
        $afterReg=Get-Item 'HKLM:\SYSTEM\CurrentControlSet\Services\RustDeskManagementAgent'
        $r.after_registry=@{};foreach($name in $afterReg.GetValueNames()){$r.after_registry[$name]=$afterReg.GetValue($name)}
        foreach($field in @('Type','Start','ErrorControl','ObjectName','DisplayName','Description')){
            Assert ($r.after_registry[$field] -eq $before.registry.$field) "SCM_CONTRACT_CHANGED_$field"
        }
        $r.budget_after=[IO.File]::ReadAllText($budget)|ConvertFrom-Json
        $previous=if($before.budget_before){($before.budget_before|ConvertFrom-Json).failures}else{0}
        Assert ($r.budget_after.failures -eq $previous+1) 'BUDGET_NOT_RECORDED'
        $r.state='pass'
    }catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
    finally{
        foreach($path in $ownedMarkers){Remove-Item -LiteralPath $path -Force}
        if($r.state -ne 'pass'){
            try{
                $code=[FixedAgentScm]::Observe()
                if($code -eq 0){Stop-Service $serviceName -ErrorAction Stop;[FixedAgentScm]::Delete();Wait-Absent}
                elseif($code -ne 1060){throw "RESTORE_UNCONFIRMED_$code"}
                $reg=$before.registry;$deps=$null
                if($reg.PSObject.Properties.Name -contains 'DependOnService'){$deps=([string[]]$reg.DependOnService -join "`0")+"`0`0"}
                [FixedAgentScm]::Restore($reg.ImagePath,$reg.DisplayName,[uint32]$reg.Type,[uint32]$reg.Start,[uint32]$reg.ErrorControl,$deps)
                & "$env:SystemRoot\System32\reg.exe" import "$prefix.scm.reg"|Out-Null;Assert ($LASTEXITCODE -eq 0) 'RESTORE_REGISTRY_FAILED'
                & "$env:SystemRoot\System32\sc.exe" sdset $serviceName $before.sddl|Out-Null;Assert ($LASTEXITCODE -eq 0) 'RESTORE_SD_FAILED'
                Start-Service $serviceName;(Get-Service $serviceName).WaitForStatus('Running',[TimeSpan]::FromSeconds(30))
                $r.restored_health=Health 'restored';$r.restored_on_failure=$true
            }catch{$r.restore_error=$_.Exception.Message}
        }
        $r.assertions=$script:checks;Save $r "$prefix.result.json"
    }
}
Unregister-ScheduledTask "RustDeskAgentLabBootstrap-$Round-$Mode" -Confirm:$false
