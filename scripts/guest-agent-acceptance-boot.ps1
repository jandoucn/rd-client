param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{8,64}$')][string]$RunId,
    [ValidateSet('Baseline','Observe')][string]$Mode='Baseline',
    [ValidatePattern('^[a-z0-9-]{8,64}$')][string]$BaselineRunId
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'guest-agent-acceptance-common.ps1')
$prefix=New-AcceptanceRun 'C:\RustDeskAgentLab' $RunId
$clock=[Diagnostics.Stopwatch]::StartNew()
$r=@{schema=1;run_id=$RunId;mode=$Mode;state='unknown';executed=0;product_mutation_performed=$false;boot_recovery='unknown';events=@();error=$null}
$channel='Microsoft-Windows-TaskScheduler/Operational'
function Read-Events([string]$Channel,[long]$Floor){
    try {@(Get-WinEvent -LogName $Channel -FilterXPath "*[System[EventRecordID > $Floor]]" -MaxEvents 10000)}
    catch {if($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*'){return @()};throw}
}
try {
    $log=Get-WinEvent -ListLog $channel
    $r.task_log_enabled=$log.IsEnabled
    $r.vm_uuid=(Get-CimInstance Win32_ComputerSystemProduct).UUID
    $r.task_xml=Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery
    $r.task_record_id=if($log.RecordCount -gt 0){(Get-WinEvent -LogName $channel -MaxEvents 1).RecordId}else{0}
    $r.system_record_id=(Get-WinEvent -LogName System -MaxEvents 1).RecordId
    $r.budget_path='C:\ProgramData\RustDesk\RecoveryBootstrap\budget.json'
    $r.budget_text=if(Test-Path -LiteralPath $r.budget_path){[IO.File]::ReadAllText($r.budget_path)}else{$null}
    $r.services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,StartMode,State,ProcessId,PathName)
    if($Mode -eq 'Baseline'){
        $r.state='baseline_captured';$r.reason=if($log.IsEnabled){'coordination_required_before_reboot'}else{'task_log_disabled_requires_coordinated_enable_before_new_baseline'}
    }else{
        if(!$BaselineRunId -or $BaselineRunId -eq $RunId){throw 'INVALID_BASELINE_RUN'}
        $base=[IO.File]::ReadAllText("C:\RustDeskAgentLab\$BaselineRunId.boot.json")|ConvertFrom-Json
        if($base.schema -ne 1 -or $base.mode -ne 'Baseline' -or $base.state -ne 'baseline_captured' -or $base.vm_uuid -ne $r.vm_uuid -or $base.run_id -ne $BaselineRunId){throw 'BASELINE_IDENTITY_MISMATCH'}
        $r.baseline_run_id=$BaselineRunId
        if(!$base.task_log_enabled -or !$log.IsEnabled){throw 'EVENT_LOG_DISABLED'}
        if($r.task_record_id -lt $base.task_record_id -or $r.system_record_id -lt $base.system_record_id){throw 'EVENT_LOG_RESET'}
        if($r.task_xml -cne $base.task_xml){throw 'TASK_DEFINITION_CHANGED'}
        $taskOldest=Get-WinEvent -LogName $channel -Oldest -MaxEvents 1
        $systemOldest=Get-WinEvent -LogName System -Oldest -MaxEvents 1
        if($taskOldest.RecordId -gt $base.task_record_id+1 -or $systemOldest.RecordId -gt $base.system_record_id+1){throw 'EVENT_LOG_WRAPPED'}
        $taskEvents=@(Read-Events $channel $base.task_record_id)
        $systemEvents=@(Read-Events 'System' $base.system_record_id)
        if($taskEvents.Count -ge 10000 -or $systemEvents.Count -ge 10000){throw 'EVENT_CAPTURE_TRUNCATED'}
        [IO.File]::WriteAllText("$prefix.task-events.xml",'<Events>'+[string]::Join('',[string[]]@($taskEvents|ForEach-Object {$_.ToXml()}))+'</Events>')
        [IO.File]::WriteAllText("$prefix.system-events.xml",'<Events>'+[string]::Join('',[string[]]@($systemEvents|ForEach-Object {$_.ToXml()}))+'</Events>')
        $boots=@($systemEvents|Where-Object {$_.ProviderName -eq 'Microsoft-Windows-Kernel-General' -and $_.Id -eq 12})
        if($boots.Count -ne 1){throw 'NEW_BOOT_NOT_UNAMBIGUOUS'}
        $r.system_boot_record_id=$boots[0].RecordId
        $events=@($taskEvents|ForEach-Object {
            [xml]$xml=$_.ToXml();$data=@{}
            foreach($d in $xml.Event.EventData.Data){$data[[string]$d.Name]=[string]$d.'#text'}
            $instance=if($data.ContainsKey('TaskInstanceId')){$data.TaskInstanceId}else{$data['InstanceId']}
            [pscustomobject]@{record_id=$_.RecordId;id=$_.Id;instance=$instance;task=$data['TaskName'];result=$data['ResultCode']}
        })
        $r.events=$events
        $r.first_boot_action=Resolve-AcceptanceBoot $events $base.task_record_id $true
        $r.state=$r.first_boot_action.state;$r.executed=1
        $r.reason='action attribution only; compare budget delta and Manual service recovery separately'
    }
}catch{$r.error=$_.Exception.Message;$r.state='unknown'}
finally{
    $r.elapsed_ms=$clock.ElapsedMilliseconds
    $r|ConvertTo-Json -Depth 12|Set-Content -LiteralPath "$prefix.boot.json" -Encoding UTF8
}
if($r.state -notin @('pass','baseline_captured')){exit 1}
