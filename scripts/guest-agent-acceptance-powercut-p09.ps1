param(
    [ValidateSet('Arm','Boot','Verify','Report')][string]$Mode,
    [ValidateSet('p09-v201','p09-v203','p09-v209')][string]$RunId='p09-v201',
    [string]$TransactionRoot,
    [string]$TestSha256
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if($env:COMPUTERNAME -ne 'RDLAB-WIN10' -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18' -or (Get-CimInstance Win32_ComputerSystemProduct).UUID -ne 'a6c9edff-6159-468f-bdba-08720a153424'){throw 'WIN10_SYSTEM_REQUIRED'}
$lab='C:\RustDeskAgentLab';$run=$RunId
$prefix="$lab\$run"
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$live='C:\ProgramData\RustDesk\ManagementAgent\state'
$channel='Microsoft-Windows-TaskScheduler/Operational'
$suffix=$run.Substring(4)
$bootTask="RustDeskAgentLabP09Boot-$suffix"
function Json([string]$Path){[IO.File]::ReadAllText($Path)|ConvertFrom-Json}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function Save($Value,[string]$Path){
    $bytes=[Text.Encoding]::UTF8.GetBytes(($Value|ConvertTo-Json -Depth 18))
    $f=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try {$f.Write($bytes,0,$bytes.Length);$f.Flush($true)}finally{$f.Dispose()}
}
function Health([string]$Label){
    $p=Start-Process $agent -ArgumentList 'status --json --edition standard --package-kind msi' -PassThru -RedirectStandardOutput "$prefix.$Label-status.json" -RedirectStandardError "$prefix.$Label-status.stderr.txt"
    $null=$p.Handle
    if(!$p.WaitForExit(30000)){throw 'STATUS_TIMEOUT'};$p.WaitForExit();if($p.ExitCode -ne 0){throw 'STATUS_FAILED'}
    $s=Json "$prefix.$Label-status.json"
    $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
    if($s.agent_installation_id -ne '76b922ac75fb14f19783bb6d48c45653' -or $s.package_identity.edition -ne 'standard' -or $s.application.state -ne 'ready' -or $s.application.build_seq -ne 2026100502 -or $s.agent.build_seq -ne 2026100502){throw 'WRONG_BASELINE_OR_UNHEALTHY'}
    if($services.Count -ne 2 -or @($services|Where-Object {$_.State -ne 'Running' -or $_.StartName -ne 'LocalSystem' -or $_.StartMode -ne 'Auto'}).Count){throw 'SERVICES_UNHEALTHY'}
    $hashes=@{agent=(Hash $agent);dll=(Hash 'C:\Program Files\RustDesk\librustdesk.dll');bootstrap=(Hash 'C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe')}
    if($hashes.agent -ne '00866e366c7cc222b39b4a4b34d8ab3938dd1c7dfb23cc00b76982ecdd155c1b' -or $hashes.bootstrap -ne $hashes.agent -or $hashes.dll -ne 'f131d0a1857a177e61a82bf978a973fec8e64edbd592311a8f08d180b53d1bb5'){throw 'BASELINE_HASH_MISMATCH'}
    @{status=$s;hashes=$hashes;services=$services;recovery_task=(Export-ScheduledTask RustDeskManagementAgentRecovery)}
}
function Check-Root([string]$Root){if($Root -notmatch '^C:\\RustDeskAgentLab\\local-command-[0-9]+\\RustDesk\\ManagementAgent\\transactions\\[a-f0-9]{32}$'){throw 'INVALID_TRANSACTION_ROOT'}}
function Export-Evidence([string]$Root,[string]$Destination){
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $files=@(Get-ChildItem -LiteralPath $lab -File -Filter "$run.*"|Where-Object Extension -ne '.zip')
    $files+=@(Get-ChildItem -LiteralPath $Root -Recurse -Force -File|Where-Object {$_.Extension -in @('.json','.log','.txt','.xml','.ps1') -and $_.Name -ne 'poll-fixture.json'})
    if($Mode -eq 'Arm'){$files=@($files|Where-Object Extension -in @('.json','.xml','.ps1'))}
    $zip=[IO.Compression.ZipFile]::Open($Destination,[IO.Compression.ZipArchiveMode]::Create)
    try {foreach($file in $files){[void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$file.FullName,$file.FullName.Substring($lab.Length+1).Replace('\','/'))}}finally{$zip.Dispose()}
}
if($Mode -eq 'Arm'){
    if(Test-Path "$prefix.arm.json"){throw 'RUN_ALREADY_USED'}
    $r=@{state='unknown';mode='Arm';run_id=$run;source='test-only bridge';errors=@()}
    try {
        $r.before=Health 'before'
        foreach($marker in @('maintenance.json','disable-agent','disable-recovery')){if(Test-Path "$live\$marker"){throw 'EXISTING_GATE'}}
        Save $r.before "$prefix.baseline.json"
        $exe="$lab\rustdesk_management_agent-$run.exe"
        if($TestSha256 -notmatch '^[a-fA-F0-9]{64}$' -or (Hash $exe) -ne $TestSha256){throw 'TEST_EXE_HASH_MISMATCH'}
        $r.test_sha256=Hash $exe;$r.driver_sha256=Hash $PSCommandPath
        $log=Get-WinEvent -ListLog $channel;$r.log_was_enabled=$log.IsEnabled
        if(!$log.IsEnabled){& "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:true;if($LASTEXITCODE -ne 0){throw 'LOG_ENABLE_FAILED'}}
        $env:RUSTDESK_LOCAL_COMMAND_LAB='v142';$env:RUSTDESK_P09_RUN=$run
        $env:AGENT_TEST_API_ROOT="$lab\api-tests\api";$env:AGENT_TEST_PHP="$lab\api-tests\php\php.exe";$env:AGENT_TEST_PHP_INI="$lab\api-tests\php-test.ini"
        $test='management_agent::operations_runtime::tests::local_api_mirrors_validation_failure_restores_real_msi_and_reports_once'
        $p=Start-Process $exe -ArgumentList "--ignored --exact $test --nocapture --test-threads=1" -PassThru -RedirectStandardOutput "$prefix.scenario.stdout.txt" -RedirectStandardError "$prefix.scenario.stderr.txt"
        $null=$p.Handle;$r.parent=@{pid=$p.Id;created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc()}
        $scene="$lab\local-command-$($p.Id)";$watch=[Diagnostics.Stopwatch]::StartNew();$barriers=@()
        while($watch.Elapsed.TotalSeconds -lt 900 -and !$p.HasExited){
            if(Test-Path "$scene\RustDesk\ManagementAgent\transactions"){$barriers=@(Get-ChildItem "$scene\RustDesk\ManagementAgent\transactions" -Recurse -Force -Filter 'p09-barrier.json');if($barriers.Count){break}}
            Start-Sleep -Milliseconds 500
        }
        if($barriers.Count -ne 1){throw 'BARRIER_NOT_REACHED'}
        $root=$barriers[0].DirectoryName;Check-Root $root
        $b=Json $barriers[0].FullName;$r.barrier=$b
        if($run -in @('p09-v203','p09-v209')){
            $r.identity_proof=Json "$scene\identity-persistence-proof.json"
            if($r.identity_proof.verified -ne $true -or $r.identity_proof.command -ne $b.transaction_id -or !(Test-Path "$scene\report-identity.json")){throw 'REPORT_IDENTITY_NOT_DURABLE'}
        }
        if($b.run_id -ne $run -or $b.phase -ne 'rolled_back' -or $b.root -ne $root){throw 'BARRIER_BINDING_FAILED'}
        foreach($pair in @(@('execution-result.json','receipt_sha256'),@('transaction.json','transaction_sha256'))){if((Hash "$root\$($pair[0])") -ne $b.($pair[1])){throw 'BARRIER_BYTES_CHANGED'}}
        if($run -eq 'p09-v209'){
            if(!$b.marker_released -or (Test-Path "$live\maintenance.json")){throw 'P09B_MARKER_NOT_RELEASED'}
        }elseif((Hash "$live\maintenance.json") -ne $b.maintenance_sha256){throw 'MARKER_MISMATCH'}
        $helper=Get-Process -Id $b.helper_owner.pid
        if($helper.StartTime.ToUniversalTime().ToFileTimeUtc() -ne $b.helper_owner.created_at){throw 'HELPER_PROCESS_CHANGED'}
        $originalTask="RustDeskManagementAgentResume-$($b.transaction_id)"
        $xml=Export-ScheduledTask $originalTask
        [IO.File]::WriteAllText("$prefix.original-task.xml",$xml)
        $r.barrier_health=Health 'barrier'
        Copy-Item -LiteralPath $PSCommandPath -Destination "$root\p09-boot.ps1"
        $bridgeHash=Hash "$root\p09-boot.ps1"
        $action=New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File $root\p09-boot.ps1 -Mode Boot -RunId $run -TransactionRoot $root"
        $principal=New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest
        $trigger=New-ScheduledTaskTrigger -AtStartup
        $settings=New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 10) -MultipleInstances IgnoreNew
        Register-ScheduledTask -TaskName $bootTask -Action $action -Principal $principal -Trigger $trigger -Settings $settings|Out-Null
        $r.bridge_task=Export-ScheduledTask $bootTask
        $r.original_task=$xml;$r.root=$root;$r.system_record_id=(Get-WinEvent -LogName System -MaxEvents 1).RecordId
        $r.task_record_id=(Get-WinEvent -LogName $channel -MaxEvents 1).RecordId
        $r.bridge_sha256=$bridgeHash;$r.state='armed';$r.barrier_observed_ms=$watch.ElapsedMilliseconds
        Save $r "$root\p09-context.json"
        Save $r "$prefix.arm.json"
        Export-Evidence $root "$prefix.precut.zip"
    }catch{$r.errors+=$_.Exception.Message;Save $r "$prefix.arm-failed.json";throw}
}elseif($Mode -eq 'Boot'){
    Check-Root $TransactionRoot;$root=$TransactionRoot
    $r=@{state='unknown';errors=@();backend_calls='unknown';scope='automatic test-only boot bridge'}
    try {
        $c=Json "$root\p09-context.json"
        if($c.root -ne $root -or $c.run_id -ne $run -or $c.state -ne 'armed' -or (Hash $PSCommandPath) -ne $c.bridge_sha256){throw 'CONTEXT_MISMATCH'}
        $r.system_boots=@(Get-WinEvent -LogName System -FilterXPath "*[System[EventID=12 and EventRecordID > $($c.system_record_id)]]"|Where-Object ProviderName -eq 'Microsoft-Windows-Kernel-General'|Select-Object Id,RecordId,ProviderName)
        if($r.system_boots.Count -ne 1){throw 'NEW_BOOT_NOT_PROVEN'}
        if((Export-ScheduledTask "RustDeskManagementAgentResume-$($c.barrier.transaction_id)") -cne $c.original_task){throw 'ORIGINAL_TASK_CHANGED'}
        $helper="$root\runtime\helper.exe"
        if((Hash $helper) -ne $c.barrier.helper_sha256){throw 'HELPER_HASH_CHANGED'}
        $env:RUSTDESK_P09_ROOT=$root
        $test='management_agent::operations_runtime::tests::p09_boot_bridge_resumes_terminal_without_backend'
        $p=Start-Process $helper -ArgumentList "--ignored --exact $test --nocapture --test-threads=1" -PassThru -RedirectStandardOutput "$prefix.resume.stdout.txt" -RedirectStandardError "$prefix.resume.stderr.txt"
        $null=$p.Handle;$r.resume_process=@{pid=$p.Id;created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc()}
        if(!$p.WaitForExit(300000)){throw 'RESUME_TIMEOUT'};$p.WaitForExit();$r.exit_code=$p.ExitCode
        if($p.ExitCode -ne 0 -or [IO.File]::ReadAllText("$prefix.resume.stdout.txt") -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored;'){throw 'RESUME_TEST_FAILED'}
        $r.proof=Json "$root\p09-resumed.json";$r.backend_calls=$r.proof.backend_calls
        $r.state='pass'
    }catch{$r.errors+=$_.Exception.Message}
    Save $r "$prefix.boot.json"
}elseif($Mode -eq 'Verify'){
    $c=Json "$prefix.arm.json";$root=$c.root;Check-Root $root
    $r=@{state='unknown';errors=@();scope='P09 real poweroff with automatic test-only boot bridge; production CLI not verified';event_reporting='not_tested'}
    try {
        $boot=Json "$prefix.boot.json";if($boot.state -ne 'pass'){throw 'BOOT_BRIDGE_FAILED'};$r.boot=$boot
        $r.health=Health 'final-v2'
        foreach($field in @('client_id','client_uuid','agent_installation_id')){if($r.health.status.$field -ne $c.before.status.$field){throw 'IDENTITY_CHANGED'}}
        if($r.health.recovery_task -cne $c.before.recovery_task){throw 'RECOVERY_TASK_CHANGED'}
        if((Hash "$root\execution-result.json") -ne $c.barrier.receipt_sha256 -or (Hash "$root\transaction.json") -ne $c.barrier.transaction_sha256){throw 'TERMINAL_RECORD_CHANGED'}
        if(Test-Path "$live\maintenance.json"){throw 'MARKER_REMAINS'}
        if(Get-ScheduledTask "RustDeskManagementAgentResume-$($c.barrier.transaction_id)" -ErrorAction SilentlyContinue){throw 'ORIGINAL_TASK_REMAINS'}
        $events=@(Get-WinEvent -LogName $channel -FilterXPath "*[System[EventRecordID > $($c.task_record_id)]]")
        [IO.File]::WriteAllText("$prefix.task-events.xml",'<Events>'+[string]::Join('',[string[]]@($events|ForEach-Object {$_.ToXml()}))+'</Events>')
        $r.events=@($events|ForEach-Object {[xml]$x=$_.ToXml();$d=@{};foreach($v in $x.SelectNodes('//*[local-name()="EventData"]/*[local-name()="Data"]')){$d[$v.GetAttribute('Name')]=$v.InnerText};@{record_id=$_.RecordId;id=$_.Id;data=$d}}|Where-Object {$_.data['TaskName'] -eq "\$bootTask"})
        $start=@($r.events|Where-Object id -eq 118);$done=@($r.events|Where-Object id -eq 201)
        if($start.Count -ne 1 -or !$done.Count){throw 'BOOT_EVENT_ATTRIBUTION_MISSING'}
        $instance=$start[0].data['InstanceId'];if(!$instance){$instance=$start[0].data['TaskInstanceId']}
        if(!$instance -or !@($done|Where-Object {$_.data['TaskInstanceId'] -eq $instance -and $_.data['ResultCode'] -eq '0'}).Count){throw 'BOOT_EVENT_INSTANCE_MISMATCH'}
        if(@($r.events|Where-Object id -eq 329).Count){throw 'BOOT_TASK_TIMEOUT_329'}
        $r.boot_instance=$instance
        Unregister-ScheduledTask $bootTask -Confirm:$false
        Unregister-ScheduledTask "RustDeskAgentLabP09Arm-$suffix" -Confirm:$false
        Unregister-ScheduledTask "RustDeskAgentLabP09Verify-$suffix" -Confirm:$false
        if(!$c.log_was_enabled){& "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:false;if($LASTEXITCODE -ne 0){throw 'LOG_RESTORE_FAILED'}}
        $r.state='pass';$r.source_retained=$root
    }catch{$r.errors+=$_.Exception.Message}
    Save $r "$prefix.verify-v2.json"
    Export-Evidence $root "$prefix.postcut-v2.zip"
    if($r.state -ne 'pass'){exit 1}
}else{
    if($run -notin @('p09-v203','p09-v209')){throw 'REPORT_REQUIRES_NEW_IDENTITY_RUN'}
    $c=Json "$prefix.arm.json";$root=$c.root;Check-Root $root
    $scene=Split-Path (Split-Path (Split-Path (Split-Path $root)))
    $r=@{state='unknown';errors=@();poll_runs=@();scope='original fixture identity post-powercut report; production CLI unverified'}
    $server=$null
    try {
        if((Json "$prefix.verify-v2.json").state -ne 'pass'){throw 'P09_NOT_VERIFIED'}
        $r.before=Health 'report-before'
        if(!(Test-Path "$scene\report-identity.json")){throw 'ORIGINAL_IDENTITY_MISSING'}
        $php="$lab\api-tests\php\php.exe";$ini="$lab\api-tests\php-test.ini"
        $env:AGENT_HTTP_FIXTURE_DB="$scene\agent-status-test.db"
        $env:RUSTDESK_DB_DRIVER='sqlite';$env:RUSTDESK_ADMIN_PATH='/agent-test';$env:RUSTDESK_INSTALL_CONFIG="$scene\absent.json"
        $query='$db=new PDO("sqlite:".getenv("AGENT_HTTP_FIXTURE_DB"));$db->setAttribute(PDO::ATTR_ERRMODE,PDO::ERRMODE_EXCEPTION);echo json_encode(["commands"=>$db->query("SELECT command_id,status,event_sequence,receipt_persisted FROM device_agent_commands")->fetchAll(PDO::FETCH_ASSOC),"events"=>$db->query("SELECT event_id,sequence,status FROM device_agent_events ORDER BY sequence")->fetchAll(PDO::FETCH_ASSOC)]);'
        [IO.File]::WriteAllText("$scene\report-query.php",'<?php '+$query)
        $baseline=& $php -c $ini "$scene\report-query.php"
        if($LASTEXITCODE -ne 0){throw 'SQL_QUERY_FAILED'}
        $r.sql_before=$baseline|ConvertFrom-Json
        if($r.sql_before.commands.Count -ne 1 -or $r.sql_before.commands[0].status -ne 'running' -or $r.sql_before.events.Count -ne 2){throw 'NOT_ORIGINAL_RUNNING_DATABASE'}
        $router=@'
<?php
$path=parse_url($_SERVER['REQUEST_URI'],PHP_URL_PATH);
$body=json_decode(file_get_contents('php://input'),true);
ob_start(function($output)use($path,$body){
    if($path==='/rd/update/v1/agent/events'&&($body['status']??null)==='rolled_back'){
        $ack=json_decode($output,true);
        file_put_contents(getenv('REPORT_LOG'),json_encode(['event_id'=>$body['event_id'],'sequence'=>$body['sequence'],'ack'=>$ack])."\n",FILE_APPEND|LOCK_EX);
        if(($ack['ok']??false)&&!file_exists(getenv('DROP_FLAG'))){file_put_contents(getenv('DROP_FLAG'),'server accepted before response loss',LOCK_EX);return '{';}
    }
    return $output;
});
require getenv('REPORT_ROUTER');
'@
        [IO.File]::WriteAllText("$scene\report-router.php",$router)
        $env:REPORT_LOG="$scene\report-http-events.jsonl";$env:DROP_FLAG="$scene\report-first-ack-lost";$env:REPORT_ROUTER="$scene\api\tests\agent_status_http_router.php"
        $listener=New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback,0);$listener.Start();$port=$listener.LocalEndpoint.Port;$listener.Stop()
        $server=Start-Process $php -ArgumentList "-c $ini -S 127.0.0.1:$port $scene\report-router.php" -PassThru -RedirectStandardOutput "$prefix.php.stdout.txt" -RedirectStandardError "$prefix.php.stderr.txt"
        $null=$server.Handle;$r.api_process=@{pid=$server.Id;created=$server.StartTime.ToUniversalTime().ToFileTimeUtc()}
        $wait=[Diagnostics.Stopwatch]::StartNew()
        while($true){try{$tcp=New-Object Net.Sockets.TcpClient;$tcp.Connect('127.0.0.1',$port);$tcp.Dispose();break}catch{if($wait.Elapsed.TotalSeconds -gt 15){throw 'API_START_TIMEOUT'};Start-Sleep -Milliseconds 100}}
        $env:RUSTDESK_REPORT_ROOT=$scene;$env:RUSTDESK_REPORT_API="127.0.0.1:$port"
        $exe="$lab\rustdesk_management_agent-$run.exe"
        if((Hash $exe) -ne $c.test_sha256){throw 'REPORT_EXE_HASH_MISMATCH'}
        $test='management_agent::operations_runtime::tests::powercut_original_identity_report_child'
        foreach($phase in @('lost-ack','retry','duplicate')){
            $env:RUSTDESK_REPORT_PHASE=$phase
            $p=Start-Process $exe -ArgumentList "--ignored --exact $test --nocapture --test-threads=1" -PassThru -RedirectStandardOutput "$prefix.$phase.stdout.txt" -RedirectStandardError "$prefix.$phase.stderr.txt"
            $null=$p.Handle;$owner=@{phase=$phase;pid=$p.Id;created=$p.StartTime.ToUniversalTime().ToFileTimeUtc()}
            if(!$p.WaitForExit(60000)){throw 'POLL_TIMEOUT'};$p.WaitForExit();$owner.exit=$p.ExitCode;$r.poll_runs+=$owner
            if($p.ExitCode -ne 0 -or [IO.File]::ReadAllText("$prefix.$phase.stdout.txt") -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored;'){throw "POLL_FAILED_$phase"}
            $sqlText=& $php -c $ini "$scene\report-query.php";if($LASTEXITCODE -ne 0){throw 'SQL_QUERY_FAILED'}
            $sql=$sqlText|ConvertFrom-Json;Save $sql "$prefix.$phase.sql.json"
            if($sql.commands.Count -ne 1 -or $sql.events.Count -ne 3 -or $sql.commands[0].status -ne 'rolled_back' -or $sql.commands[0].event_sequence -ne 3 -or $sql.events[2].status -ne 'rolled_back'){throw 'SQL_TERMINAL_MISMATCH'}
            if($phase -eq 'lost-ack'){$r.event_id=$sql.events[2].event_id;$r.after_lost_ack=$sql}else{if($sql.events[2].event_id -ne $r.event_id){throw 'EVENT_ID_CHANGED'}}
        }
        $acks=@(Get-Content "$scene\report-http-events.jsonl"|ForEach-Object {$_|ConvertFrom-Json})
        if($acks.Count -ne 2 -or $acks[0].ack.duplicate -ne $false -or $acks[1].ack.duplicate -ne $true -or $acks[0].event_id -ne $acks[1].event_id){throw 'P11_DUPLICATE_NOT_PROVEN'}
        $pending=Json "$scene\report-lost-ack.json"
        if($pending.pending_events.Count -ne 1 -or $pending.pending_events[0].event_id -ne $r.event_id){throw 'JOURNAL_OUTBOX_ID_MISMATCH'}
        if((Json "$scene\report-retry.json").pending_events.Count -ne 0 -or (Json "$scene\report-duplicate.json").pending_events.Count -ne 0){throw 'JOURNAL_ACK_NOT_PERSISTED'}
        $r.acks=$acks
        if((Hash "$root\execution-result.json") -ne $c.barrier.receipt_sha256 -or (Hash "$root\transaction.json") -ne $c.barrier.transaction_sha256){throw 'TERMINAL_RECORD_CHANGED'}
        $r.after=Health 'report-after'
        foreach($field in @('hashes','services','recovery_task')){if(($r.before.$field|ConvertTo-Json -Compress) -cne ($r.after.$field|ConvertTo-Json -Compress)){throw "HEALTH_CHANGED_$field"}}
        foreach($field in @('client_id','client_uuid','agent_installation_id')){if($r.before.status.$field -ne $r.after.status.$field){throw 'IDENTITY_CHANGED'}}
        if($r.before.status.application.pid -ne $r.after.status.application.pid -or $r.before.status.application.process_created_at -ne $r.after.status.application.process_created_at){throw 'APPLICATION_PROCESS_CHANGED'}
        Remove-Item -LiteralPath "$scene\report-identity.json" -Force;$r.secret_removed=!(Test-Path "$scene\report-identity.json")
        $r.state='pass'
    }catch{$r.errors+=$_.Exception.Message}
    finally{
        if($server -and !$server.HasExited){$server.Kill();$server.WaitForExit()}
        Save $r "$prefix.report.json"
        if($r.state -eq 'pass'){
            Copy-Item -LiteralPath "$scene\agent-status-test.db" -Destination "$prefix.final.db"
            foreach($name in @('report-http-events.jsonl','report-lost-ack.json','report-retry.json','report-duplicate.json','identity-persistence-proof.json')){Copy-Item -LiteralPath "$scene\$name" -Destination "$prefix.$name"}
            Unregister-ScheduledTask "RustDeskAgentLabP09Report-$suffix" -Confirm:$false
        }
        Export-Evidence $root "$prefix.report-evidence.zip"
    }
    if($r.state -ne 'pass'){exit 1}
}
