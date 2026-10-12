param([Parameter(Mandatory=$true)][string]$Source)
$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -notin @('RDLAB-WIN10','RDLAB-WIN11') -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'SYSTEM lab VM required'}
. $Source
$root=Join-Path $env:TEMP ('msi-bootstrap-preflight-'+[guid]::NewGuid().ToString('N'))
$count=0
function Assert-Case($Condition,$Message){if(!$Condition){throw $Message};$script:count++}
try {
    Assert-Case (!(Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk')) 'Absent root must not initialize'
    foreach($name in @('RecoveryBootstrap','ManagementAgent\state')){New-Item -ItemType Directory -Path (Join-Path $root $name) -Force|Out-Null}
    & "$env:SystemRoot\System32\icacls.exe" $root /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /T /Q|Out-Null
    if($LASTEXITCODE -ne 0){throw 'Fixture ACL setup failed'}
    $recovery=Join-Path $root 'RecoveryBootstrap'
    & "$env:SystemRoot\System32\icacls.exe" $recovery /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' /Q|Out-Null
    if($LASTEXITCODE -ne 0){throw 'Fixture ACL setup failed'}
    $agent=Join-Path $root 'ManagementAgent\rustdesk-management-agent.exe'
    $entry=Join-Path $recovery 'rustdesk-recovery-bootstrap.exe'
    [IO.File]::WriteAllText($agent,'approved bytes');[IO.File]::WriteAllText($entry,'approved bytes')
    $manifest=Join-Path $recovery 'bootstrap.json'
    @{schema=1;edition='standard';package_kind='msi';normal_install_path='C:\Program Files\RustDesk';agent_sha256=(Get-FileHash $agent).Hash}|ConvertTo-Json|Set-Content $manifest -Encoding UTF8
    $stateRoot=Join-Path $root 'ManagementAgent\state'
    @{disabled=$false;management_agent_enabled=$true;normal_install_path='C:\Program Files\RustDesk';target_key='windows-x86_64-msi-standard'}|ConvertTo-Json|Set-Content (Join-Path $stateRoot 'agent.json') -Encoding UTF8
    $before=[IO.File]::ReadAllText($manifest)
    Assert-Case (Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk') 'Valid existing bootstrap refused'
    & "$env:SystemRoot\System32\icacls.exe" $root /grant '*S-1-5-32-545:(WD,AD,WEA,WA)' '*S-1-3-0:(OI)(CI)(IO)F' /Q|Out-Null
    if($LASTEXITCODE -ne 0){throw 'Fixture parent ACL setup failed'}
    Assert-Case (Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk') 'Parent create-only permissions do not overwrite existing protected entries'
    & "$env:SystemRoot\System32\icacls.exe" $root /grant '*S-1-5-32-545:(DC)' /Q|Out-Null
    if($LASTEXITCODE -ne 0){throw 'Fixture delete-child ACL setup failed'}
    $rejected=$false
    try{Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk'|Out-Null}catch{$rejected=$_.Exception.Message -eq 'Bootstrap writable by unrelated principal'}
    Assert-Case $rejected 'Parent delete-child permission must be refused'
    & "$env:SystemRoot\System32\icacls.exe" $root /remove:g '*S-1-5-32-545' /Q|Out-Null
    if($LASTEXITCODE -ne 0){throw 'Fixture ACL reset failed'}
    foreach($gate in @('disable-agent','disable-recovery','maintenance.json')){
        $path=Join-Path $stateRoot $gate;[IO.File]::WriteAllText($path,'keep')
        Assert-Case (!(Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk')) 'Local gate bypassed'
        Assert-Case ([IO.File]::ReadAllText($path) -eq 'keep') 'Gate changed'
        [IO.File]::Delete($path)
    }
    [IO.File]::WriteAllText($entry,'tampered')
    $rejected=$false
    try{Test-RdBootstrapUpgrade $root standard 'C:\Program Files\RustDesk'|Out-Null}catch{$rejected=$_.Exception.Message -eq 'Bootstrap original integrity mismatch'}
    Assert-Case $rejected 'Drift must fail closed'
    Assert-Case ([IO.File]::ReadAllText($entry) -eq 'tampered') 'Drift evidence changed'
    Assert-Case ([IO.File]::ReadAllText($manifest) -eq $before) 'Manifest changed'
    "PASS: $count assertions"
}finally{if(Test-Path -LiteralPath $root){Remove-Item -LiteralPath $root -Recurse -Force}}
