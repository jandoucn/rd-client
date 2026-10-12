param([Parameter(Mandatory=$true)][string]$Package,
      [Parameter(Mandatory=$true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$PackageSha256,
      [Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{1,48}$')][string]$RunId)
$ErrorActionPreference='Stop'
$root='C:\RustDeskAgentLab'
if($env:COMPUTERNAME -notin @('RDLAB-WIN10','RDLAB-WIN11') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'SYSTEM lab VM required'}
if([IO.Path]::GetDirectoryName($Package) -ne $root -or [IO.Path]::GetExtension($Package) -ne '.msi'){throw 'Only a package directly in the lab directory is permitted'}
$prefix=Join-Path $root $RunId
$fixture="$prefix.msi"
if((Test-Path $fixture) -or (Test-Path "$prefix.result.json")){throw 'Run already exists; preserve evidence'}
$result=@{run=$RunId;computer=$env:COMPUTERNAME;status='FAIL';started=[DateTime]::UtcNow.ToString('o')}
function Query-Msi($Database,$Query,[bool]$Integer=$false){
    $view=$Database.OpenView($Query)
    $record=$null
    try{[void]$view.Execute();$record=$view.Fetch();if($record){if($Integer){return $record.IntegerData(1)};return $record.StringData(1)}}finally{
        if($record){[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record)}
        [void]$view.Close();[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    }
}
function Exec-Msi($Database,$Query){
    $view=$Database.OpenView($Query)
    try{[void]$view.Execute()}finally{[void]$view.Close();[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)}
}
try {
    if((Get-FileHash $Package).Hash -ne $PackageSha256){throw 'Source package digest mismatch'}
    $agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
    $entry='C:\ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe'
    $manifest='C:\ProgramData\RustDesk\RecoveryBootstrap\bootstrap.json'
    $statePath='C:\ProgramData\RustDesk\ManagementAgent\state\agent.json'
    $beforeState=Get-Content $statePath -Raw|ConvertFrom-Json
    $paths=@($agent,$entry,$manifest,'C:\Program Files\RustDesk\librustdesk.dll')
    $beforeHashes=@{}
    foreach($path in $paths){$beforeHashes[$path]=(Get-FileHash $path).Hash}
    if($beforeHashes[$agent] -ne $beforeHashes[$entry]){throw 'Healthy bootstrap baseline required'}
    $taskBefore=Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery
    $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,PathName,StartMode,StartName)
    if($services.Count -ne 2 -or @($services|Where-Object {$_.State -ne 'Running'}).Count){throw 'Running service baseline required'}
    $installer=New-Object -ComObject WindowsInstaller.Installer
    $originalCode='{1A718E63-99A7-49AA-8D4F-5DDD2031646D}'
    if($installer.ProductState($originalCode) -ne 5){throw 'Verified old MSI baseline required'}
    [IO.File]::Copy($Package,$fixture,$false)
    $database=$installer.OpenDatabase($fixture,1)
    $targetCode=Query-Msi $database 'SELECT `Value` FROM `Property` WHERE `Property`=''ProductCode'''
    if($targetCode -eq $originalCode){throw 'Target product must differ from original'}
    $sequence=[int](Query-Msi $database 'SELECT `Sequence` FROM `InstallExecuteSequence` WHERE `Action`=''InstallFinalize''' $true)
    $files=[int](Query-Msi $database 'SELECT `Sequence` FROM `InstallExecuteSequence` WHERE `Action`=''InstallFiles''' $true)
    if($sequence -le $files){throw 'InstallFinalize is not after file installation'}
    $view=$database.OpenView('SELECT `Action`,`Sequence` FROM `InstallExecuteSequence` WHERE `Sequence` >= '+$sequence)
    [void]$view.Execute()
    $rows=@()
    while($row=$view.Fetch()){
        $rows+=@{action=$row.StringData(1);sequence=$row.IntegerData(2)}
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($row)
    }
    [void]$view.Close()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    foreach($row in ($rows|Sort-Object sequence -Descending)){
        $shifted=$row.sequence+2
        if($shifted -gt 32767){throw 'Fixture sequence overflow'}
        $action=$row.action.Replace("'","''")
        Exec-Msi $database "UPDATE ``InstallExecuteSequence`` SET ``Sequence``=$shifted WHERE ``Action``='$action'"
    }
    $position=$sequence+1
    # Execute the queued installation script before the immediate error action.
    $flushPosition=$sequence
    if(Query-Msi $database ('SELECT `Action` FROM `InstallExecuteSequence` WHERE `Sequence`='+$flushPosition)){throw 'Fixture flush sequence occupied'}
    if(Query-Msi $database 'SELECT `Action` FROM `InstallExecuteSequence` WHERE `Action`=''InstallExecuteAgain'''){throw 'Fixture flush action already exists'}
    Exec-Msi $database "INSERT INTO ``InstallExecuteSequence`` (``Action``,``Condition``,``Sequence``) VALUES ('InstallExecuteAgain','1',$flushPosition)"
    Exec-Msi $database "INSERT INTO ``CustomAction`` (``Action``,``Type``,``Target``) VALUES ('LabFailAfterExecute',19,'Local fixture failure after installation script')"
    Exec-Msi $database "INSERT INTO ``InstallExecuteSequence`` (``Action``,``Condition``,``Sequence``) VALUES ('LabFailAfterExecute','1',$position)"
    $database.Commit()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($database)
    $result.fixture_sha256=(Get-FileHash $fixture).Hash
    $result.original_product=$originalCode;$result.target_product=$targetCode
    $process=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList ('/i "'+$fixture+'" /qn /norestart REBOOT=ReallySuppress LAUNCH_TRAY_APP=N DEFENDER_EXCLUSIONS=0 /L*v "'+$prefix+'.install.log"') -PassThru
    $null=$process.Handle
    if(!$process.WaitForExit(600000)){throw 'Installer timeout; preserve scene'}
    $process.WaitForExit();$result.installer_exit=$process.ExitCode
    if($process.ExitCode -ne 1603){throw 'Expected injected installation failure'}
    $log=Get-Content "$prefix.install.log" -Raw
    if($log -notmatch 'LabFailAfterExecute' -or $log -notmatch 'SourceCabKey=RecoveryBootstrap.Exe' -or $log -notmatch 'SourceCabKey=RecoveryBootstrap.Manifest'){
        throw 'Failure occurred before bootstrap replacement, not a valid rollback test'
    }
    $result.after_hashes=@{}
    foreach($path in $paths){
        $actual=(Get-FileHash $path).Hash;$result.after_hashes[$path]=$actual
        if($actual -ne $beforeHashes[$path]){throw "Rollback did not restore: $path"}
    }
    if($installer.ProductState($originalCode) -ne 5 -or $installer.ProductState($targetCode) -eq 5){throw 'Product registration was not rolled back'}
    if((Get-Content $statePath -Raw|ConvertFrom-Json).agent_installation_id -ne $beforeState.agent_installation_id){throw 'Installation identity changed'}
    if((Export-ScheduledTask -TaskName RustDeskManagementAgentRecovery) -ne $taskBefore){throw 'Recovery task changed'}
    $recoveryStarted=[DateTime]::UtcNow
    $recoveryDeadline=$recoveryStarted.AddSeconds(300)
    do {
        $after=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,PathName,StartMode,StartName)
        if($after.Count -eq 2 -and @($after|Where-Object {$_.State -ne 'Running'}).Count -eq 0){break}
        Start-Sleep -Seconds 2
    }while([DateTime]::UtcNow -lt $recoveryDeadline)
    $result.recovery_seconds=([DateTime]::UtcNow-$recoveryStarted).TotalSeconds
    foreach($old in $services){
        $current=$after|Where-Object {$_.Name -eq $old.Name}
        if(!$current -or $current.State -ne 'Running' -or $current.PathName -ne $old.PathName -or $current.StartMode -ne $old.StartMode -or $current.StartName -ne $old.StartName){throw 'Service was not restored'}
    }
    $p=Start-Process $entry -ArgumentList 'bootstrap --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk"' -Wait -PassThru -RedirectStandardOutput "$prefix.probe.stdout.txt" -RedirectStandardError "$prefix.probe.stderr.txt"
    $result.bootstrap_exit=$p.ExitCode
    if($p.ExitCode -ne 0){throw 'Restored bootstrap cannot run'}
    $deadline=[DateTime]::UtcNow.AddSeconds(60)
    do {
        $p=Start-Process $agent -ArgumentList 'status --json --edition standard --package-kind msi' -Wait -PassThru -RedirectStandardOutput "$prefix.status.json" -RedirectStandardError "$prefix.status.stderr.txt"
        if($p.ExitCode -ne 0){throw 'Restored agent status failed'}
        $health=Get-Content "$prefix.status.json" -Raw|ConvertFrom-Json
        if($health.application.state -eq 'ready'){break}
        Start-Sleep -Seconds 2
    }while([DateTime]::UtcNow -lt $deadline)
    $result.application=$health.application
    if($health.application.state -ne 'ready' -or $health.agent_installation_id -ne $beforeState.agent_installation_id){throw 'Rollback restored files but not application readiness or identity'}
    $result.status='PASS'
}catch{$result.error=$_.Exception.Message;$result.position=$_.InvocationInfo.PositionMessage;$result.stack=$_.ScriptStackTrace}
finally{$result.at=[DateTime]::UtcNow.ToString('o');$result|ConvertTo-Json -Depth 8|Set-Content "$prefix.result.json" -Encoding UTF8}
if($result.status -ne 'PASS'){exit 1}
