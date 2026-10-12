param([Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{1,48}$')][string]$RunId)
$ErrorActionPreference='Stop'
$root='C:\RustDeskAgentLab'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if($env:COMPUTERNAME -notin @('RDLAB-WIN10','RDLAB-WIN11') -or $sid -ne 'S-1-5-18'){throw 'SYSTEM lab VM required'}
$prefix=Join-Path $root $RunId
if(Test-Path "$prefix.result.json"){throw 'Run already exists; preserve evidence'}
$package=Join-Path $root 'agent-auto-bootstrap-v139.msi'
$directory='C:\ProgramData\RustDesk\RecoveryBootstrap'
$manifest=Join-Path $directory 'bootstrap.json'
$entry=Join-Path $directory 'rustdesk-recovery-bootstrap.exe'
$stateRoot='C:\ProgramData\RustDesk\ManagementAgent\state'
$ownedMarker=$null
$original=$null
$written=$null
$result=@{run=$RunId;computer=$env:COMPUTERNAME;sid=$sid;status='RUNNING';cases=@();started=[DateTime]::UtcNow.ToString('o')}
function Save-Result {$result|ConvertTo-Json -Depth 8|Set-Content "$prefix.result.json" -Encoding UTF8}
function Services { @(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,ProcessId,PathName,StartName,StartMode|Sort-Object Name) }
function Install-Case([string]$Name,[bool]$Repair) {
    $arguments='/i "'+$package+'" /qn /norestart REBOOT=ReallySuppress LAUNCH_TRAY_APP=N DEFENDER_EXCLUSIONS=0 /L*v "'+$prefix+'.'+$Name+'.log"'
    if($Repair){$arguments+=' REINSTALL=ALL REINSTALLMODE=amus'}
    $process=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList $arguments -PassThru
    $null=$process.Handle
    if(!$process.WaitForExit(600000)){throw 'Installer timeout; preserve process and evidence'}
    $process.WaitForExit()
    return $process.ExitCode
}
try {
    if((Get-FileHash $package).Hash -ne '5c4e4508ee116d64d4314d289cb7b0ad0d6d1e6c213d3607b3722a697ac05979'){throw 'Verified v139 package required'}
    foreach($gate in @('disable-agent','disable-recovery','maintenance.json')) {
        if(Test-Path (Join-Path $stateRoot $gate)){throw 'Existing local gate; do not change scene'}
    }
    $beforeState=Get-Content (Join-Path $stateRoot 'agent.json') -Raw|ConvertFrom-Json
    $original=[IO.File]::ReadAllBytes($manifest)
    $entryHash=(Get-FileHash $entry).Hash
    $task=Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery
    $code=Install-Case 'adopt' $false
    if($code -ne 0){throw "Same-package adoption failed: $code"}
    $result.cases+=@{name='adopt';exit_code=$code;status='PASS'}
    Save-Result
    foreach($gate in @('maintenance.json','disable-agent','disable-recovery')) {
        $ownedMarker=Join-Path $stateRoot $gate
        $handle=[IO.File]::Open($ownedMarker,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try{$bytes=[Text.Encoding]::UTF8.GetBytes($RunId);$handle.Write($bytes,0,$bytes.Length);$handle.Flush($true)}finally{$handle.Dispose()}
        $written=[Text.Encoding]::UTF8.GetBytes([Text.Encoding]::UTF8.GetString($original)+"`r`n  ")
        [IO.File]::WriteAllBytes($manifest,$written)
        $manifestHash=(Get-FileHash $manifest).Hash
        $code=Install-Case $gate $true
        $unchanged=(Get-FileHash $manifest).Hash -eq $manifestHash -and (Get-FileHash $entry).Hash -eq $entryHash
        $result.cases+=@{name=$gate;exit_code=$code;bootstrap_preserved=$unchanged;status=if($code -eq 0 -and $unchanged){'PASS'}else{'FAIL'}}
        Save-Result
        if($code -ne 0 -or !$unchanged){throw "Blocked recovery component changed or repair failed: $gate"}
        if([IO.File]::ReadAllText($ownedMarker) -ne $RunId){throw 'Local gate changed'}
        [IO.File]::WriteAllBytes($manifest,$original);$written=$null
        [IO.File]::Delete($ownedMarker);$ownedMarker=$null
    }
    $beforeServices=Services|ConvertTo-Json -Compress
    $beforeBytes=[IO.File]::ReadAllBytes((Join-Path $stateRoot 'agent.json'))
    $written=[Text.Encoding]::UTF8.GetBytes('{"schema":1,"agent_sha256":"invalid"}')
    [IO.File]::WriteAllBytes($manifest,$written)
    $code=Install-Case 'drift' $false
    $log=Get-Content "$prefix.drift.log" -Raw
    if($code -ne 1603 -or $log -notmatch 'Bootstrap preflight result: 20'){throw 'Drift was not refused at preflight'}
    if((Services|ConvertTo-Json -Compress) -ne $beforeServices){throw 'Rejected install changed service state or PID'}
    if([Convert]::ToBase64String([IO.File]::ReadAllBytes($manifest)) -ne [Convert]::ToBase64String($written)){throw 'Rejected install changed malformed manifest'}
    if([Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $stateRoot 'agent.json'))) -ne [Convert]::ToBase64String($beforeBytes)){throw 'Rejected install changed agent state'}
    $result.cases+=@{name='drift';exit_code=$code;status='PASS'}
    [IO.File]::WriteAllBytes($manifest,$original);$written=$null
    $code=Install-Case 'repeat' $false
    if($code -ne 0 -or (Get-FileHash $entry).Hash -ne $entryHash){throw 'Repeated installation changed recovery entry'}
    if((Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery) -ne $task){throw 'Task changed'}
    if((Get-Content (Join-Path $stateRoot 'agent.json') -Raw|ConvertFrom-Json).agent_installation_id -ne $beforeState.agent_installation_id){throw 'Identity changed'}
    $result.cases+=@{name='repeat';exit_code=$code;status='PASS'}
    $result.status='PASS'
}catch{$result.status='FAIL';$result.error=$_.Exception.Message;$result.position=$_.InvocationInfo.PositionMessage}
finally {
    if($written -and $original){
        if([Convert]::ToBase64String([IO.File]::ReadAllBytes($manifest)) -eq [Convert]::ToBase64String($written)){
            [IO.File]::WriteAllBytes($manifest,$original);$result.fixture_manifest_restored=$true
        }
    }
    if($ownedMarker -and (Test-Path $ownedMarker) -and [IO.File]::ReadAllText($ownedMarker) -eq $RunId){
        [IO.File]::Delete($ownedMarker);$result.fixture_gate_removed=$true
    }
    $result.at=[DateTime]::UtcNow.ToString('o');Save-Result
}
if($result.status -ne 'PASS'){exit 1}
