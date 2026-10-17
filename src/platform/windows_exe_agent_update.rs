use hbb_common::{
    anyhow::anyhow,
    base64::{engine::general_purpose::STANDARD, Engine as _},
    ResultType,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

// The legacy caller has already verified the downloaded outer package. Bind the
// agent beside its extracted runner, never a caller-supplied replacement path.
pub fn commands(body: &str, source: &str, installed: &str, backup: &str) -> ResultType<String> {
    for value in [source, installed, backup] {
        super::validate_install_value(value)?;
    }
    let target = Path::new(source)
        .parent()
        .ok_or_else(|| anyhow!("extracted update directory missing"))?
        .join("rustdesk-management-agent.exe");
    let mut file = std::fs::File::open(&target)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json"))?;
    let build = metadata["build_seq"]
        .as_u64()
        .ok_or_else(|| anyhow!("update build missing"))?;
    let edition = if option_env!("RUSTDESK_SOS") == Some("1") {
        "sos"
    } else {
        "standard"
    };
    let literal = |value: &str| value.replace('\'', "''");
    // Retain the main backup until the outer transaction has verified both builds.
    let body = body.replace(
        &format!("rd /s /q \"{backup}\""),
        "rem joint transaction retains backup",
    );
    let body = body.replace(
        "if errorlevel 8 goto update_backup_failed",
        "if errorlevel 8 goto update_backup_failed\r\n> \"%~dp0main-backup-ready\" echo ready",
    );
    let body = format!("{body}\r\nexit /b 0\r\n");
    let script = format!(
        "{SCRIPT}\ntry {{ Invoke-LegacyAgentUpdate '{}' '{}' '{}' '{}' '{}' {} '{}' $PSScriptRoot; exit 0 }} catch {{ [Console]::Error.WriteLine($_); if ($_.Exception.Message.StartsWith('[update-pending]')) {{ exit 75 }}; if ($_.Exception.Message.StartsWith('[update-rollback-failed]')) {{ exit 76 }}; if ($_.Exception.Message.StartsWith('[update-rolled-back]')) {{ exit 78 }}; exit 77 }}",
        literal(&target.to_string_lossy()), literal(installed), literal(backup),
        hex::encode(hash.finalize()), edition, build, STANDARD.encode(body.as_bytes())
    );
    let encoded = STANDARD.encode(script.as_bytes());
    let mut result = String::new();
    for (index, chunk) in encoded.as_bytes().chunks(120).enumerate() {
        result.push_str(&format!(
            "{} \"%~f0.agent-update.b64\" echo {}\r\n",
            if index == 0 { ">" } else { ">>" },
            String::from_utf8_lossy(chunk)
        ));
    }
    result.push_str("certutil -f -decode \"%~f0.agent-update.b64\" \"%~f0.agent-update.ps1\" >nul\r\nif errorlevel 1 exit /b 76\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~f0.agent-update.ps1\"\r\nexit /b %ERRORLEVEL%\r\n");
    Ok(result)
}

const SCRIPT: &str = r###"
function Confirm-LegacyInstallationIdentity($Old, $Disk, $Status, $Edition, [string]$Bound) {
    $hasIdentity = $null -ne $Old.PSObject.Properties['agent_installation_id']
    if ($hasIdentity -and $Old.agent_installation_id -cnotmatch '^[0-9a-f]{32}$') { throw 'invalid existing installation identity' }
    $observed = [string]$Status.agent_installation_id
    if ($observed -cnotmatch '^[0-9a-f]{32}$' -or $Disk.agent_installation_id -cne $observed -or $Disk.status_session_generation -lt 1 -or $Status.session_generation -ne $Disk.status_session_generation) { throw 'initialized installation identity unconfirmed' }
    if ($Disk.edition -ne $Edition -or $Disk.package_kind -ne 'exe' -or $Status.package_identity.edition -ne $Edition -or $Status.package_identity.kind -ne 'exe') { throw 'initialized package identity mismatch' }
    if (($hasIdentity -and $Old.agent_installation_id -cne $observed) -or ($Bound -and $Bound -cne $observed)) { throw 'installation identity changed' }
    $observed
}
function Assert-PlainPath([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    while ($null -ne $item) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'reparse path rejected' }
        if ($item -is [IO.FileInfo]) { $item = $item.Directory } else { $item = $item.Parent }
    }
}
function Get-Digest([string]$Path) {
    Assert-PlainPath $Path
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
}
function Assert-PlainTree([string]$Path) {
    Assert-PlainPath $Path
    foreach ($entry in Get-ChildItem -LiteralPath $Path -Force) {
        Assert-PlainPath $entry.FullName
        if ($entry.PSIsContainer) { Assert-PlainTree $entry.FullName }
    }
}
function Get-AgentService {
    $s = Get-CimInstance Win32_Service -Filter "Name='RustDeskManagementAgent'" -ErrorAction Stop
    if (!$s) { throw 'agent SCM missing' }
    $s
}
function Stop-Agent {
    $images = @()
    $fixed = 'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
    try {
        foreach ($candidate in Get-CimInstance Win32_Process -Filter "Name='rustdesk-management-agent.exe'" -ErrorAction Stop) {
            if (!$candidate.ExecutablePath -or !$candidate.CreationDate) { throw 'candidate identity unavailable' }
            if (![string]::Equals($candidate.ExecutablePath,$fixed,[StringComparison]::OrdinalIgnoreCase)) { continue }
            $p = [Diagnostics.Process]::GetProcessById([int]$candidate.ProcessId)
            $images += $p
            $null = $p.Handle
            $created = $p.StartTime.ToUniversalTime()
            if (![string]::Equals($p.MainModule.FileName,$fixed,[StringComparison]::OrdinalIgnoreCase) -or [Math]::Abs(($created-$candidate.CreationDate.ToUniversalTime()).TotalMilliseconds) -ge 1) { throw 'candidate identity changed' }
        }
    $s = Get-AgentService
    $owned = $null
    if ($s.ProcessId -ne 0) {
        $owned = [Diagnostics.Process]::GetProcessById([int]$s.ProcessId)
        $null = $owned.Handle
    }
    try {
        switch ($s.State) {
            'Running' { (Get-Service RustDeskManagementAgent).Stop() }
            'Stop Pending' { }
            'Stopped' { }
            default { throw '[update-pending] agent state cannot be stopped safely' }
        }
        try { (Get-Service RustDeskManagementAgent).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(90)) }
        catch { throw '[update-pending] agent stop unconfirmed' }
        if ($owned -and !$owned.WaitForExit(30000)) { throw '[update-pending] agent process exit unconfirmed' }
    } finally { if ($owned) { $owned.Dispose() } }
        foreach ($p in $images) {
            if (!$p.WaitForExit(30000)) { throw '[update-pending] resident image exit unconfirmed' }
        }
        foreach ($candidate in Get-CimInstance Win32_Process -Filter "Name='rustdesk-management-agent.exe'" -ErrorAction Stop) {
            if (!$candidate.ExecutablePath -or [string]::Equals($candidate.ExecutablePath,$fixed,[StringComparison]::OrdinalIgnoreCase)) { throw '[update-pending] resident image appeared or identity unavailable' }
        }
    } catch {
        if ($_.Exception.Message.StartsWith('[update-pending]')) { throw }
        throw '[update-pending] agent stop or process identity unconfirmed'
    } finally { foreach ($p in $images) { $p.Dispose() } }
}
function Start-Agent {
    Start-Service RustDeskManagementAgent -ErrorAction Stop
    (Get-Service RustDeskManagementAgent).WaitForStatus('Running',[TimeSpan]::FromSeconds(60))
}
function Save-MainIdentityDiagnostic($Diagnostic) {
    try {
        $path = Join-Path $Work ('legacy-main-identity-'+$Diagnostic.process_id+'.json')
        [IO.File]::WriteAllText($path,($Diagnostic | ConvertTo-Json -Depth 4))
    } catch { [Console]::Error.WriteLine('main identity diagnostic persistence failed; original failure retained') }
}
function Stop-Main([string]$Installed) {
    Stop-Service RustDesk -ErrorAction Stop
    try { (Get-Service RustDesk).WaitForStatus('Stopped',[TimeSpan]::FromSeconds(60)) }
    catch { throw '[update-pending] main stop unconfirmed' }
    foreach ($process in Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'") {
        if ($process.ExecutablePath -eq $Installed) {
            $identityDiagnostic = [ordered]@{process_id=$process.ProcessId;cim_creation_ticks=$null;start_ticks=$null;path_matches=$null;has_exited=$null;read_error=$null}
            if ($process.CreationDate) { $identityDiagnostic.cim_creation_ticks=[string]$process.CreationDate.ToUniversalTime().Ticks }
            try { $owned = [Diagnostics.Process]::GetProcessById([int]$process.ProcessId) }
            catch {
                $inner = $_.Exception.InnerException
                if ($inner -isnot [ArgumentException]) { throw '[update-pending] main process handle unavailable' }
                Confirm-MainProcessGone $process.ProcessId
                continue
            }
            try {
                $null = $owned.Handle
                $hasExited = $owned.HasExited
                $identityDiagnostic.has_exited=$hasExited
                if ($hasExited) {
                    Save-MainIdentityDiagnostic $identityDiagnostic
                    Confirm-MainProcessGone $process.ProcessId
                    continue
                }
                try {
                    $start = $owned.StartTime.ToUniversalTime()
                    $modulePath = $owned.MainModule.FileName
                    $identityDiagnostic.start_ticks=[string]$start.Ticks
                    $timeMismatch = !$process.CreationDate -or [Math]::Abs(($start-$process.CreationDate.ToUniversalTime()).TotalMilliseconds) -ge 1
                    $pathMatches = [string]::Equals($modulePath,$Installed,[StringComparison]::OrdinalIgnoreCase)
                    $identityDiagnostic.path_matches=$pathMatches
                    if ($timeMismatch -or !$pathMatches) { throw '[update-pending] main process identity changed' }
                } catch {
                    $identityDiagnostic.read_error=$_.Exception.GetType().FullName
                    Save-MainIdentityDiagnostic $identityDiagnostic
                    if ($owned.HasExited) {
                        Confirm-MainProcessGone $process.ProcessId
                        continue
                    }
                    throw '[update-pending] main process identity changed'
                }
                if (!$owned.HasExited) { $owned.Kill() }
                if (!$owned.WaitForExit(30000)) { throw '[update-pending] main process exit unconfirmed' }
                Confirm-MainProcessGone $process.ProcessId
            } finally { $owned.Dispose() }
        }
    }
    $remaining = @(Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'" -ErrorAction Stop)
    if (@($remaining | Where-Object { !$_.ExecutablePath -or $_.ExecutablePath -eq $Installed }).Count) { throw '[update-pending] main process remains or identity unavailable' }
}
function Confirm-MainProcessGone([int]$ProcessId) {
    $remaining = @(Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop)
    if ($remaining.Count) { throw '[update-pending] main process remains or identity unavailable' }
}
function Invoke-Bounded([string]$File, [string]$Arguments, [string]$Output) {
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -PassThru -NoNewWindow -RedirectStandardOutput $Output
    try {
        $null = $p.Handle
        if (!$p.WaitForExit(180000)) { throw '[update-pending] child completion unconfirmed' }
        $p.WaitForExit()
        if ($p.ExitCode -ne 0) { throw ('child failed: '+$p.ExitCode) }
    } finally { $p.Dispose() }
}
function Copy-Checked([string]$Source, [string]$Destination) {
    $expected = Get-Digest $Source
    Copy-Item -LiteralPath $Source -Destination $Destination -Force -ErrorAction Stop
    if ((Get-Digest $Destination) -ne $expected) { throw 'copy hash mismatch' }
}
function Copy-TreeChecked([string]$Source, [string]$Destination) {
    Assert-PlainTree $Source
    Copy-Item -LiteralPath $Source -Destination $Destination -Recurse -Force -ErrorAction Stop
    foreach ($file in Get-ChildItem -LiteralPath $Source -Recurse -Force -File) {
        $relative = $file.FullName.Substring($Source.TrimEnd('\').Length).TrimStart('\')
        if ((Get-Digest $file.FullName) -ne (Get-Digest (Join-Path $Destination $relative))) { throw 'backup tree hash mismatch' }
    }
}
function Sync-Bootstrap($Directory, $Target, $Hash, $Edition, $Normal, $Existing) {
    $entry = "$Directory\rustdesk-recovery-bootstrap.exe"
    $arguments = 'bootstrap --edition '+$Edition+' --package-kind exe --normal-install-path "'+$Normal+'"'
    $task = Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery -ErrorAction SilentlyContinue
    if ($Existing) {
        if (!$task -or $task.Actions.Count -ne 1 -or $task.Actions[0].Execute -ne $entry -or $task.Actions[0].Arguments -ne $arguments) { throw 'bootstrap task mismatch' }
    } else {
        if ($task) { throw 'bootstrap task already exists' }
        New-Item -ItemType Directory -Path $Directory | Out-Null
        & "$env:SystemRoot\System32\icacls.exe" $Directory /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'bootstrap ACL failed' }
    }
    Copy-Checked $Target $entry
    $manifest = @{schema=1;agent_sha256=$Hash;edition=$Edition;package_kind='exe';normal_install_path=$Normal}
    [IO.File]::WriteAllText("$Directory\bootstrap.json",($manifest | ConvertTo-Json))
    if (!$Existing) {
        $xmlPath = [Security.SecurityElement]::Escape($entry)
        $xmlArgs = [Security.SecurityElement]::Escape($arguments)
        $xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<Triggers><BootTrigger><Enabled>true</Enabled><Delay>PT1M</Delay></BootTrigger><TimeTrigger><Repetition><Interval>PT5M</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2026-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
<Principals><Principal id="System"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><ExecutionTimeLimit>PT1M</ExecutionTimeLimit><Enabled>true</Enabled></Settings>
<Actions Context="System"><Exec><Command>$xmlPath</Command><Arguments>$xmlArgs</Arguments></Exec></Actions></Task>
"@
        Register-ScheduledTask -TaskName RustDeskManagementAgentRecovery -Xml $xml -ErrorAction Stop | Out-Null
    }
    $task = Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery -ErrorAction Stop
    if ($task.Actions.Count -ne 1 -or $task.Actions[0].Execute -ne $entry -or $task.Actions[0].Arguments -ne $arguments -or $task.Principal.UserId -notin @('SYSTEM','S-1-5-18') -or (Get-Digest $entry) -ne $Hash) { throw 'bootstrap readback failed' }
}
function Invoke-LegacyAgentUpdate($Target, $Installed, $MainBackup, $Hash, $Edition, $Build, $Body, $Work) {
    $ErrorActionPreference = 'Stop'
    $productRoot = 'C:\ProgramData\RustDesk'
    $agent = "$productRoot\ManagementAgent\rustdesk-management-agent.exe"
    $state = "$productRoot\ManagementAgent\state"
    $bootstrap = "$productRoot\RecoveryBootstrap"
    $config = "$env:SystemRoot\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config"
    $normal = Split-Path -Parent $Installed
    if ($normal -notin @('C:\Program Files\RustDesk','C:\Program Files (x86)\RustDesk')) { throw 'normal install path rejected' }
    foreach ($path in @($Target,$agent,$state,$config,$normal,$Work)) { Assert-PlainPath $path }
    $lock = [Threading.Mutex]::new($false,'Global\RustDeskManagementAgentInstall')
    $held = $false
    $targetLock = $null
    $finalStateLock = $null
    $saved = $false
    $stopped = $false
    $mainStarted = $false
    $mainStopped = $false
    $bootstrapStarted = $false
    $committed = $false
    $maintenanceOwned = $false
    $backup = Join-Path $Work ('agent-backup-'+[Guid]::NewGuid().ToString('N'))
    try {
        $held = $lock.WaitOne(0)
        if (!$held) { throw 'installation in progress' }
        foreach ($marker in @('disable-agent','disable-recovery','maintenance.json')) {
            if (Test-Path -LiteralPath "$state\$marker") { throw 'local update gate' }
        }
        $oldState = Get-Content -LiteralPath "$state\agent.json" -Raw | ConvertFrom-Json
        if ($oldState.disabled -or !$oldState.management_agent_enabled) { throw 'agent locally disabled' }
        if ($oldState.edition -ne $Edition -or $oldState.package_kind -ne 'exe') { throw 'agent package mismatch' }
        $legacyIdentity = $null -eq $oldState.PSObject.Properties['agent_installation_id']
        if (!$legacyIdentity -and $oldState.agent_installation_id -cnotmatch '^[0-9a-f]{32}$') { throw 'invalid existing installation identity' }
        if ($legacyIdentity -and (($null -ne $oldState.PSObject.Properties['schema_version'] -and (($oldState.schema_version -isnot [int] -and $oldState.schema_version -isnot [long]) -or $oldState.schema_version -ne 1)) -or $oldState.target_key -ne ('windows-x86_64-exe-'+$Edition))) { throw 'legacy state contract mismatch' }
        if (!(Test-Path -LiteralPath $bootstrap) -and (Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery -ErrorAction SilentlyContinue)) { throw 'bootstrap task exists without manifest' }
        $intent = Get-Content -LiteralPath "$config\RustDesk2.toml" -Raw
        if ($intent -match '(?m)^\s*stop-service\s*=' -and $intent -notmatch '(?m)^\s*stop-service\s*=\s*["''](?:N)?["'']\s*$') { throw 'stop intent or unknown intent' }
        $s = Get-AgentService
        $expected = '"'+$agent+'" --service --edition '+$Edition+' --package-kind exe --normal-install-path "'+$normal+'"'
        if ($s.PathName -ne $expected -or $s.StartName -ne 'LocalSystem' -or $s.StartMode -ne 'Auto' -or $s.State -ne 'Running') { throw 'agent SCM contract mismatch' }
        $targetLock = [IO.File]::Open($Target,'Open','Read','Read')
        if ((Get-Digest $Target) -ne $Hash) { throw 'target agent hash mismatch' }
        if ((Get-Service RustDesk).Status -ne 'Running') { throw 'main service not running; preserve user intent' }
        $identityBefore = @{}
        foreach ($name in @('RustDesk.toml','RustDesk2.toml')) { $identityBefore[$name] = Get-Digest "$config\$name" }
        $maintenance = [IO.File]::Open("$state\maintenance.json",'CreateNew','Write','None')
        try {
            $bytes = [Text.Encoding]::UTF8.GetBytes((@{owner='legacy-exe-update';backup=$backup;build=$Build;agent_sha256=$Hash} | ConvertTo-Json))
            $maintenance.Write($bytes,0,$bytes.Length)
            $maintenance.Flush($true)
        } finally { $maintenance.Dispose() }
        $maintenanceOwned = $true
        $maintenanceHash = Get-Digest "$state\maintenance.json"
        function Assert-OwnedMaintenance {
            try {
                foreach ($marker in @('disable-agent','disable-recovery')) {
                    if (Test-Path -LiteralPath "$state\$marker") { throw 'local stop marker' }
                }
                if ((Get-Digest "$state\maintenance.json") -ne $maintenanceHash) { throw 'owner changed' }
                $current = Get-Content "$state\agent.json" -Raw | ConvertFrom-Json
                if ($current.disabled -or !$current.management_agent_enabled -or $current.edition -ne $oldState.edition -or $current.package_kind -ne $oldState.package_kind -or ((!$legacyIdentity -or !$mainStarted) -and $current.agent_installation_id -cne $oldState.agent_installation_id)) { throw 'policy changed' }
            } catch { throw '[update-pending] maintenance ownership or local policy changed' }
        }
        Stop-Agent
        $stopped = $true
        foreach ($name in $identityBefore.Keys) {
            if ((Get-Digest "$config\$name") -ne $identityBefore[$name]) { throw '[update-pending] configuration changed before main stop' }
        }
        $stateNow = Get-Content -LiteralPath "$state\agent.json" -Raw | ConvertFrom-Json
        if ($stateNow.agent_installation_id -ne $oldState.agent_installation_id -or $stateNow.disabled -or !$stateNow.management_agent_enabled) { throw '[update-pending] agent identity or policy changed' }
        $mainStopped = $true
        Stop-Main $Installed
        Assert-PlainTree $state
        Assert-PlainTree $config
        New-Item -ItemType Directory -Path $backup | Out-Null
        Copy-Checked $agent "$backup\agent.exe"
        Copy-TreeChecked $state "$backup\state"
        Remove-Item -LiteralPath "$backup\state\maintenance.json"
        Copy-TreeChecked $config "$backup\config"
        $hadBootstrap = Test-Path -LiteralPath $bootstrap
        if ($hadBootstrap) {
            Assert-PlainTree $bootstrap
            Copy-TreeChecked $bootstrap "$backup\bootstrap"
            $manifest = Get-Content -LiteralPath "$bootstrap\bootstrap.json" -Raw | ConvertFrom-Json
            if ($manifest.agent_sha256 -ne (Get-Digest $agent) -or $manifest.edition -ne $Edition -or $manifest.package_kind -ne 'exe' -or $manifest.normal_install_path -ne $normal) { throw 'bootstrap identity mismatch' }
        }
        $saved = $true
        $migration = @{schema=1;legacy=$legacyIdentity;original_state_sha256=(Get-Digest "$backup\state\agent.json");target_agent_sha256=$Hash;build=$Build;edition=$Edition;installation_id=$null}
        [IO.File]::WriteAllText("$backup\identity-migration.json",($migration | ConvertTo-Json -Depth 8))
        foreach ($name in $identityBefore.Keys) {
            if ((Get-Digest "$config\$name") -ne $identityBefore[$name] -or (Get-Digest "$backup\config\$name") -ne $identityBefore[$name]) { throw '[update-pending] configuration changed during backup' }
        }
        $batch = Join-Path $backup 'main.cmd'
        [IO.File]::WriteAllBytes($batch,[Convert]::FromBase64String($Body))
        $mainStarted = $true
        Invoke-Bounded "$env:SystemRoot\System32\cmd.exe" ('/d /c ""'+$batch+'""') "$backup\main.log"
        Assert-OwnedMaintenance
        Copy-Checked $Target $agent
        Start-Agent
        $stopped = $false
        $arguments = 'status --json --edition '+$Edition+' --package-kind exe --normal-install-path "'+$normal+'"'
        $deadline = [DateTime]::UtcNow.AddSeconds(90)
        $boundIdentity = ''
        do {
            try {
                Invoke-Bounded $agent $arguments "$backup\status.json"
                $status = Get-Content -LiteralPath "$backup\status.json" -Raw | ConvertFrom-Json
                $diskState = Get-Content -LiteralPath "$state\agent.json" -Raw | ConvertFrom-Json
                $boundIdentity = Confirm-LegacyInstallationIdentity $oldState $diskState $status $Edition $boundIdentity
            } catch {
                if ($_.Exception.Message.StartsWith('[update-pending]')) { throw }
                $status = $null
            }
            if ($status.application.state -eq 'ready') { break }
            Start-Sleep -Milliseconds 500
        } while ([DateTime]::UtcNow -lt $deadline)
        if (!$boundIdentity -or $status.agent.build_seq -ne $Build -or $status.application.build_seq -ne $Build -or $status.application.state -ne 'ready' -or $status.package_identity.edition -ne $Edition -or $status.package_identity.kind -ne 'exe') { throw 'target health or identity mismatch' }
        $migration.installation_id = $boundIdentity
        $migration.session_generation = $diskState.status_session_generation
        [IO.File]::WriteAllText("$backup\identity-migration.json",($migration | ConvertTo-Json -Depth 8))
        if ((Get-Digest $agent) -ne $Hash) { throw 'resident target not replaced' }
        $s = Get-AgentService
        if ($s.State -ne 'Running' -or $s.PathName -ne $expected -or $s.ProcessId -eq 0) { throw 'new resident SCM unconfirmed' }
        foreach ($name in @('RustDesk.toml','RustDesk2.toml')) {
            if ((Get-Digest "$config\$name") -ne (Get-Digest "$backup\config\$name")) { throw '[update-pending] client configuration changed' }
        }
        $bootstrapStarted = $true
        Sync-Bootstrap $bootstrap $Target $Hash $Edition $normal $hadBootstrap
        $finalStateLock = [IO.File]::Open("$state\agent.json",'Open','Read','Read')
        $finalReader = [IO.StreamReader]::new($finalStateLock,[Text.Encoding]::UTF8,$true,4096,$true)
        try { $finalState = $finalReader.ReadToEnd() | ConvertFrom-Json } finally { $finalReader.Dispose() }
        if ($finalState.agent_installation_id -cne $migration.installation_id -or $finalState.status_session_generation -ne $migration.session_generation -or $finalState.edition -ne $Edition -or $finalState.package_kind -ne 'exe' -or $finalState.disabled -or !$finalState.management_agent_enabled) { throw '[update-pending] final installation identity or policy changed' }
        foreach ($name in $identityBefore.Keys) {
            if ((Get-Digest "$config\$name") -ne $identityBefore[$name]) { throw '[update-pending] configuration changed before commit' }
        }
        # Work is the protected elevated runner directory, not the removable backup.
        $auditPath = Join-Path $Work ('legacy-update-audit-'+(Split-Path $backup -Leaf)+'.json')
        $audit = [IO.File]::Open($auditPath,'CreateNew','Write','None')
        try {
            $auditBytes = [Text.Encoding]::UTF8.GetBytes(($migration | ConvertTo-Json -Depth 8))
            $audit.Write($auditBytes,0,$auditBytes.Length)
            $audit.Flush($true)
        } finally { $audit.Dispose() }
        Assert-OwnedMaintenance
        Remove-Item -LiteralPath "$state\maintenance.json"
        $maintenanceOwned = $false
        $committed = $true
        try {
            Remove-Item -LiteralPath $MainBackup -Recurse -Force
            Remove-Item -LiteralPath $backup -Recurse -Force
        } catch { Write-Warning ('committed update backup cleanup failed: '+$_.Exception.Message) }
    } catch {
        $failure = $_
        # Keep diagnostics outside runner.* (the elevated handoff deletes those).
        # Do not persist exception text, source lines or arguments: they may hold configuration.
        try {
            $diagnostic = @{schema=1;exception_type=$failure.Exception.GetType().FullName;line=$failure.InvocationInfo.ScriptLineNumber;offset=$failure.InvocationInfo.OffsetInLine;main_stop_attempted=$mainStopped;backup_complete=$saved;main_started=$mainStarted;committed=$committed}
            $diagnosticPath = Join-Path $Work ('legacy-update-failure-'+(Split-Path $backup -Leaf)+'.json')
            $diagnosticFile = [IO.File]::Open($diagnosticPath,'CreateNew','Write','None')
            try {
                $diagnosticBytes = [Text.Encoding]::UTF8.GetBytes(($diagnostic | ConvertTo-Json))
                $diagnosticFile.Write($diagnosticBytes,0,$diagnosticBytes.Length)
                $diagnosticFile.Flush($true)
            } finally { $diagnosticFile.Dispose() }
        } catch { [Console]::Error.WriteLine('update diagnostic persistence failed; original failure retained') }
        if ($finalStateLock) { $finalStateLock.Dispose(); $finalStateLock = $null }
        if ($committed -or $failure.Exception.Message.StartsWith('[update-pending]')) { throw $failure }
        try {
            if ($maintenanceOwned) { Assert-OwnedMaintenance }
            if ($saved) {
                foreach ($name in $identityBefore.Keys) {
                    if ((Get-Digest "$config\$name") -ne $identityBefore[$name]) { throw '[update-pending] concurrent configuration writer; rollback withheld' }
                }
                Stop-Agent
                if ($mainStarted -and (Test-Path -LiteralPath "$backup\main-backup-ready")) {
                    if (!(Test-Path -LiteralPath $MainBackup)) { throw 'main backup missing' }
                    Stop-Main $Installed
                    & "$env:SystemRoot\System32\robocopy.exe" $MainBackup $normal /MIR /IS /IT /COPY:DAT /DCOPY:DAT /R:1 /W:1 | Out-Null
                    if ($LASTEXITCODE -ge 8) { throw 'main restore failed' }
                }
                Copy-Checked "$backup\agent.exe" $agent
                foreach ($pair in @(@('state',$state),@('config',$config))) {
                    & "$env:SystemRoot\System32\robocopy.exe" "$backup\$($pair[0])" $pair[1] /MIR /IS /IT /COPY:DAT /DCOPY:DAT /R:1 /W:1 | Out-Null
                    if ($LASTEXITCODE -ge 8) { throw 'identity restore failed' }
                }
                if ($hadBootstrap) {
                    & "$env:SystemRoot\System32\robocopy.exe" "$backup\bootstrap" $bootstrap /MIR /IS /IT /COPY:DAT /DCOPY:DAT /R:1 /W:1 | Out-Null
                    if ($LASTEXITCODE -ge 8) { throw 'bootstrap restore failed' }
                } elseif ($bootstrapStarted) {
                    $task = Get-ScheduledTask -TaskName RustDeskManagementAgentRecovery -ErrorAction SilentlyContinue
                    if ($task) {
                        if ($task.Actions.Count -ne 1 -or $task.Actions[0].Execute -ne "$bootstrap\rustdesk-recovery-bootstrap.exe") { throw 'bootstrap rollback task ownership mismatch' }
                        Unregister-ScheduledTask -TaskName RustDeskManagementAgentRecovery -Confirm:$false
                    }
                    if (Test-Path -LiteralPath $bootstrap) { Remove-Item -LiteralPath $bootstrap -Recurse -Force }
                }
                Start-Service RustDesk
                Start-Agent
                $maintenanceOwned = $false
            } else {
                if ($mainStopped) {
                    foreach ($name in $identityBefore.Keys) {
                        if ((Get-Digest "$config\$name") -ne $identityBefore[$name]) { throw '[update-pending] configuration changed; main restart withheld' }
                    }
                    foreach ($marker in @('disable-agent','disable-recovery')) {
                        if (Test-Path -LiteralPath "$state\$marker") { throw '[update-pending] local stop marker; main restart withheld' }
                    }
                    Start-Service RustDesk
                    (Get-Service RustDesk).WaitForStatus('Running',[TimeSpan]::FromSeconds(60))
                }
                if ($stopped) { Start-Agent }
                if ($maintenanceOwned) { Assert-OwnedMaintenance; Remove-Item -LiteralPath "$state\maintenance.json"; $maintenanceOwned = $false }
            }
        } catch {
            if ($_.Exception.Message.StartsWith('[update-pending]')) { throw }
            throw ('[update-rollback-failed] '+$failure.Exception.Message+'; '+$_.Exception.Message)
        }
        if ($saved) { throw ('[update-rolled-back] '+$failure.Exception.Message) }
        throw $failure
    } finally {
        if ($finalStateLock) { $finalStateLock.Dispose() }
        if ($targetLock) { $targetLock.Dispose() }
        if ($held) { $lock.ReleaseMutex() }
        $lock.Dispose()
    }
}
"###;
