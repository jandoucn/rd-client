param([Parameter(Mandatory=$true)][string]$Source, [string]$OnlyCase='')
$ErrorActionPreference='Stop'
if ((Get-CimInstance Win32_OperatingSystem).BuildNumber -ne '17763' -or (Get-CimInstance Win32_ComputerSystem).Model -ne 'VirtualBox') { throw 'Win10 isolated guest required' }
$text = [IO.File]::ReadAllText($Source)
$script = $text.Split(@('const SCRIPT: &str = r###"'),[StringSplitOptions]::None)[1].Split(@('"###;'),[StringSplitOptions]::None)[0]
$root = Join-Path $env:TEMP ('legacy-agent-fixture-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
# Relocate only fixed filesystem roots in the test copy. Service operations are
# intercepted below; production code has no environment or CLI override.
$script = $script.Replace('C:\ProgramData\RustDesk',"$root\data").Replace('C:\Program Files\RustDesk',"$root\normal").Replace('$env:SystemRoot\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config',"$root\config").Replace('Global\RustDeskManagementAgentInstall',('Local\LegacyAgentFixture-'+[Guid]::NewGuid().ToString('N'))).Replace('AddSeconds(90)','AddSeconds(0)')
Invoke-Expression $script
$fixtureLock=[Threading.Mutex]::new($false,'Global\RustDeskWin10LegacyAgentFixture')
if (!$fixtureLock.WaitOne(0)) { throw 'Win10 fixture already owned' }
$nativeBatch=Join-Path $root 'native child.cmd'
[IO.File]::WriteAllText($nativeBatch,"@echo off`r`necho native-child-completed`r`nexit /b 0`r`n")
Invoke-Bounded "$env:SystemRoot\System32\cmd.exe" ('/d /c ""'+$nativeBatch+'""') "$root\native.log"
if (!(Get-Content "$root\native.log" -Raw).Contains('native-child-completed')) { throw 'native cmd execution missing' }
function Get-Service {
    param($Name)
    $service=[pscustomobject]@{Status='Running'}
    $service | Add-Member ScriptMethod WaitForStatus { param($State,$Timeout) }
    $service
}
function Get-ScheduledTask { param($TaskName,$ErrorAction) }
function Get-AgentService { [pscustomobject]@{ProcessId=1;State='Running';StartMode='Auto';StartName='LocalSystem';PathName=('"'+$root+'\data\ManagementAgent\rustdesk-management-agent.exe" --service --edition '+$script:edition+' --package-kind exe --normal-install-path "'+$root+'\normal"')} }
function Stop-Agent {
    if ($script:fault -eq 'stop_timeout') { throw '[update-pending] fixture stop timeout' }
    $script:stops++
    if ($script:stops -eq 1 -and $script:fault -eq 'late_config') { [IO.File]::WriteAllText("$root\config\RustDesk2.toml","[options]`nstop-service='Y'") }
    if ($script:stops -eq 1 -and $script:fault -eq 'late_identity') { [IO.File]::WriteAllText("$root\config\RustDesk.toml",'changed identity') }
}
function Start-Agent {
    $script:agentRestarts++
    if ($script:fault -eq 'new_start' -and (Get-Digest "$root\data\ManagementAgent\rustdesk-management-agent.exe") -eq $script:newHash) { throw 'fixture new start failed' }
    if ((Get-Digest "$root\data\ManagementAgent\rustdesk-management-agent.exe") -eq $script:newHash) {
        $s=Get-Content "$root\data\ManagementAgent\state\agent.json" -Raw | ConvertFrom-Json
        if (!$s.PSObject.Properties['agent_installation_id']) { $s | Add-Member NoteProperty agent_installation_id ('a'*32) }
        $s | Add-Member NoteProperty status_session_generation 1 -Force
        $s | ConvertTo-Json -Depth 8 | Set-Content "$root\data\ManagementAgent\state\agent.json"
    }
}
function Stop-Service { param($Name,$ErrorAction) }
function Stop-Main { param($Installed)
    if ($script:fault -eq 'policy_flip') {
        $s=Get-Content "$root\data\ManagementAgent\state\agent.json" -Raw | ConvertFrom-Json
        $s.disabled=$true
        $s|ConvertTo-Json|Set-Content "$root\data\ManagementAgent\state\agent.json"
        throw 'fixture policy flipped'
    }
    if ($script:fault -eq 'foreign_marker') { [IO.File]::WriteAllText("$root\data\ManagementAgent\state\maintenance.json",'foreign');throw 'fixture marker replaced' }
    if ($script:fault -eq 'prebackup_stop_error') { throw 'fixture stop-main post-stop failure' }
}
function Start-Service { param($Name,$ErrorAction) if ($Name -eq 'RustDesk') { $script:mainRestarts++ } }
function Sync-Bootstrap {
    param($Directory,$Target,$Hash,$Edition,$Normal,$Existing)
    Copy-Checked $Target "$Directory\rustdesk-recovery-bootstrap.exe"
    [IO.File]::WriteAllText("$Directory\bootstrap.json",'{"new":true}')
    if ($script:fault -like 'legacy_marker_commit_*') { Add-LateStopMarker }
    if ($script:fault -eq 'bootstrap') { throw 'fixture bootstrap failure' }
    if ($script:fault -eq 'commit_identity') { [IO.File]::WriteAllText("$root\config\RustDesk.toml",'late commit identity') }
    if ($script:fault -in @('final_instance','final_generation')) {
        $s=Get-Content "$root\data\ManagementAgent\state\agent.json" -Raw | ConvertFrom-Json
        if ($script:fault -eq 'final_instance') { $s.agent_installation_id='b'*32 } else { $s.status_session_generation=2 }
        $s | ConvertTo-Json -Depth 8 | Set-Content "$root\data\ManagementAgent\state\agent.json"
    }
}
function Add-LateStopMarker {
    $script:markerName = if ($script:fault.EndsWith('_agent')) { 'disable-agent' } else { 'disable-recovery' }
    [IO.File]::WriteAllText("$root\data\ManagementAgent\state\$markerName",'new local stop intent')
    $script:markerSnapshot=@{}
    foreach ($file in Get-ChildItem "$root\data\ManagementAgent\state" -File) { $script:markerSnapshot[$file.Name]=Get-Digest $file.FullName }
    $script:startsAtMarker=$script:agentRestarts+$script:mainRestarts
}
$nativeCopy = ${function:Copy-Checked}
function Copy-Checked {
    param($Source,$Destination)
    if ($script:fault -eq 'copy_fail' -and $Source -eq "$root\payload\rustdesk-management-agent.exe") { throw 'fixture copy failure' }
    if ($script:fault -eq 'not_replaced' -and $Destination -eq "$root\data\ManagementAgent\rustdesk-management-agent.exe" -and $Source -like '*payload*') { return }
    & $nativeCopy $Source $Destination
}
function Invoke-Bounded {
    param($File,$Arguments,$Output)
    if ($File -like '*cmd.exe') {
        Copy-Item "$root\normal" "$root\main-backup" -Recurse
        [IO.File]::WriteAllText((Join-Path (Split-Path $Output) 'main-backup-ready'),'ready')
        [IO.File]::WriteAllText("$root\normal\RustDesk.exe",'new main')
        if ($script:fault -like 'legacy_marker_rollback_*') { Add-LateStopMarker; throw 'fixture post-backup failure' }
        if ($script:fault -like 'legacy_marker_start_*') { Add-LateStopMarker }
        return
    }
    $health = if ($script:fault -in @('health_fail','legacy_health_fail')) {'unknown'} else {'ready'}
    @{agent=@{build_seq=2026101502};application=@{build_seq=2026101502;state=$health};package_identity=@{edition=$script:edition;kind='exe'};agent_installation_id=('a'*32);session_generation=1} | ConvertTo-Json -Depth 5 | Set-Content $Output
}
$results = @()
try {
    foreach ($script:edition in @('standard','sos')) {
        foreach ($script:fault in @('none','stop_timeout','missing_target','copy_fail','new_start','health_fail','not_replaced','bootstrap','late_config','late_identity','commit_identity','legacy_success','legacy_health_fail','legacy_disabled','legacy_stop','legacy_unapproved','final_instance','final_generation','durable_audit','prebackup_stop_error','legacy_marker_commit_agent','legacy_marker_commit_recovery','legacy_marker_rollback_agent','legacy_marker_rollback_recovery','legacy_marker_start_agent','legacy_marker_start_recovery','legacy_schema_missing','legacy_schema_null','legacy_schema_string','legacy_schema_wrongvalue','legacy_schema_bool','legacy_schema_array','legacy_schema_float')) {
            if ($OnlyCase -and $fault -ne $OnlyCase -and !(($OnlyCase -in @('policy_flip','foreign_marker')) -and $fault -eq 'prebackup_stop_error')) { continue }
            if ($OnlyCase -in @('policy_flip','foreign_marker')) { $script:fault=$OnlyCase }
            Get-ChildItem $root | Remove-Item -Recurse -Force
            foreach ($dir in @('normal','payload','data\ManagementAgent\state','data\RecoveryBootstrap','config','work')) { New-Item -ItemType Directory "$root\$dir" -Force | Out-Null }
            $script:stops=0
            $script:mainRestarts=0
            $script:agentRestarts=0
            [IO.File]::WriteAllText("$root\normal\RustDesk.exe",'old main')
            [IO.File]::WriteAllText("$root\payload\rustdesk-management-agent.exe",'new agent')
            [IO.File]::WriteAllText("$root\data\ManagementAgent\rustdesk-management-agent.exe",'old agent')
            [IO.File]::WriteAllText("$root\data\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe",'old agent')
            [IO.File]::WriteAllText("$root\config\RustDesk.toml",'old identity')
            [IO.File]::WriteAllText("$root\config\RustDesk2.toml","[options]`nstop-service='N'")
            @{edition=$edition;package_kind='exe';management_agent_enabled=$true;disabled=$false;agent_installation_id=('a'*32)} | ConvertTo-Json | Set-Content "$root\data\ManagementAgent\state\agent.json"
            if ($fault -like 'legacy_*') {
                # Actual SOS0510 field names/types sampled read-only; values below are fixtures.
                @{schema_version=1;edition=$edition;package_kind='exe';target_key=('windows-x86_64-exe-'+$edition);manifest_url='https://download.yan.life/rustdesk/stable/manifest.json';normal_install_path="$root\normal";recovery_install_path="$root\fallback";next_check_at=$null;retry_stage=0;last_version='';last_primary_url='';last_error='';management_agent_enabled=$true;disabled=($fault -eq 'legacy_disabled');management_agent_policy_revision=0;last_policy_check_at=$null;last_policy_error='';unknown_legacy=@{preserve='old unknown bytes'}} | ConvertTo-Json -Depth 8 | Set-Content "$root\data\ManagementAgent\state\agent.json"
                if ($fault -eq 'legacy_stop') { [IO.File]::WriteAllText("$root\config\RustDesk2.toml","[options]`nstop-service='Y'") }
                if ($fault -like 'legacy_schema_*') {
                    $legacy=Get-Content "$root\data\ManagementAgent\state\agent.json" -Raw | ConvertFrom-Json
                    switch ($fault) {
                        'legacy_schema_missing' { $legacy.PSObject.Properties.Remove('schema_version') }
                        'legacy_schema_null' { $legacy.schema_version=$null }
                        'legacy_schema_string' { $legacy.schema_version='1' }
                        'legacy_schema_wrongvalue' { $legacy.schema_version=2 }
                        'legacy_schema_bool' { $legacy.schema_version=$true }
                        'legacy_schema_array' { $legacy.schema_version=@(1) }
                        'legacy_schema_float' { $legacy.schema_version=1.5 }
                    }
                    $legacy | ConvertTo-Json -Depth 8 | Set-Content "$root\data\ManagementAgent\state\agent.json"
                }
            }
            @{schema=1;edition=$edition;package_kind='exe';normal_install_path="$root\normal";agent_sha256=(Get-Digest "$root\data\ManagementAgent\rustdesk-management-agent.exe")} | ConvertTo-Json | Set-Content "$root\data\RecoveryBootstrap\bootstrap.json"
            $before=@{}
            foreach ($file in @('normal\RustDesk.exe','data\ManagementAgent\rustdesk-management-agent.exe','data\ManagementAgent\state\agent.json','data\RecoveryBootstrap\bootstrap.json','data\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe','config\RustDesk.toml','config\RustDesk2.toml')) { $before[$file]=Get-Digest "$root\$file" }
            $script:newHash=Get-Digest "$root\payload\rustdesk-management-agent.exe"
            if ($fault -eq 'legacy_unapproved') { $script:newHash='0'*64 }
            if ($fault -eq 'missing_target') { Remove-Item "$root\payload\rustdesk-management-agent.exe" }
            $errorText=''
            try { Invoke-LegacyAgentUpdate "$root\payload\rustdesk-management-agent.exe" "$root\normal\RustDesk.exe" "$root\main-backup" $newHash $edition 2026101502 '' "$root\work" } catch { $errorText=$_.Exception.Message }
            if ($fault -in @('none','legacy_success','durable_audit','legacy_schema_missing')) {
                if ($errorText) { throw "${edition}/${fault}: $errorText" }
                if ((Get-Digest "$root\data\ManagementAgent\rustdesk-management-agent.exe") -ne $newHash) { throw 'resident unchanged' }
                if (Test-Path "$root\main-backup") { throw 'commit backup retained' }
                if ($fault -in @('legacy_success','legacy_schema_missing')) {
                    $saved=Get-Content "$root\data\ManagementAgent\state\agent.json" -Raw | ConvertFrom-Json
                    if ($saved.unknown_legacy.preserve -ne 'old unknown bytes') { throw 'opaque lost on success' }
                }
                if ($fault -eq 'durable_audit') {
                    $receipts=@(Get-ChildItem "$root\work" -Filter 'legacy-update-audit-*.json')
                    if ($receipts.Count -ne 1) { throw 'durable migration audit missing after cleanup' }
                    $receipt=Get-Content $receipts[0].FullName -Raw | ConvertFrom-Json
                    if ($receipt.installation_id -ne ('a'*32) -or $receipt.session_generation -ne 1 -or $receipt.original_state_sha256 -ne $before['data\ManagementAgent\state\agent.json']) { throw 'migration audit binding invalid' }
                }
            } else {
                if ($fault -like 'legacy_marker_*') {
                    $issues=@()
                    if (!$errorText.StartsWith('[update-pending]')) { $issues+='error not Pending' }
                    if (($agentRestarts+$mainRestarts) -ne $startsAtMarker) { $issues+='service started after marker' }
                    foreach ($name in $markerSnapshot.Keys) {
                        $p="$root\data\ManagementAgent\state\$name"
                        if (!(Test-Path $p) -or (Get-Digest $p) -ne $markerSnapshot[$name]) { $issues+="state/marker lost: $name" }
                    }
                    $backups=@(Get-ChildItem "$root\work" -Directory -Filter 'agent-backup-*')
                    if ($backups.Count -ne 1 -or !(Test-Path "$root\main-backup")) { $issues+='backups lost' }
                    elseif ((Get-Digest "$($backups[0].FullName)\state\agent.json") -ne $before['data\ManagementAgent\state\agent.json']) { $issues+='old state backup changed' }
                    if ($issues.Count) { throw ($issues -join '; ') }
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText};continue
                }
                if ($fault -like 'legacy_schema_*') {
                    if ($errorText -ne 'legacy state contract mismatch' -or $stops -ne 0 -or $agentRestarts -ne 0 -or $mainRestarts -ne 0) { throw "invalid schema not rejected before service actions: $errorText" }
                    foreach ($file in $before.Keys) { if ((Get-Digest "$root\$file") -ne $before[$file]) { throw "schema rejection lost old bytes: $file" } }
                    if (Test-Path "$root\data\ManagementAgent\state\maintenance.json") { throw 'schema rejection created maintenance' }
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText};continue
                }
                if (!$errorText) { throw "${edition}/${fault}: false success" }
                if ($fault -in @('policy_flip','foreign_marker')) {
                    if(!$errorText.StartsWith('[update-pending]') -or $mainRestarts -ne 0){throw 'unsafe restart after policy/owner changed'}
                    if($fault -eq 'foreign_marker' -and [IO.File]::ReadAllText("$root\data\ManagementAgent\state\maintenance.json") -ne 'foreign'){throw 'foreign marker lost'}
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText};continue
                }
                if ($fault -eq 'prebackup_stop_error') {
                    if ($mainRestarts -ne 1 -or !$errorText.Contains('fixture stop-main post-stop failure')) { throw 'original main not restored or original error masked' }
                    $diagnostics=@(Get-ChildItem "$root\work" -Filter 'legacy-update-failure-*.json')
                    if ($diagnostics.Count -ne 1) { throw 'failure diagnostics missing' }
                    $diagnostic=Get-Content $diagnostics[0].FullName -Raw | ConvertFrom-Json
                    if (!$diagnostic.main_stop_attempted -or $diagnostic.backup_complete -or $diagnostic.line -le 0 -or !$diagnostic.exception_type) { throw 'failure diagnostics invalid' }
                    foreach ($file in $before.Keys) { if ((Get-Digest "$root\$file") -ne $before[$file]) { throw 'prebackup old bytes changed' } }
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText}
                    continue
                }
                if ($fault -in @('final_instance','final_generation')) {
                    if (!$errorText.StartsWith('[update-pending]') -or !(Test-Path "$root\main-backup")) { throw 'final state drift not preserved as pending' }
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText}
                    continue
                }
                if ($fault -eq 'commit_identity') {
                    if (!$errorText.StartsWith('[update-pending]') -or !(Test-Path "$root\main-backup")) { throw 'late commit must retain backup and remain pending' }
                    if ([IO.File]::ReadAllText("$root\config\RustDesk.toml") -ne 'late commit identity') { throw 'late commit data overwritten' }
                    $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText}
                    continue
                }
                if ($fault -in @('late_config','late_identity')) {
                    if (!$errorText.StartsWith('[update-pending]')) { throw "late change not protected: $errorText" }
                    if (Test-Path "$root\main-backup") { throw 'late change reached main replacement' }
                    $changed = if ($fault -eq 'late_config') {'config\RustDesk2.toml'} else {'config\RustDesk.toml'}
                    if ((Get-Digest "$root\$changed") -eq $before[$changed]) { throw 'late writer data overwritten' }
                    $before.Remove($changed)
                }
                if ($fault -eq 'stop_timeout' -and !$errorText.StartsWith('[update-pending]')) { throw 'timeout not pending' }
                if ($fault -notin @('stop_timeout','missing_target','late_config','late_identity','legacy_disabled','legacy_stop','legacy_unapproved') -and !$errorText.StartsWith('[update-rolled-back]')) { throw "rollback not confirmed: $errorText" }
                foreach ($file in $before.Keys) { if ((Get-Digest "$root\$file") -ne $before[$file]) { throw "old bytes lost: $file ($errorText)" } }
            }
            $results += [pscustomobject]@{edition=$edition;case=$fault;passed=$true;error=$errorText}
        }
    }
    $results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath "$PSScriptRoot\legacy-agent-fixture-result.json" -Encoding UTF8
    $results | ConvertTo-Json -Depth 5
    if (!$results.Count) { throw 'zero fixture cases executed' }
    "fixture_count=$($results.Count)"
} finally { Remove-Item -LiteralPath $root -Recurse -Force; $fixtureLock.ReleaseMutex(); $fixtureLock.Dispose() }
