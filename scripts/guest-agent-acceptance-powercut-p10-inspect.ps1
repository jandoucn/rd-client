param([Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{8,64}$')][string]$RunId)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'WIN10_SYSTEM_REQUIRED: set RUSTDESK_LAB_HOST to this COMPUTERNAME (RUSTDESK_LAB_OS unset or win10) and run as SYSTEM'}
$lab='C:\RustDeskAgentLab'
$scene="$lab\local-command-5608"
$transaction='517bab529ccc3a5494642fd63a606d2b'
$tx="$scene\RustDesk\ManagementAgent\transactions\$transaction"
$prefix="$lab\$RunId"
if(@(Get-ChildItem -LiteralPath $lab -Filter "$RunId.*").Count){throw 'RUN_ID_REUSED'}
$claim=[IO.File]::Open("$prefix.claim",[IO.FileMode]::CreateNew);$claim.Dispose()
$r=@{run_id=$RunId;state='unknown';errors=@();poll_tests_executed=0;secret_values_exported=$false;production_cli='not_verified';started=[DateTime]::UtcNow.ToString('o')}
$timer=[Diagnostics.Stopwatch]::StartNew()
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path).Hash}
function Health([string]$Moment){
    $agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
    $p=Start-Process $agent -ArgumentList 'status --json --edition standard --package-kind msi' -PassThru -RedirectStandardOutput "$prefix.$Moment.status.json" -RedirectStandardError "$prefix.$Moment.stderr.txt"
    $null=$p.Handle
    if(!$p.WaitForExit(30000)){throw 'STATUS_TIMEOUT'};$p.WaitForExit();if($p.ExitCode -ne 0){throw 'STATUS_FAILED'}
    $s=[IO.File]::ReadAllText("$prefix.$Moment.status.json")|ConvertFrom-Json
    if($s.application.state -ne 'ready' -or $s.agent.build_seq -ne 2026100502 -or $s.application.build_seq -ne 2026100502 -or $s.agent_installation_id -ne '76b922ac75fb14f19783bb6d48c45653' -or $s.package_identity.edition -ne 'standard'){throw 'HEALTH_BASELINE_MISMATCH'}
    $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
    if($services.Count -ne 2 -or @($services|Where-Object {$_.State -ne 'Running' -or $_.StartName -ne 'LocalSystem' -or $_.StartMode -ne 'Auto'}).Count){throw 'SERVICE_UNHEALTHY'}
    $processes=@($services|ForEach-Object {$p=Get-Process -Id $_.ProcessId;@{pid=$p.Id;created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc();path=$p.Path}})
    $app=Get-Process -Id $s.application.pid
    $hashes=@{agent=(Hash $agent);dll=(Hash 'C:\Program Files\RustDesk\librustdesk.dll');bootstrap=(Hash 'C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe')}
    if($hashes.agent -ne '00866e366c7cc222b39b4a4b34d8ab3938dd1c7dfb23cc00b76982ecdd155c1b' -or $hashes.bootstrap -ne $hashes.agent -or $hashes.dll -ne 'f131d0a1857a177e61a82bf978a973fec8e64edbd592311a8f08d180b53d1bb5'){throw 'HASH_MISMATCH'}
    foreach($name in @('maintenance.json','disable-agent','disable-recovery')){if(Test-Path "C:\ProgramData\RustDesk\ManagementAgent\state\$name"){throw 'LOCAL_MARKER_PRESENT'}}
    @{status=$s;services=$services;processes=$processes;hashes=$hashes;app=@{pid=$app.Id;created_filetime=$app.StartTime.ToUniversalTime().ToFileTimeUtc()};task=(Export-ScheduledTask RustDeskManagementAgentRecovery)}
}
function Evidence-Hashes {
    $values=@{}
    foreach($path in @("$scene\agent-status-test.db","$scene\RustDesk\ManagementAgent\state\commands-1.json","$tx\transaction.json","$tx\execution-result.json")){$values[$path]=Hash $path}
    $values
}
try {
    $r.before=Health 'before';$r.evidence_before=Evidence-Hashes
    $r.identity_files=@()
    foreach($path in @("$scene\poll-fixture.json","$scene\report-identity.json","$scene\p10-report-identity.json")){
        $present=Test-Path -LiteralPath $path
        $entry=@{path=$path;present=$present;secret_field_present=$false;secret_decoded_length=0}
        if($present){
            $value=[IO.File]::ReadAllText($path)|ConvertFrom-Json
            if($value.PSObject.Properties.Name -contains 'secret'){
                $entry.secret_field_present=$true
                $entry.secret_decoded_length=([Convert]::FromBase64String($value.secret)).Length
            }
        }
        $r.identity_files+=$entry
    }
    # Only file names and lengths are inventoried; no JSON contents or key bytes are exported.
    $r.scene_files=@(Get-ChildItem -LiteralPath $scene -Recurse -Force -File|Where-Object {$_.Name -match '(poll-fixture|report-identity|secret|credential|key)'}|Select-Object FullName,Length)
    $r.journal=([IO.File]::ReadAllText("$scene\RustDesk\ManagementAgent\state\commands-1.json")|ConvertFrom-Json)
    $r.terminal=([IO.File]::ReadAllText("$tx\execution-result.json")|ConvertFrom-Json)
    $r.php_available=Test-Path "$lab\api-tests\php\php.exe"
    $r.api_source_available=Test-Path "$lab\api-tests\api\sqlite\agent_operations.php"
    $r.sources=@(Get-ChildItem -LiteralPath $tx -Recurse -Force -Filter '*.msi'|ForEach-Object {@{path=$_.FullName;hash=(Hash $_.FullName)}})
    $r.secret_available=@($r.identity_files|Where-Object {$_.secret_field_present -and $_.secret_decoded_length -eq 64}).Count -gt 0
    $r.state=if($r.secret_available){'identity_candidate_requires_crypto_validation'}else{'blocked_original_fixture_secret_not_persisted'}
}catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
finally {
    try {
        $r.after=Health 'after';$r.evidence_after=Evidence-Hashes
        foreach($field in @('services','processes','app','hashes','task')){
            if(($r.before.$field|ConvertTo-Json -Depth 8 -Compress) -cne ($r.after.$field|ConvertTo-Json -Depth 8 -Compress)){throw "HEALTH_CHANGED_$field"}
        }
        foreach($field in @('client_id','client_uuid','agent_installation_id')){if($r.before.status.$field -cne $r.after.status.$field){throw 'IDENTITY_CHANGED'}}
        foreach($path in $r.evidence_before.Keys){if($r.evidence_before[$path] -ne $r.evidence_after[$path]){throw 'EVIDENCE_CHANGED'}}
        $r.health_and_evidence_preserved=$true
    }catch{$r.errors+=$_.Exception.Message;$r.state='fail'}
    $r.remaining_processes=@(Get-CimInstance Win32_Process|Where-Object Name -in @('php.exe','helper.exe','rustdesk_management_agent-p09-v201.exe')|Select-Object Name,ProcessId)
    Unregister-ScheduledTask -TaskName "RustDeskAgentLabP10-$RunId" -Confirm:$false
    $r.task_removed=$true;$r.elapsed_ms=$timer.ElapsedMilliseconds
    $r|ConvertTo-Json -Depth 18|Set-Content "$prefix.result.json" -Encoding UTF8
    Copy-Item -LiteralPath "$scene\agent-status-test.db" -Destination "$prefix.commands.db"
}
if($r.state -eq 'fail'){exit 1}
