param(
    [Parameter(Mandatory=$true)]
    [ValidatePattern('^[a-z0-9-]{1,48}$')]
    [string]$RunId,
    [ValidateSet('Manual','Periodic')]
    [string]$Trigger='Periodic',
    [ValidateSet('standard','sos')]
    [string]$Edition='standard'
)

$ErrorActionPreference='Stop'
$root='C:\RustDeskAgentLab'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or $sid -ne 'S-1-5-18'){
    throw '仅允许在 RUSTDESK_LAB_HOST 声明为本机 COMPUTERNAME 的 SYSTEM 环境执行'
}
if(!(Test-Path -LiteralPath $root -PathType Container)){throw '测试证据目录不存在'}
$prefix=Join-Path $root $RunId
foreach($suffix in @('.result.json','.progress.json','.status.json','.status.stderr.txt')){
    if(Test-Path -LiteralPath "$prefix$suffix"){throw '本轮标识已使用，保留既有证据'}
}
$stateRoot='C:\ProgramData\RustDesk\ManagementAgent\state'
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$bootstrap='C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe'
$marker=Join-Path $stateRoot 'disable-agent'
$markerText="bootstrap-test-$RunId"
$ownedMarker=$false
$result=@{run=$RunId;trigger=$Trigger;edition=$Edition;computer=$env:COMPUTERNAME;sid=$sid;status='RUNNING';started=[DateTime]::UtcNow.ToString('o');task_samples=@()}

function Get-LabServices {
    @(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|
        Select-Object Name,State,ProcessId,PathName,StartName,StartMode|Sort-Object Name)
}

function Save-Progress {
    $result|ConvertTo-Json -Depth 10|Set-Content "$prefix.progress.json" -Encoding UTF8
}

function Get-RecoveryTaskSample {
    $task=Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery
    $info=Get-ScheduledTaskInfo -TaskName RustDeskManagementAgentRecovery
    $sample=@{utc=[DateTime]::UtcNow.ToString('o');last_run=$info.LastRunTime.ToString('o');
        state=[int]$task.State;exit_code=$info.LastTaskResult}
    $last=if($result.task_samples.Count){$result.task_samples[-1]}else{@{}}
    if($last.last_run -ne $sample.last_run -or $last.state -ne $sample.state -or $last.exit_code -ne $sample.exit_code){
        $result.task_samples+=$sample
        Save-Progress
    }
    $sample
}

function Invoke-RegisteredRecoveryTask {
    $previous=Get-RecoveryTaskSample
    if($previous.state -ne 3){throw '恢复任务尚未就绪'}
    $channel='Microsoft-Windows-TaskScheduler/Operational'
    $wasEnabled=(Get-WinEvent -ListLog $channel).IsEnabled
    if(!$wasEnabled){
        & "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:true
        if($LASTEXITCODE -ne 0){throw '无法启用任务完成事件采集'}
    }
    try {
    $scheduler=New-Object -ComObject Schedule.Service
    $scheduler.Connect()
    $registered=$scheduler.GetFolder('\').GetTask('RustDeskManagementAgentRecovery')
    $instance=$registered.Run($null).InstanceGuid
    $running=$false
    $timer=[Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 500
        $sample=Get-RecoveryTaskSample
        if($sample.state -eq 4){$running=$true}
        $completed=@(Get-WinEvent -LogName $channel -FilterXPath "*[System[EventID=201] and EventData[Data[@Name='TaskInstanceId']='$instance']]" -ErrorAction SilentlyContinue)
        if($completed.Count){
            [xml]$event=$completed[0].ToXml()
            $code=($event.Event.EventData.Data|Where-Object {$_.Name -eq 'ResultCode'}).'#text'
            if($code -ne '0'){throw "恢复任务实例失败：$code"}
            $result.verified_task_instance=$instance
            Save-Progress
            return
        }
        # Win10 的连续启动可能复用 LastRunTime，实际 Running→Ready 也可证明完成。
        if($sample.state -eq 3 -and ($sample.last_run -ne $previous.last_run -or $running)){
            if($sample.exit_code -ne 0){throw "恢复任务失败：$($sample.exit_code)"}
            return
        }
    }while($timer.Elapsed.TotalSeconds -lt 120)
    throw '恢复任务未在限定时间内完成'
    } finally {
        if(!$wasEnabled){
            & "$env:SystemRoot\System32\wevtutil.exe" sl $channel /e:false
            if($LASTEXITCODE -ne 0){throw '无法还原任务日志设置'}
        }
    }
}

function Read-InstalledStatus {
    $process=Start-Process $agent -ArgumentList "status --json --edition $Edition --package-kind msi" -PassThru `
        -RedirectStandardOutput "$prefix.status.json" -RedirectStandardError "$prefix.status.stderr.txt"
    $null=$process.Handle
    if(!$process.WaitForExit(30000)){throw '状态探测超时，保留进程和现场'}
    $process.WaitForExit()
    if($process.ExitCode -ne 0){throw "状态探测失败：$($process.ExitCode)"}
    Get-Content "$prefix.status.json" -Raw|ConvertFrom-Json
}

try {
    $result.before=Get-LabServices
    if($result.before.Count -ne 2 -or @($result.before|Where-Object {
        $_.State -ne 'Running' -or $_.StartName -ne 'LocalSystem' -or $_.StartMode -ne 'Auto'
    }).Count){throw '双服务健康基线不满足'}
    foreach($name in @('disable-agent','disable-recovery','maintenance.json')){
        if(Test-Path -LiteralPath (Join-Path $stateRoot $name)){throw "已有本地门禁：$name"}
    }
    $beforeState=Get-Content "$stateRoot\agent.json" -Raw|ConvertFrom-Json
    if($beforeState.edition -ne $Edition -or $beforeState.package_kind -ne 'msi'){throw '安装版本类型与测试参数不匹配'}
    if(!$beforeState.management_agent_enabled -or $beforeState.disabled -or
        !(Test-Path -LiteralPath "$stateRoot\service-recovery-enabled")){throw '恢复未启用'}
    $baseline=Read-InstalledStatus
    if($baseline.application.state -ne 'ready' -or $baseline.intent.state -ne 'enabled' -or
        $baseline.agent_installation_id -ne $beforeState.agent_installation_id){throw '应用健康或身份基线不满足'}
    $hash=(Get-FileHash $agent).Hash
    if((Get-FileHash $bootstrap).Hash -ne $hash){throw '保底入口摘要不匹配'}
    $taskBefore=Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery
    [xml]$definition=$taskBefore
    $ns=New-Object Xml.XmlNamespaceManager($definition.NameTable)
    $ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
    $interval=$definition.SelectSingleNode('/t:Task/t:Triggers/t:TimeTrigger/t:Repetition/t:Interval',$ns)
    if(!$interval -or $interval.InnerText -ne 'PT5M'){throw '周期任务不是五分钟'}
    $budgetPath='C:\ProgramData\RustDesk\RecoveryBootstrap\budget.json'
    if(Test-Path -LiteralPath $budgetPath){
        $budget=Get-Content $budgetPath -Raw|ConvertFrom-Json
        if([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() -lt $budget.next_at){throw '恢复退避未到期，保留原预算'}
    }
    $bytes=[Text.Encoding]::UTF8.GetBytes($markerText)
    $handle=[IO.File]::Open($marker,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$handle.Write($bytes,0,$bytes.Length);$handle.Flush($true)}finally{$handle.Dispose()}
    $ownedMarker=$true
    Stop-Service RustDeskManagementAgent
    (Get-Service RustDeskManagementAgent).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
    Stop-Service RustDesk
    (Get-Service RustDesk).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(30))
    $stopped=Get-LabServices
    if(@($stopped|Where-Object {$_.State -ne 'Stopped' -or $_.ProcessId -ne 0}).Count){throw '双服务没有实际停止'}
    Invoke-RegisteredRecoveryTask
    Start-Sleep -Seconds 3
    if(((Get-LabServices)|ConvertTo-Json -Compress) -ne ($stopped|ConvertTo-Json -Compress) -or
        [IO.File]::ReadAllText($marker) -ne $markerText){throw '本地禁用未保留服务或标记'}
    $result.disabled_gate='PASS'
    $beforeRecovery=Get-RecoveryTaskSample
    [IO.File]::Delete($marker)
    $ownedMarker=$false
    $result.recovery_started=[DateTime]::UtcNow.ToString('o')
    $timer=[Diagnostics.Stopwatch]::StartNew()
    if($Trigger -eq 'Manual'){Invoke-RegisteredRecoveryTask}
    $taskCompleted=$false
    $running=$false
    # 五分钟周期 + 三分钟 agent 宽限 + 轮询和健康探测余量。
    $deadline=[DateTime]::UtcNow.AddSeconds(660)
    do {
        $sample=Get-RecoveryTaskSample
        if($sample.state -eq 4){$running=$true}
        if($sample.state -eq 3 -and ($sample.last_run -ne $beforeRecovery.last_run -or $running)){
            if($sample.exit_code -ne 0){throw "恢复任务失败：$($sample.exit_code)"}
            $taskCompleted=$true
        }
        $services=Get-LabServices
        if($taskCompleted -and @($services|Where-Object {$_.State -ne 'Running'}).Count -eq 0){break}
        Start-Sleep -Seconds 2
    }while([DateTime]::UtcNow -lt $deadline)
    if(!$taskCompleted -or @($services|Where-Object {$_.State -ne 'Running'}).Count){throw '限定窗口内未恢复双服务'}
    $result.recovery_seconds=$timer.Elapsed.TotalSeconds
    $status=Read-InstalledStatus
    if($status.application.state -ne 'ready' -or $status.agent_installation_id -ne $beforeState.agent_installation_id -or
        $status.package_identity.edition -ne $Edition){
        throw '恢复后应用未就绪或身份变化'
    }
    if((Get-FileHash $agent).Hash -ne $hash -or (Get-FileHash $bootstrap).Hash -ne $hash){throw '恢复修改了批准文件'}
    foreach($old in $result.before){
        $new=$services|Where-Object {$_.Name -eq $old.Name}
        if($new.PathName -ne $old.PathName -or $new.StartMode -ne $old.StartMode -or $new.StartName -ne $old.StartName){
            throw '恢复修改了服务配置'
        }
    }
    if((Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery) -ne $taskBefore){throw '任务定义变化'}
    $result.after=$services
    $result.application=$status.application
    $result.installation_id=$status.agent_installation_id
    $result.status='PASS'
}catch{$result.status='FAIL';$result.error=$_.Exception.Message}
finally {
    if($ownedMarker -and (Test-Path -LiteralPath $marker) -and [IO.File]::ReadAllText($marker) -eq $markerText){
        [IO.File]::Delete($marker)
        $result.owned_marker_removed=$true
    }
    $result.at=[DateTime]::UtcNow.ToString('o')
    $result|ConvertTo-Json -Depth 10|Set-Content "$prefix.result.json" -Encoding UTF8
}
if($result.status -ne 'PASS'){exit 1}
