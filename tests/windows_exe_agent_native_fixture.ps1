param([Parameter(Mandatory=$true)][string]$Source)
$ErrorActionPreference='Stop'
if ($env:COMPUTERNAME -ne 'RDLAB-WIN10' -or (Get-CimInstance Win32_OperatingSystem).BuildNumber -ne '17763') { throw 'isolated Win10 required' }
if (!(New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'normal elevated token required' }
$mutex=[Threading.Mutex]::new($false,'Global\RustDeskWin10LegacyAgentFixture')
if (!$mutex.WaitOne(0)) { throw 'fixture already owned' }
$root=Join-Path $env:TEMP ('native-agent-'+[Guid]::NewGuid().ToString('N'))
$name='RustDeskFixture'+[Guid]::NewGuid().ToString('N')
$created=$false
$result=@{passed=$false;service=$name;root=$root}
try {
    New-Item -ItemType Directory $root | Out-Null
    $code=@'
using System;
using System.ServiceProcess;
using System.Threading;
public class FixtureService : ServiceBase {
    public FixtureService() { ServiceName=Environment.GetCommandLineArgs()[1]; CanStop=true; }
    protected override void OnStart(string[] args) { }
    protected override void OnStop() {
        if (System.IO.File.Exists(AppDomain.CurrentDomain.BaseDirectory+"pending")) {
            System.IO.File.WriteAllText(AppDomain.CurrentDomain.BaseDirectory+"pending-ready", "ready");
            while (!System.IO.File.Exists(AppDomain.CurrentDomain.BaseDirectory+"release-pending")) Thread.Sleep(20);
        }
        if (System.IO.File.Exists(AppDomain.CurrentDomain.BaseDirectory+"halt")) new Thread(() => {
            System.IO.File.WriteAllText(AppDomain.CurrentDomain.BaseDirectory+"waiting-exit", "ready");
            while (!System.IO.File.Exists(AppDomain.CurrentDomain.BaseDirectory+"release-exit")) Thread.Sleep(20);
        }).Start();
    }
    public static void Main() { ServiceBase.Run(new FixtureService()); }
}
'@
    Add-Type -TypeDefinition $code -ReferencedAssemblies System.ServiceProcess -OutputAssembly "$root\service.exe" -OutputType ConsoleApplication
    $digest=(Get-FileHash "$root\service.exe").Hash
    New-Service -Name $name -BinaryPathName ('"'+$root+'\service.exe" '+$name) -StartupType Manual | Out-Null
    $created=$true
    $text=[IO.File]::ReadAllText($Source)
    $body=$text.Split(@('const SCRIPT: &str = r###"'),[StringSplitOptions]::None)[1].Split(@('"###;'),[StringSplitOptions]::None)[0]
    # Only the isolated service name and timeout change; SCM/process code is real.
    $body=$body.Replace('RustDeskManagementAgent',$name).Replace("FromSeconds(90)","FromMilliseconds(200)").Replace('WaitForExit(30000)','WaitForExit(200)').Replace('C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe',"$root\service.exe").Replace("Name='rustdesk-management-agent.exe'","Name='service.exe'")
    Invoke-Expression $body
    Start-Agent
    $running=Get-AgentService
    if ($running.State -ne 'Running' -or $running.ProcessId -eq 0) { throw 'fixture did not run' }
    $old=[Diagnostics.Process]::GetProcessById([int]$running.ProcessId)
    $null=$old.Handle
    Stop-Agent
    if (!$old.HasExited -or (Get-AgentService).State -ne 'Stopped') { throw 'SCM stop did not wait for PID exit' }
    $old.Dispose()
    $result.stop_and_pid_exit=$true
    Stop-Agent
    $result.already_stopped_verified=$true
    [IO.File]::WriteAllText("$root\pending",'pending')
    Start-Agent
    (Get-Service $name).Stop()
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do { $pending=Get-AgentService; if ($pending.State -eq 'Stop Pending' -and (Test-Path "$root\pending-ready")) { break }; Start-Sleep -Milliseconds 20 } while ([DateTime]::UtcNow -lt $deadline)
    if ($pending.State -ne 'Stop Pending') { throw 'StopPending fixture not established' }
    $failure=''
    try { Stop-Agent } catch { $failure=$_.Exception.Message }
    if (!$failure.StartsWith('[update-pending]')) { throw 'StopPending not withheld' }
    [IO.File]::WriteAllText("$root\release-pending",'release')
    (Get-Service $name).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(10))
    Stop-Agent
    $result.stop_pending_release_verified=$true
    Remove-Item "$root\pending"
    foreach ($fault in @('copyfail','newstartfail')) {
        [IO.File]::WriteAllText("$root\main-old",'main old bytes')
        [IO.File]::WriteAllText("$root\main",'new main bytes')
        Copy-Checked "$root\service.exe" "$root\agent-backup.exe"
        $failure=''
        if ($fault -eq 'copyfail') {
            $locked=[IO.File]::Open("$root\service.exe",'Open','Read','Read')
            try { Copy-Checked "$root\main" "$root\service.exe" } catch { $failure='copyfail' } finally { $locked.Dispose() }
        } else {
            Copy-Checked "$root\main" "$root\service.exe"
            try { Start-Agent } catch { $failure='newstartfail' }
        }
        if ($failure -ne $fault) { throw 'native failure injection did not execute' }
        Stop-Agent
        Copy-Checked "$root\agent-backup.exe" "$root\service.exe"
        Copy-Checked "$root\main-old" "$root\main"
        Start-Agent
        if ((Get-Digest "$root\service.exe") -ne $digest.ToLowerInvariant() -or [IO.File]::ReadAllText("$root\main") -ne 'main old bytes') { throw 'native rollback bytes lost' }
        Stop-Agent
        $result[('stopped_'+$fault+'_rollback')]=$true
    }
    Start-Agent
    [IO.File]::WriteAllText("$root\halt",'halt')
    $haltProcess=[Diagnostics.Process]::GetProcessById([int](Get-AgentService).ProcessId)
    $null=$haltProcess.Handle
    $failure=''
    try { Stop-Agent } catch { $failure=$_.Exception.Message }
    if (!$failure.StartsWith('[update-pending]')) { throw "halt not pending: $failure" }
    if ((Get-FileHash "$root\service.exe").Hash -ne $digest) { throw 'halt changed executable' }
    (Get-Service $name).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(15))
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do { $stopped=Get-AgentService; if ($stopped.State -eq 'Stopped' -and $stopped.ProcessId -eq 0 -and (Test-Path "$root\waiting-exit")) { break }; Start-Sleep -Milliseconds 20 } while ([DateTime]::UtcNow -lt $deadline)
    if ($stopped.State -ne 'Stopped' -or $stopped.ProcessId -ne 0 -or $haltProcess.HasExited) { throw 'Stopped/PID0/live fixture not established' }
    $failure=''
    try { Stop-Agent } catch { $failure=$_.Exception.Message }
    if (!$failure.StartsWith('[update-pending]')) { throw 'Stopped PID0 live image falsely accepted' }
    $result.stopped_pid_zero_live_pending=$true
    [IO.File]::WriteAllText("$root\release-exit",'release')
    if (!$haltProcess.WaitForExit(10000)) { throw 'released fixture failed to exit' }
    $haltProcess.Dispose()
    Stop-Agent
    $result.released_image_allows_stop=$true
    $result.halt_pending_old_bytes=$true
    [IO.File]::WriteAllText("$root\old",'old')
    [IO.File]::WriteAllText("$root\new",'new')
    $locked=[IO.File]::Open("$root\old",'Open','Read','Read')
    try {
        if (Test-Path "$root\pending") { [IO.File]::WriteAllText("$root\release-pending",'cleanup release') }
        if (Test-Path "$root\halt") {
            [IO.File]::WriteAllText("$root\release-exit",'cleanup release')
            foreach ($p in Get-Process service -ErrorAction SilentlyContinue) {
                if ($p.Path -eq "$root\service.exe" -and !$p.WaitForExit(10000)) { throw 'fixture cleanup exit unconfirmed' }
            }
        }
        if ($haltProcess) { $haltProcess.Dispose() }
        $failed=$false
        try { Copy-Checked "$root\new" "$root\old" } catch { $failed=$true }
        if (!$failed -or [IO.File]::ReadAllText("$root\old") -ne 'old') { throw 'locked copy lost original' }
    } finally { $locked.Dispose() }
    $result.locked_copy_old_bytes=$true
    $bootstrap=Join-Path $root 'bootstrap'
    Sync-Bootstrap $bootstrap "$root\service.exe" $digest.ToLowerInvariant() 'standard' "$root\normal" $false
    $manifest=Get-Content "$bootstrap\bootstrap.json" -Raw | ConvertFrom-Json
    if ($manifest.agent_sha256 -ne $digest.ToLowerInvariant()) { throw 'native bootstrap manifest hash mismatch' }
    $result.bootstrap_create_verified=$true
    Sync-Bootstrap $bootstrap "$root\service.exe" $digest.ToLowerInvariant() 'standard' "$root\normal" $true
    $result.bootstrap_existing_verified=$true
    $result.passed=$true
} catch { $result.error=$_.Exception.Message }
finally {
    try {
        $task=Get-ScheduledTask -TaskName ($name+'Recovery') -ErrorAction SilentlyContinue
        if ($task) { Unregister-ScheduledTask -TaskName ($name+'Recovery') -Confirm:$false -ErrorAction Stop }
        if ($created) {
            Stop-Service $name -ErrorAction Stop
            (Get-Service $name).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(20))
            & "$env:SystemRoot\System32\sc.exe" delete $name | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'fixture service delete failed' }
        }
        if (Test-Path $root) { Remove-Item $root -Recurse -Force }
    } catch { $result.passed=$false; $result.cleanup_error=$_.Exception.Message }
    $result | ConvertTo-Json -Depth 5 | Set-Content "$PSScriptRoot\legacy-native-result.json" -Encoding UTF8
    $mutex.ReleaseMutex();$mutex.Dispose()
}
if (!$result.passed) { throw ($result | ConvertTo-Json -Compress) }
