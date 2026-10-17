param([Parameter(Mandatory=$true)][string]$Source)
$ErrorActionPreference='Stop'
$text=[IO.File]::ReadAllText($Source)
$body=$text.Split(@('const SCRIPT: &str = r###"'),[StringSplitOptions]::None)[1].Split(@('"###;'),[StringSplitOptions]::None)[0]
Invoke-Expression $body
$count=0
foreach ($edition in @('standard','sos')) {
    # f79ed6 AgentState has neither installation ID nor session generation.
    $old=('{"schema_version":1,"edition":"'+$edition+'","package_kind":"exe","target_key":"windows-x86_64-exe-'+$edition+'","manifest_url":"https://download.yan.life/rustdesk/stable/manifest.json","disabled":false,"management_agent_enabled":true,"unknown_legacy":{"keep":7}}' | ConvertFrom-Json)
    $status=@{agent_installation_id=('a'*32);session_generation=1;package_identity=@{edition=$edition;kind='exe'}}
    $disk=@{agent_installation_id=('a'*32);status_session_generation=1;edition=$edition;package_kind='exe'}
    $bound=Confirm-LegacyInstallationIdentity $old $disk $status $edition ''
    if ($bound -ne ('a'*32)) { throw 'first initialization rejected' };$count++
    if ((Confirm-LegacyInstallationIdentity $old $disk $status $edition $bound) -ne $bound) { throw 'stable identity rejected' };$count++
    $disk.agent_installation_id='b'*32
    $failed=$false
    try { Confirm-LegacyInstallationIdentity $old $disk $status $edition $bound | Out-Null } catch { $failed=$true }
    if (!$failed) { throw 'disk identity drift accepted' };$count++
    $disk.agent_installation_id='a'*32
    $old | Add-Member NoteProperty agent_installation_id ''
    $failed=$false
    try { Confirm-LegacyInstallationIdentity $old $disk $status $edition '' | Out-Null } catch { $failed=$true }
    if (!$failed) { throw 'present malformed identity accepted as legacy' };$count++
}
"passed=$count"
