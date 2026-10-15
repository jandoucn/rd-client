param([Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{8,64}$')][string]$RunId)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'guest-agent-acceptance-common.ps1')
$prefix=New-AcceptanceRun 'C:\RustDeskAgentLab' $RunId
$clock=[Diagnostics.Stopwatch]::StartNew()
$checks=New-Object System.Collections.ArrayList
$report=@{schema=1;run_id=$RunId;kind='installed_readonly';state='unknown';computer=$env:COMPUTERNAME;sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;
    started_utc=[DateTime]::UtcNow.ToString('o');driver_sha256=(Get-FileHash -LiteralPath $PSCommandPath).Hash;
    common_sha256=(Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'guest-agent-acceptance-common.ps1')).Hash;
    dimensions=@{scm='unknown';local_application='unknown';registration='unknown';remote_control='not_tested'};observations=@{};checks=@();executed=0}
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$dll='C:\Program Files\RustDesk\librustdesk.dll'
$bootstrap='C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe'
$manifest='C:\ProgramData\RustDesk\RecoveryBootstrap\bootstrap.json'
$stateRoot='C:\ProgramData\RustDesk\ManagementAgent\state'
$expectedAgent='d46fcbc3d4554fea8d8bdfa0166cd69c0597f09484210572d6e50280e543f423'
$expectedDll='4d0375fc508f3906836017f53f593a56c8443cdc3e99a0e209c4d7a1ff1ed23b'
function Require([bool]$Condition,[string]$Message){if(!$Condition){throw "ASSERTION: $Message"}}
function Check([string]$Name,[scriptblock]$Body){
    try {& $Body;[void]$checks.Add(@{name=$Name;state='pass'})}
    catch {$s=if($_.Exception.Message.StartsWith('ASSERTION:')){'fail'}else{'unknown'};[void]$checks.Add(@{name=$Name;state=$s;error=$_.Exception.Message})}
    $report.checks=@($checks);$report.executed=$checks.Count
    $report|ConvertTo-Json -Depth 14|Set-Content -LiteralPath "$prefix.progress.json" -Encoding UTF8
}
function Services {
    @(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" |
        Select-Object Name,State,StartMode,StartName,PathName,ProcessId|Sort-Object Name)
}
function ProcessIdentity([int]$ProcessId){
    $p=Get-Process -Id $ProcessId
    @{pid=$p.Id;created_filetime=$p.StartTime.ToUniversalTime().ToFileTimeUtc();path=$p.Path}
}
$before=$null;$taskBefore=$null;$identityBefore=$null;$probe=$null
try {
    Check 'installed hashes match explicit SOS baseline' {
        $hashes=@{agent=(Get-FileHash -LiteralPath $agent).Hash;dll=(Get-FileHash -LiteralPath $dll).Hash;bootstrap=(Get-FileHash -LiteralPath $bootstrap).Hash;manifest=(Get-FileHash -LiteralPath $manifest).Hash}
        $report.observations.hashes=$hashes
        Require ($hashes.agent -eq $expectedAgent -and $hashes.dll -eq $expectedDll -and $hashes.bootstrap -eq $expectedAgent) 'installed payload mismatch'
    }
    Check 'SCM contracts and process creation identities' {
        $script:before=Services;$report.observations.services_before=$before
        Require ($before.Count -eq 2) 'missing fixed service'
        foreach($s in $before){
            Require ($s.State -eq 'Running' -and $s.StartMode -eq 'Auto' -and $s.StartName -eq 'LocalSystem') "service contract $($s.Name)"
            $p=ProcessIdentity $s.ProcessId;$report.observations[$s.Name+'_process']=$p
            $expected=if($s.Name -eq 'RustDesk'){'C:\Program Files\RustDesk\RustDesk.exe'}else{$agent}
            Require ($p.path -eq $expected) 'service process executable mismatch'
            $expectedCommand=if($s.Name -eq 'RustDesk'){'"C:\Program Files\RustDesk\RustDesk.exe" --service'}else{'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk"'}
            Require ($s.PathName -eq $expectedCommand) 'service arguments mismatch'
        }
        $report.dimensions.scm='running'
    }
    Check 'installed identity and source package retained' {
        $s=[IO.File]::ReadAllText("$stateRoot\agent.json")|ConvertFrom-Json
        $script:identityBefore=$s.agent_installation_id
        $report.observations.state_identity=@{installation_id=$s.agent_installation_id;edition=$s.edition;package_kind=$s.package_kind}
        Require ($s.agent_installation_id -eq 'dcc73dd2fba22bf04c7df497053ae2df' -and $s.edition -eq 'sos' -and $s.package_kind -eq 'msi') 'baseline identity mismatch'
        $source='C:\RustDeskAgentLab\agent-sos-v187.msi'
        $report.observations.source_sha256=(Get-FileHash -LiteralPath $source).Hash
        Require ($report.observations.source_sha256 -eq 'b912548e12248e8950abae312afba156ac4b9aab7ee07b8c0a72b988855813a9') 'source package mismatch'
    }
    Check 'local blocking markers absent and recovery opt-in present' {
        $markers=@{};foreach($name in @('disable-agent','disable-recovery','maintenance.json','service-recovery-enabled')){$markers[$name]=Test-Path -LiteralPath (Join-Path $stateRoot $name)}
        $report.observations.markers=$markers
        Require (!$markers['disable-agent'] -and !$markers['disable-recovery'] -and !$markers['maintenance.json'] -and $markers['service-recovery-enabled']) 'local marker baseline mismatch'
    }
    Check 'recovery task exact executable SYSTEM triggers and manifest' {
        $script:taskBefore=Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery
        [IO.File]::WriteAllText("$prefix.recovery-task.xml",$taskBefore)
        [xml]$xml=$taskBefore;$ns=New-Object Xml.XmlNamespaceManager($xml.NameTable);$ns.AddNamespace('t','http://schemas.microsoft.com/windows/2004/02/mit/task')
        $principal=$xml.SelectSingleNode('//t:Principal/t:UserId',$ns).InnerText
        $command=$xml.SelectSingleNode('//t:Exec/t:Command',$ns).InnerText.Trim('"')
        $arguments=$xml.SelectSingleNode('//t:Exec/t:Arguments',$ns).InnerText
        Require ($principal -eq 'S-1-5-18' -and $command -eq $bootstrap -and $arguments -eq 'bootstrap --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk"') 'task identity/action mismatch'
        Require ($null -ne $xml.SelectSingleNode('//t:BootTrigger',$ns) -and $xml.SelectSingleNode('//t:Repetition/t:Interval',$ns).InnerText -eq 'PT5M') 'task triggers mismatch'
        Require ($xml.SelectSingleNode('//t:Principal/t:RunLevel',$ns).InnerText -eq 'HighestAvailable' -and $xml.SelectSingleNode('/t:Task/t:Settings/t:MultipleInstancesPolicy',$ns).InnerText -eq 'IgnoreNew') 'task execution settings mismatch'
        Require ((Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery).State -ne 'Disabled') 'recovery task disabled'
        $report.observations.task_execution_limit=$xml.SelectSingleNode('/t:Task/t:Settings/t:ExecutionTimeLimit',$ns).InnerText
        $m=[IO.File]::ReadAllText($manifest)|ConvertFrom-Json
        $report.observations.bootstrap_manifest=$m
        Require ($m.agent_sha256 -eq $expectedAgent -and $m.edition -eq 'sos' -and $m.package_kind -eq 'msi') 'bootstrap manifest mismatch'
    }
    Check 'installed status exits within monotonic deadline' {
        Require ((Get-FileHash -LiteralPath $agent).Hash -eq $expectedAgent) 'status executable is not the approved baseline'
        $script:probe=Invoke-AcceptanceProcess $agent 'status --json --edition sos --package-kind msi' "$prefix.status" 30000
        $report.observations.status_process=$probe
        if($probe.state -eq 'unknown'){throw $probe.error}
        Require ($probe.state -eq 'pass') 'status process nonzero exit'
        $raw=[IO.File]::ReadAllText($probe.stdout)
        $report.dimensions=Get-AcceptanceDimensions $raw
        $report.observations.installed=ConvertFrom-Json -InputObject $raw
    }
    Check 'local ready matches live process creation time and build' {
        $s=$report.observations.installed
        if($s.application.state -eq 'unknown'){throw 'APPLICATION_HEALTH_UNKNOWN'}
        Require ($s.application.state -eq 'ready') 'application not ready'
        $p=ProcessIdentity $s.application.pid;$report.observations.application_process=$p
        Require (Test-AcceptanceProcessIdentity $s.application.pid $s.application.process_created_at $p.pid $p.created_filetime) 'application process identity changed'
        Require ($p.path -eq 'C:\Program Files\RustDesk\RustDesk.exe' -and $s.application.build_seq -eq 2026100502 -and $s.agent.build_seq -eq 2026100502) 'application path/build mismatch'
        Require ($s.agent_installation_id -eq $identityBefore -and $s.intent.state -eq 'enabled' -and $s.recovery.effective_enabled -eq $true) 'SYSTEM identity/intent/recovery mismatch'
        Require ($s.client_id -eq '124504004' -and $s.client_uuid -eq 'ZGVlZWJkZGItNWZmYy00MzQ3LTk0MmYtODFiNzBhMzI0NTFj') 'client identity changed'
        Require ($s.package_identity.edition -eq 'sos' -and $s.package_identity.kind -eq 'msi' -and $s.operation.state -eq 'idle') 'installed status contract mismatch'
    }
    Check 'fixture services resume tasks and test processes absent' {
        $services=@(Get-CimInstance Win32_Service | Where-Object {$_.Name -match '^RustDesk.*(Test|Fixture)'})
        $tasks=@(Get-ScheduledTask | Where-Object {$_.TaskName -like 'RustDeskManagementAgentResume-*'})
        $processes=@(Get-CimInstance Win32_Process | Where-Object {$_.Name -match '^(php|helper)\.exe$' -or $_.Name -match '^management_agent.*\.exe$'})
        $report.observations.fixture_residue=@{services=@($services|Select-Object Name,State);tasks=@($tasks|Select-Object TaskName,State);processes=@($processes|Select-Object Name,ProcessId,CreationDate)}
        $report.observations.lab_tasks=@(Get-ScheduledTask|Where-Object TaskName -like 'RustDeskAgentLab*'|Select-Object TaskName,State)
        Require ($services.Count -eq 0 -and $tasks.Count -eq 0 -and $processes.Count -eq 0) 'fixture runtime residue'
    }
    Check 'event evidence availability recorded without changing log settings' {
        $channel='Microsoft-Windows-TaskScheduler/Operational'
        $log=Get-WinEvent -ListLog $channel
        $report.observations.task_log=@{enabled=$log.IsEnabled;record_count=$log.RecordCount;last_record_id=$null}
        if($log.RecordCount -gt 0){$last=Get-WinEvent -LogName $channel -MaxEvents 1;$report.observations.task_log.last_record_id=$last.RecordId}
        $system=Get-WinEvent -LogName System -MaxEvents 1
        $report.observations.system_last_record_id=$system.RecordId
        $report.observations.boot_attribution=@{state='unknown';reason='no_preboot_recordid_baseline_in_readonly_run'}
    }
    Check 'services task hashes and installation identity unchanged after observation' {
        $after=Services;$report.observations.services_after=$after
        Require (($before|ConvertTo-Json -Compress) -ceq ($after|ConvertTo-Json -Compress)) 'service snapshot changed'
        foreach($s in $after){$p=ProcessIdentity $s.ProcessId;$old=$report.observations[$s.Name+'_process'];Require (Test-AcceptanceProcessIdentity $old.pid $old.created_filetime $p.pid $p.created_filetime) 'service process reused'}
        Require ((Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery) -ceq $taskBefore) 'task definition changed'
        Require ((Get-FileHash -LiteralPath $agent).Hash -eq $expectedAgent -and (Get-FileHash -LiteralPath $dll).Hash -eq $expectedDll -and (Get-FileHash -LiteralPath $bootstrap).Hash -eq $expectedAgent) 'payload changed'
        Require ((Get-FileHash -LiteralPath $manifest).Hash -eq $report.observations.hashes.manifest) 'bootstrap manifest changed'
        Require (([IO.File]::ReadAllText("$stateRoot\agent.json")|ConvertFrom-Json).agent_installation_id -eq $identityBefore) 'installation identity changed'
    }
}catch{$report.fatal=$_.Exception.Message}
finally{
    $report.checks=@($checks);$report.executed=$checks.Count
    $report.passed=@($checks|Where-Object state -eq 'pass').Count
    $report.failed=@($checks|Where-Object state -eq 'fail').Count
    $report.unknown=@($checks|Where-Object state -eq 'unknown').Count
    $report.state=if($report.failed){'fail'}elseif($report.unknown -or $checks.Count -ne 10 -or $report.ContainsKey('fatal')){'unknown'}else{'pass'}
    $report.elapsed_ms=$clock.ElapsedMilliseconds;$report.finished_utc=[DateTime]::UtcNow.ToString('o')
    $report|ConvertTo-Json -Depth 14|Set-Content -LiteralPath "$prefix.result.json" -Encoding UTF8
}
if($report.state -ne 'pass'){exit 1}
