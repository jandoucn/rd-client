param([ValidateSet('Prepare','Run')][string]$Mode)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'WIN10_SYSTEM_REQUIRED: set RUSTDESK_LAB_HOST to this COMPUTERNAME (RUSTDESK_LAB_OS unset or win10) and run as SYSTEM'}
$lab='C:\RustDeskAgentLab';$prefix="$lab\standard-v214-$Mode"
if(Test-Path "$prefix.result.json"){throw 'RUN_USED'}
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$state='C:\ProgramData\RustDesk\ManagementAgent\state'
$bootRoot='C:\ProgramData\RustDesk\RecoveryBootstrap'
$configRoot='C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config'
$intent="$configRoot\RustDesk2.toml"
$backup="$lab\standard-v214-protected-backup"
$r=@{state='unknown';mode=$Mode;errors=@();cases=@();remote_control='not_tested';production_api='not_tested';started=[DateTime]::UtcNow.ToString('o')}
$clock=[Diagnostics.Stopwatch]::StartNew();$ownedMarker=$null;$changedIntent=$false
function Assert([bool]$Condition,[string]$Code){if(!$Condition){throw $Code}}
function Hash([string]$Path){[string](Get-FileHash -LiteralPath $Path).Hash}
function Save($Value,[string]$Path){[IO.File]::WriteAllText($Path,($Value|ConvertTo-Json -Depth 16))}
function Agent-Owner {
    $s=Get-CimInstance Win32_Service|Where-Object Name -eq 'RustDeskManagementAgent'
    Assert ($s.State -eq 'Running') 'AGENT_NOT_RUNNING'
    $p=Get-Process -Id $s.ProcessId
    @{pid=$p.Id;created=[string]$p.StartTime.ToUniversalTime().ToFileTimeUtc();path=$p.Path}
}
function Call-Agent([string]$Command,[string]$Label){
    $p=Start-Process $agent -ArgumentList "$Command --edition standard --package-kind msi" -PassThru -RedirectStandardOutput "$prefix.$Label.stdout.txt" -RedirectStandardError "$prefix.$Label.stderr.txt"
    $null=$p.Handle;Assert ($p.WaitForExit(30000)) 'CLI_TIMEOUT';$p.WaitForExit()
    @{exit=$p.ExitCode;stdout=[IO.File]::ReadAllText("$prefix.$Label.stdout.txt");stderr=[IO.File]::ReadAllText("$prefix.$Label.stderr.txt")}
}
function Status([string]$Label){
    $p=Call-Agent 'status --json' $Label;Assert ($p.exit -eq 0) 'STATUS_EXIT'
    $s=$p.stdout|ConvertFrom-Json
    Assert ($s.agent.build_seq -eq 2026100504 -and $s.agent_installation_id -eq '76b922ac75fb14f19783bb6d48c45653' -and $s.package_identity.edition -eq 'standard' -and $s.package_identity.kind -eq 'msi') 'STATUS_IDENTITY_BUILD'
    $s
}
function Healthy([string]$Label){
    $s=Status $Label
    Assert ($s.application.state -eq 'ready' -and $s.application.build_seq -eq 2026100504 -and $s.intent.state -eq 'enabled' -and $s.recovery.effective_enabled) 'LOCAL_HEALTH_FAILED'
    $services=@(Get-CimInstance Win32_Service|Where-Object Name -in @('RustDesk','RustDeskManagementAgent')|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
    Assert ($services.Count -eq 2 -and @($services|Where-Object {$_.State -ne 'Running' -or $_.StartName -ne 'LocalSystem' -or $_.StartMode -ne 'Auto'}).Count -eq 0) 'SCM_CONTRACT'
    @{status=$s;services=$services;agent_owner=(Agent-Owner);task=(Export-ScheduledTask RustDeskManagementAgentRecovery);hashes=@{agent=(Hash $agent);dll=(Hash 'C:\Program Files\RustDesk\librustdesk.dll');bootstrap=(Hash "$bootRoot\rustdesk-recovery-bootstrap.exe");manifest=(Hash "$bootRoot\bootstrap.json")};bootstrap_budget=[IO.File]::ReadAllText("$bootRoot\budget.json")}
}
function Check-Agent {
    $current=Agent-Owner
    Assert ($current.pid -eq $baseline.agent_owner.pid -and $current.created -eq $baseline.agent_owner.created) 'AGENT_PROCESS_CHANGED'
}
function Observe-Stopped([string]$Label){
    $watch=[Diagnostics.Stopwatch]::StartNew();$samples=@()
    do{
        Check-Agent
        $s=Get-Service RustDesk;Assert ($s.Status -eq 'Stopped') "GATE_BYPASSED_$Label"
        $samples+=@{elapsed_ms=$watch.ElapsedMilliseconds;state=[string]$s.Status}
        Start-Sleep -Seconds 5
    }while($watch.Elapsed.TotalSeconds -lt 75)
    $samples
}
function Wait-Recovery([string]$Label,[int]$OldPid){
    $budget=[IO.File]::ReadAllText("$state\agent.json")|ConvertFrom-Json
    $remaining=[math]::Max(0,[int64]$budget.service_recovery_next_at-[DateTimeOffset]::UtcNow.ToUnixTimeSeconds())
    $window=[math]::Max(420,$remaining+240)
    $watch=[Diagnostics.Stopwatch]::StartNew();$samples=@()
    do{
        Check-Agent
        $s=Get-CimInstance Win32_Service|Where-Object Name -eq 'RustDesk'
        $samples+=@{elapsed_ms=$watch.ElapsedMilliseconds;state=[string]$s.State;pid=[int]$s.ProcessId}
        Save @{stage=$Label;elapsed=$watch.Elapsed.TotalSeconds;window=$window;main_state=[string]$s.State;main_pid=[int]$s.ProcessId} "$prefix.progress.json"
        if($s.State -eq 'Running'){
            $status=Status "$Label-ready"
            if($status.application.state -eq 'ready'){
                Assert ($s.ProcessId -ne $OldPid) 'MAIN_NOT_RESTARTED'
                Assert ($status.application.build_seq -eq 2026100504 -and $status.intent.state -eq 'enabled') 'RECOVERED_WRONG_BUILD_OR_INTENT'
                return @{seconds=$watch.Elapsed.TotalSeconds;window=$window;samples=$samples;status=$status;old_pid=$OldPid;new_pid=[int]$s.ProcessId;manual_start=$false}
            }
        }
        Start-Sleep -Seconds 2
    }while($watch.Elapsed.TotalSeconds -lt $window)
    throw "R1_RECOVERY_TIMEOUT_$Label"
}
function Write-Intent([string]$Value){
    $bytes=[IO.File]::ReadAllBytes("$backup\files\RustDesk2.toml")
    $text=[Text.Encoding]::UTF8.GetString($bytes)
    $section=[regex]::Match($text,'(?ms)^\[options\][^\r\n]*\r?\n.*?(?=^\[|\z)')
    Assert $section.Success 'OPTIONS_SECTION_MISSING'
    $pattern='(?m)^\s*(?:stop-service|"stop-service"|''stop-service'')\s*=.*$'
    Assert ([regex]::Matches($section.Value,$pattern).Count -le 1) 'AMBIGUOUS_STOP_KEY'
    $replacement="stop-service = '$Value'"
    $newSection=if([regex]::IsMatch($section.Value,$pattern)){[regex]::Replace($section.Value,$pattern,$replacement)}else{$section.Value+$replacement+"`r`n"}
    $changed=$text.Substring(0,$section.Index)+$newSection+$text.Substring($section.Index+$section.Length)
    [IO.File]::WriteAllBytes($intent,[Text.Encoding]::UTF8.GetBytes($changed))
}
try{
    if($Mode -eq 'Prepare'){
        $r.baseline=Healthy 'baseline'
        Assert ($r.baseline.hashes.agent -eq '5716185c0099d3c9c4964c28705b69d61e5475934388d57de35153809ef8c2b8' -and $r.baseline.hashes.bootstrap -eq $r.baseline.hashes.agent -and $r.baseline.hashes.dll -eq '7d49190d41bd5f7b5f63b9c4a69ac82dace58dd1445f30fe67a442581e5f8984') 'PAYLOAD_NOT_V213_STANDARD'
        $r.versions=@()
        foreach($path in @($agent,"$bootRoot\rustdesk-recovery-bootstrap.exe",'C:\Program Files\RustDesk\librustdesk.dll','C:\Program Files\RustDesk\RustDesk.exe')){
            $v=[Diagnostics.FileVersionInfo]::GetVersionInfo($path)
            $r.versions+=@{path=$path;file_version=$v.FileVersion;product_version=$v.ProductVersion;major=$v.FileMajorPart;minor=$v.FileMinorPart;build=$v.FileBuildPart;private=$v.FilePrivatePart}
        }
        $v=$r.versions[0];Assert ($v.file_version -eq '1.5.3+2026100504' -and (([int64]$v.build -shl 16)+$v.private) -eq 2026100504) 'AGENT_RESOURCE_VERSION'
        Assert (!(Test-Path $backup)) 'BACKUP_EXISTS'
        New-Item -ItemType Directory $backup|Out-Null
        & "$env:SystemRoot\System32\icacls.exe" $backup /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F'|Out-Null;Assert ($LASTEXITCODE -eq 0) 'BACKUP_ACL'
        Copy-Item -LiteralPath $configRoot -Destination "$backup\files" -Recurse
        $r.config_inventory=@(Get-ChildItem -LiteralPath $configRoot -Force -Recurse -File|ForEach-Object {@{relative=$_.FullName.Substring($configRoot.Length+1);hash=(Hash $_.FullName);sddl=(Get-Acl -LiteralPath $_.FullName).Sddl}})
        $r.config_root=$configRoot;$r.protected_backup=$backup
        $r.registered=@(Get-ChildItem 'HKLM:\SOFTWARE\Classes\Installer\Products'|ForEach-Object {$n=[string]$_.GetValue('ProductName');if($n -eq 'RustDesk'){$s=Get-Item ($_.PSPath+'\SourceList');@{package=[string]$s.GetValue('PackageName');source=[string]$s.GetValue('LastUsedSource')}}})
        $r.msi_sha256=Hash "$lab\agent-standard-v213.msi"
        $r.capabilities=$r.baseline.status.capabilities;$r.state='prepared'
    }else{
        $prepared=[IO.File]::ReadAllText("$lab\standard-v214-Prepare.result.json")|ConvertFrom-Json
        Assert ($prepared.state -eq 'prepared') 'PREPARE_FAILED';$baseline=$prepared.baseline
        $r.before=Healthy 'before';Check-Agent
        foreach($marker in @('disable-agent','disable-recovery','maintenance.json')){Assert (!(Test-Path "$state\$marker")) 'PREEXISTING_GATE'}
        $elapsed=([DateTime]::UtcNow-(Get-Process -Id $baseline.agent_owner.pid).StartTime.ToUniversalTime()).TotalSeconds
        Assert ($elapsed -ge 180) 'AGENT_STARTUP_GRACE_NOT_OVER'
        foreach($marker in @('disable-agent','disable-recovery','maintenance.json')){
            $ownedMarker="$state\$marker";$f=[IO.File]::Open($ownedMarker,[IO.FileMode]::CreateNew);$f.Dispose()
            $p=Call-Agent 'check-now' "deny-$marker"
            Assert ($p.exit -eq 1 -and $p.stderr -like '*package recovery locally blocked*') 'CHECK_NOW_GATE_FAILED'
            Check-Agent;Assert ((Get-Service RustDesk).Status -eq 'Running') 'GATE_STOPPED_MAIN'
            Remove-Item -LiteralPath $ownedMarker;$ownedMarker=$null
            $r.cases+=@{name="check-now-$marker";state='pass';exit=$p.exit;error=$p.stderr}
        }
        # Stop one: local prohibition must hold; release lets the installed agent perform R1.
        $ownedMarker="$state\disable-recovery";$f=[IO.File]::Open($ownedMarker,[IO.FileMode]::CreateNew);$f.Dispose()
        $oldPid=(Get-CimInstance Win32_Service|Where-Object Name -eq 'RustDesk').ProcessId
        Stop-Service RustDesk;(Get-Service RustDesk).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
        $samples=Observe-Stopped 'local-disable'
        $blocked=Status 'local-disable-status';Assert (!$blocked.recovery.effective_enabled -and $blocked.service.state -eq 'stopped') 'LOCAL_DISABLE_STATUS_FALSE_SUCCESS'
        Remove-Item -LiteralPath $ownedMarker;$ownedMarker=$null
        $recovered=Wait-Recovery 'R1-first' $oldPid
        $r.cases+=@{name='first-normal-stop-local-disable-then-R1';state='pass';blocked_samples=$samples;recovery=$recovered}
        Save $r "$prefix.progress-result.json"
        # Stop two: stage under a local gate, persist user intent after service has stopped.
        $ownedMarker="$state\disable-recovery";$f=[IO.File]::Open($ownedMarker,[IO.FileMode]::CreateNew);$f.Dispose()
        $oldPid=(Get-CimInstance Win32_Service|Where-Object Name -eq 'RustDesk').ProcessId
        Stop-Service RustDesk;(Get-Service RustDesk).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
        Write-Intent 'Y';$changedIntent=$true
        $intentHash=Hash $intent
        Remove-Item -LiteralPath $ownedMarker;$ownedMarker=$null
        $stopped=Status 'user-stop';Assert ($stopped.intent.state -eq 'disabled' -and !$stopped.recovery.effective_enabled) 'USER_INTENT_NOT_READ'
        $deny=Call-Agent 'check-now' 'user-stop-denied';Assert ($deny.exit -eq 1 -and $deny.stderr -like '*user stop intent*') 'USER_STOP_CHECK_NOW_NOT_DENIED'
        $samples=Observe-Stopped 'user-stop';Assert ((Hash $intent) -eq $intentHash) 'USER_STOP_CONFIG_OVERWRITTEN'
        [IO.File]::WriteAllBytes($intent,[IO.File]::ReadAllBytes("$backup\files\RustDesk2.toml"));$changedIntent=$false
        $recovered=Wait-Recovery 'R1-second' $oldPid
        $r.cases+=@{name='second-normal-stop-user-intent-then-R1';state='pass';blocked_samples=$samples;recovery=$recovered;config_restored=$true}
        $r.after=Healthy 'final';Check-Agent
        Assert ($r.after.bootstrap_budget -ceq $baseline.bootstrap_budget) 'BOOTSTRAP_BUDGET_CHANGED'
        Assert ($r.after.task -ceq $baseline.task) 'RECOVERY_TASK_CHANGED'
        foreach($key in @('agent','bootstrap','dll','manifest')){Assert ($r.after.hashes[$key] -eq $baseline.hashes.$key) 'PAYLOAD_CHANGED'}
        foreach($key in @('client_id','client_uuid','agent_installation_id')){Assert ($r.after.status.$key -eq $baseline.status.$key) 'IDENTITY_CHANGED'}
        $r.config_after=@()
        foreach($f in $prepared.config_inventory){
            $path=Join-Path $configRoot $f.relative
            $r.config_after+=@{relative=$f.relative;hash=(Hash $path);sddl=(Get-Acl $path).Sddl}
            Assert ((Hash $path) -eq $f.hash -and (Get-Acl $path).Sddl -eq $f.sddl) 'WHOLE_CONFIG_OR_ACL_CHANGED'
        }
        $r.state='pass'
    }
}catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
finally{
    if($changedIntent){[IO.File]::WriteAllBytes($intent,[IO.File]::ReadAllBytes("$backup\files\RustDesk2.toml"));$r.emergency_config_restored=$true}
    if($ownedMarker -and (Test-Path $ownedMarker)){Remove-Item -LiteralPath $ownedMarker}
    if($Mode -eq 'Run' -and $r.state -eq 'fail'){
        try{if((Get-Service RustDesk).Status -ne 'Running'){Start-Service RustDesk;(Get-Service RustDesk).WaitForStatus('Running',[TimeSpan]::FromSeconds(30));$r.manual_failure_restore=$true};$r.restored=Healthy 'failure-restored'}catch{$r.restore_error=$_.Exception.Message}
    }
    $r.case_count=$r.cases.Count;$r.elapsed_seconds=$clock.Elapsed.TotalSeconds;$r.finished=[DateTime]::UtcNow.ToString('o')
    Save $r "$prefix.result.json"
    Unregister-ScheduledTask "RustDeskAgentLabStandard-v214-$Mode" -Confirm:$false
}
