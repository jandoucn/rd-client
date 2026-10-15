$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
function Assert-AcceptanceContext([string]$Computer,[string]$Sid,[string]$Uuid){
    if($Computer -ne 'RDLAB-WIN11' -or $Sid -ne 'S-1-5-18' -or $Uuid -ne 'd91a13d9-2bb7-40bf-b5e4-9d57870cd242'){
        throw 'VM_SYSTEM_REQUIRED'
    }
}
Assert-AcceptanceContext $env:COMPUTERNAME ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value) ((Get-CimInstance Win32_ComputerSystemProduct).UUID)

function New-AcceptanceRun([string]$Root,[string]$RunId){
    if($RunId -notmatch '^[a-z0-9-]{8,64}$'){throw 'INVALID_RUN_ID'}
    if($Root -ne 'C:\RustDeskAgentLab' -or !(Test-Path -LiteralPath $Root -PathType Container)){throw 'INVALID_EVIDENCE_ROOT'}
    $prefix=Join-Path $Root $RunId
    if(@(Get-ChildItem -LiteralPath $Root -Filter "$RunId.*" -Force).Count){throw 'RUN_ID_REUSED'}
    try {$file=[IO.File]::Open("$prefix.claim.json",[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)}
    catch {throw "RUN_ID_REUSED: $($_.Exception.Message)"}
    try {
        $data=[Text.Encoding]::UTF8.GetBytes((@{run_id=$RunId;pid=$PID;created_filetime=(Get-Process -Id $PID).StartTime.ToUniversalTime().ToFileTimeUtc();utc=[DateTime]::UtcNow.ToString('o')}|ConvertTo-Json -Compress))
        $file.Write($data,0,$data.Length);$file.Flush($true)
    }finally{$file.Dispose()}
    $prefix
}

function Get-AcceptanceDimensions([string]$Text){
    $r=@{scm='unknown';local_application='unknown';registration='unknown';remote_control='not_tested'}
    try {
        $s=ConvertFrom-Json -InputObject $Text
        if($s.service.state -in @('absent','stopped','start_pending','running','stop_pending','paused','unknown')){$r.scm=$s.service.state}
    }catch{}
    try {
        if($s.application.state -in @('ready','unresponsive','starting','unknown')){$r.local_application=$s.application.state}
    }catch{}
    try {
        if($s.application.registered -is [bool]){$r.registration=if($s.application.registered){'registered_client_report'}else{'disconnected'}}
    }catch{}
    $r
}

function Test-AcceptanceProcessIdentity([long]$ExpectedPid,[long]$ExpectedCreated,[long]$ActualPid,[long]$ActualCreated){
    $ExpectedPid -gt 0 -and $ExpectedCreated -gt 0 -and $ExpectedPid -eq $ActualPid -and $ExpectedCreated -eq $ActualCreated
}

function Invoke-AcceptanceProcess([string]$Exe,[string]$Arguments,[string]$Prefix,[int]$TimeoutMs){
    if($TimeoutMs -lt 1){throw 'INVALID_TIMEOUT'}
    foreach($suffix in @('.stdout.txt','.stderr.txt','.process.json')){if(Test-Path -LiteralPath "$Prefix$suffix"){throw 'PROCESS_EVIDENCE_EXISTS'}}
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $r=@{state='unknown';error=$null;pid=$null;created_filetime=$null;exit_code=$null;elapsed_ms=0;stdout="$Prefix.stdout.txt";stderr="$Prefix.stderr.txt"}
    try {
        $p=Start-Process -FilePath $Exe -ArgumentList $Arguments -PassThru -RedirectStandardOutput $r.stdout -RedirectStandardError $r.stderr
        $null=$p.Handle
        $r.pid=$p.Id;$r.created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc()
        $r | ConvertTo-Json | Set-Content -LiteralPath "$Prefix.process.json" -Encoding UTF8
        while(!$p.HasExited -and $watch.ElapsedMilliseconds -lt $TimeoutMs){[void]$p.WaitForExit(50)}
        if(!$p.HasExited){$r.error='PROCESS_TIMEOUT'}else{
            $p.WaitForExit();$r.exit_code=$p.ExitCode
            if($p.ExitCode -eq 0){$r.state='pass'}else{$r.state='fail';$r.error='PROCESS_EXIT_NONZERO'}
        }
    }catch{$r.error=$_.Exception.Message}
    finally{
        $r.elapsed_ms=$watch.ElapsedMilliseconds
        $r | ConvertTo-Json | Set-Content -LiteralPath "$Prefix.process.json" -Encoding UTF8
    }
    $r
}

function Read-AcceptanceTestSummary([string]$Text,[int]$ExitCode){
    if($ExitCode -ne 0){throw 'TEST_PROCESS_FAILED'}
    $matches=[regex]::Matches($Text,'(?m)^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;')
    if($matches.Count -eq 0){throw 'MISSING_SUMMARY'}
    $last=$matches[$matches.Count-1]
    $r=@{passed=[int]$last.Groups[2].Value;failed=[int]$last.Groups[3].Value;ignored=[int]$last.Groups[4].Value}
    if($r.passed+$r.failed -eq 0){throw 'ZERO_TESTS'}
    if($r.failed -gt 0 -or $last.Groups[1].Value -ne 'ok'){throw 'TEST_ASSERTIONS_FAILED'}
    $r
}

function Resolve-AcceptanceBoot([object[]]$Events,[long]$LowerRecordId,[bool]$LogEnabled){
    $r=@{state='unknown';reason='missing_boot_instance';instance=$null;first_trigger_record=$null}
    if(!$LogEnabled){$r.reason='event_log_disabled';return $r}
    $fresh=@($Events|Where-Object {$_.record_id -gt $LowerRecordId -and $_.task -eq '\RustDeskManagementAgentRecovery'}|Sort-Object record_id)
    $boots=@($fresh|Where-Object id -eq 118)
    if(!$boots.Count){return $r}
    $first=$boots[0];$r.instance=$first.instance;$r.first_trigger_record=$first.record_id
    if([string]::IsNullOrWhiteSpace($first.instance)){$r.reason='missing_instance_id';return $r}
    $same=@($fresh|Where-Object {$_.instance -eq $first.instance -and $_.record_id -ge $first.record_id})
    if(@($same|Where-Object id -eq 329).Count){$r.state='fail';$r.reason='first_boot_timeout_329';return $r}
    $starts=@($same|Where-Object id -eq 200)
    $done=@($same|Where-Object id -eq 201)
    if(!$starts.Count -or !$done.Count){$r.reason='incomplete_instance';return $r}
    if($done[0].record_id -le $starts[0].record_id){$r.reason='invalid_event_order';return $r}
    if($done[0].result -notmatch '^(0|0x0+)$'){$r.state='fail';$r.reason='first_boot_exit_nonzero';return $r}
    $r.state='pass';$r.reason='first_boot_action_completed';$r
}
