param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{8,64}$')][string]$RunId,
    [ValidateSet('CaptureSos','CaptureStandard','Upgrade','Rollback')][string]$Mode,
    [ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$TestSha256
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'WIN10_SYSTEM_REQUIRED: set RUSTDESK_LAB_HOST to this COMPUTERNAME (RUSTDESK_LAB_OS unset or win10) and run as SYSTEM'}
$lab='C:\RustDeskAgentLab';$prefix=Join-Path $lab $RunId
if(@(Get-ChildItem -LiteralPath $lab -Filter "$RunId.*").Count){throw 'RUN_ID_REUSED'}
$claim=[IO.File]::Open("$prefix.claim",[IO.FileMode]::CreateNew);$claim.Dispose()
$clock=[Diagnostics.Stopwatch]::StartNew()
$r=@{run_id=$RunId;mode=$Mode;state='unknown';sid='S-1-5-18';computer=$env:COMPUTERNAME;started=[DateTime]::UtcNow.ToString('o');tests_executed=0;errors=@();remote_control='not_tested'}
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
function Run([string]$Exe,[string]$Arguments,[string]$Name,[int]$Seconds){
    $p=Start-Process $Exe -ArgumentList $Arguments -PassThru -RedirectStandardOutput "$prefix.$Name.stdout.txt" -RedirectStandardError "$prefix.$Name.stderr.txt"
    $null=$p.Handle
    $record=@{pid=$p.Id;created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc();exit_code=$null;state='unknown'}
    $record|ConvertTo-Json|Set-Content "$prefix.$Name.process.json" -Encoding UTF8
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while(!$p.HasExited -and $watch.Elapsed.TotalSeconds -lt $Seconds){[void]$p.WaitForExit(250)}
    $record.elapsed_ms=$watch.ElapsedMilliseconds
    if($p.HasExited){$p.WaitForExit();$record.exit_code=$p.ExitCode;$record.state='exited'}
    $record|ConvertTo-Json|Set-Content "$prefix.$Name.process.json" -Encoding UTF8
    $record
}
function Health([string]$Moment,[string]$Edition,[long]$Build,[string]$Installation){
    $p=Run $agent "status --json --edition $Edition --package-kind msi" "$Moment-status" 30
    if($p.exit_code -ne 0){throw "STATUS_FAILED_$Moment"}
    $s=[IO.File]::ReadAllText("$prefix.$Moment-status.stdout.txt")|ConvertFrom-Json
    $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
    if($services.Count -ne 2 -or @($services|Where-Object {$_.State -ne 'Running' -or $_.StartMode -ne 'Auto' -or $_.StartName -ne 'LocalSystem'}).Count){throw 'SERVICES_UNHEALTHY'}
    if($s.application.state -ne 'ready' -or $s.application.build_seq -ne $Build -or $s.agent.build_seq -ne $Build -or $s.agent_installation_id -ne $Installation -or $s.package_identity.edition -ne $Edition){throw 'HEALTH_IDENTITY_BUILD_MISMATCH'}
    $processes=@($services|ForEach-Object {$p=Get-Process -Id $_.ProcessId;@{pid=$p.Id;created=$p.StartTime.ToUniversalTime().ToFileTimeUtc();path=$p.Path}})
    foreach($name in @('disable-agent','disable-recovery','maintenance.json')){if(Test-Path "C:\ProgramData\RustDesk\ManagementAgent\state\$name"){throw "LOCAL_MARKER_$name"}}
    $hash=@{agent=(Get-FileHash $agent).Hash;dll=(Get-FileHash 'C:\Program Files\RustDesk\librustdesk.dll').Hash;bootstrap=(Get-FileHash 'C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe').Hash}
    $expectedAgent=if($Edition -eq 'sos'){'d46fcbc3d4554fea8d8bdfa0166cd69c0597f09484210572d6e50280e543f423'}elseif($Build -eq 2026100503){'091831fac3da7c33c27ac81672f5f7e8572fde17e1b7ce300d12217025f0d504'}else{'00866e366c7cc222b39b4a4b34d8ab3938dd1c7dfb23cc00b76982ecdd155c1b'}
    $expectedDll=if($Edition -eq 'sos'){'4d0375fc508f3906836017f53f593a56c8443cdc3e99a0e209c4d7a1ff1ed23b'}elseif($Build -eq 2026100503){'8c523baf9937057c6d6260b0698963c544685c40d3bf6de485acad924143a9ef'}else{'f131d0a1857a177e61a82bf978a973fec8e64edbd592311a8f08d180b53d1bb5'}
    if($hash.agent -ne $expectedAgent -or $hash.bootstrap -ne $expectedAgent -or $hash.dll -ne $expectedDll){throw 'INSTALLED_HASH_MISMATCH'}
    $xml=Export-ScheduledTask RustDeskManagementAgentRecovery
    [IO.File]::WriteAllText("$prefix.$Moment-task.xml",$xml)
    @{status=$s;services=$services;processes=$processes;hashes=$hash;task=$xml;fixture_services=@(Get-CimInstance Win32_Service|Where-Object Name -like 'RustDeskAgentTest*'|Select-Object Name,State);resume_tasks=@(Get-ScheduledTask|Where-Object TaskName -like 'RustDeskManagementAgentResume-*'|Select-Object TaskName,State)}
}
try {
    $edition=if($Mode -eq 'CaptureSos'){'sos'}else{'standard'}
    $installation=if($edition -eq 'sos'){'075c943c979685b89ff9f4515560cbda'}else{'76b922ac75fb14f19783bb6d48c45653'}
    $r.before=Health 'before' $edition 2026100502 $installation
    if($Mode -in @('Upgrade','Rollback')){
        $exe="$lab\rustdesk_management_agent-mirror-e2e.exe"
        $r.test_sha256=(Get-FileHash $exe).Hash
        if(!$TestSha256 -or $r.test_sha256 -ne $TestSha256){throw 'TEST_HASH_MISMATCH'}
        $r.target_hash=(Get-FileHash "$lab\agent-cross-v153.msi").Hash
        $r.rollback_hash=(Get-FileHash "$lab\agent-auto-bootstrap-v139.msi").Hash
        if($r.target_hash -ne 'af62de5a7edf73b8f8a515513b7680f8e48ed1c03bfe73a653efbc9f78b97782' -or $r.rollback_hash -ne '5c4e4508ee116d64d4314d289cb7b0ad0d6d1e6c213d3607b3722a697ac05979'){throw 'MSI_HASH_MISMATCH'}
        $env:RUSTDESK_LOCAL_COMMAND_LAB='v142'
        $env:AGENT_TEST_API_ROOT="$lab\api-tests\api"
        $env:AGENT_TEST_PHP="$lab\api-tests\php\php.exe"
        $env:AGENT_TEST_PHP_INI="$lab\api-tests\php-test.ini"
        $name=if($Mode -eq 'Upgrade'){'local_api_mirrors_upgrade_real_msi_and_report_once'}else{'local_api_mirrors_validation_failure_restores_real_msi_and_reports_once'}
        $test="management_agent::operations_runtime::tests::$name"
        $listed=Run $exe "--list --exact $test" 'list' 30
        if($listed.exit_code -ne 0 -or [IO.File]::ReadAllText("$prefix.list.stdout.txt") -notmatch '1 test, 0 benchmarks'){throw 'TEST_NOT_FOUND'}
        $p=Run $exe "--ignored --exact $test --nocapture --test-threads=1" 'test' 1200
        $r.test_process=$p
        $r.transaction_root="$lab\local-command-$($p.pid)"
        $text=[IO.File]::ReadAllText("$prefix.test.stdout.txt")
        if($text -match 'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;'){$r.tests_executed=[int]$Matches[2]+[int]$Matches[3];$r.passed=[int]$Matches[2];$r.failed=[int]$Matches[3]}
        if($p.state -ne 'exited' -or $p.exit_code -ne 0 -or $r.tests_executed -ne 1 -or $r.passed -ne 1){throw 'REAL_MSI_E2E_FAILED_PRESERVE_SCENE'}
        $build=if($Mode -eq 'Upgrade'){2026100503}else{2026100502}
        $r.after=Health 'after' 'standard' $build $installation
        foreach($field in @('client_id','client_uuid','agent_installation_id')){if($r.before.status.$field -ne $r.after.status.$field){throw "IDENTITY_CHANGED_$field"}}
        if($r.before.task -cne $r.after.task){throw 'RECOVERY_TASK_CHANGED'}
        $r.requests=[IO.File]::ReadAllText("$($r.transaction_root)\mirror-http-requests.json")|ConvertFrom-Json
        $r.downloads=@([IO.File]::ReadAllText("$($r.transaction_root)\mirror-cache-verified.json")|ConvertFrom-Json)
        foreach($download in $r.downloads){if($download.download_elapsed_ms -ge 60000){throw 'REAL_PACKAGE_DOWNLOAD_BUDGET_EXCEEDED'}}
        $r.sql=[IO.File]::ReadAllText("$($r.transaction_root)\verified-sql.json")|ConvertFrom-Json
        $r.retained_source=$r.transaction_root
        $r.residue=@(Get-CimInstance Win32_Process|Where-Object {$_.Name -eq 'php.exe' -or $_.Name -eq 'helper.exe'}|Select-Object Name,ProcessId,ExecutablePath)
        if($r.residue.Count -or $r.after.resume_tasks.Count -or $r.after.fixture_services.Count){throw 'FIXTURE_RUNTIME_REMAINS'}
    }
    $r.state='pass'
}catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
finally {
    $r.elapsed_ms=$clock.ElapsedMilliseconds;$r.finished=[DateTime]::UtcNow.ToString('o')
    if($r.state -eq 'pass'){
        Unregister-ScheduledTask -TaskName "RustDeskAgentLabMirror-$RunId" -Confirm:$false
        $r.task_removed=$true
    }
    $r|ConvertTo-Json -Depth 18|Set-Content "$prefix.result.json" -Encoding UTF8
    $files=@(Get-ChildItem -LiteralPath $lab -File -Filter "$RunId.*")
    if($r.ContainsKey('transaction_root') -and (Test-Path -LiteralPath $r.transaction_root)){
        $files+=@(Get-ChildItem -LiteralPath $r.transaction_root -Recurse -Force -File | Where-Object {$_.Extension -in @('.json','.log','.txt','.db','.xml') -and $_.Name -ne 'poll-fixture.json' -and $_.FullName -notlike '*\api\*'})
    }
    # Include the SQLite database and transaction evidence while retaining all MSI source assets in place.
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip=[IO.Compression.ZipFile]::Open("$prefix.evidence.zip",[IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach($file in $files){
            $entry=$file.FullName.Substring($lab.Length+1).Replace('\','/')
            [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$file.FullName,$entry)
        }
    }finally{$zip.Dispose()}
}
if($r.state -ne 'pass'){exit 1}
