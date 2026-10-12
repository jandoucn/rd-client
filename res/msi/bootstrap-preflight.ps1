function Test-RdBootstrapUpgrade {
    param([string]$Root, [string]$Edition, [string]$NormalPath)
    $ErrorActionPreference='Stop'
    if($Edition -notin @('standard','sos')){throw 'Invalid bootstrap edition'}
    $directory=Join-Path $Root 'RecoveryBootstrap'
    if(!(Test-Path -LiteralPath $directory)){return $false}
    $stateRoot=Join-Path $Root 'ManagementAgent\state'
    foreach($name in @('disable-agent','disable-recovery','maintenance.json')){
        if(Test-Path -LiteralPath (Join-Path $stateRoot $name)){return $false}
    }
    $paths=@($Root,$directory,(Join-Path $Root 'ManagementAgent'),$stateRoot,
        (Join-Path $directory 'bootstrap.json'),(Join-Path $directory 'rustdesk-recovery-bootstrap.exe'),
        (Join-Path $Root 'ManagementAgent\rustdesk-management-agent.exe'),(Join-Path $stateRoot 'agent.json'))
    foreach($path in $paths){
        $item=Get-Item -LiteralPath $path -Force
        if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Bootstrap reparse path refused'}
        $acl=Get-Acl -LiteralPath $path
        if($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin @('S-1-5-18','S-1-5-32-544')){throw 'Bootstrap owner refused'}
        foreach($ace in $acl.Access){
            if($ace.AccessControlType -ne 'Allow'){throw 'Bootstrap ACL type unsupported'}
            if($ace.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly){continue}
            $sid=$ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
            $allowed=0x1200a9
            # Legacy ProgramData ancestors allow creating new children, but not
            # replacing these existing files or deleting protected children.
            if($item.PSIsContainer -and $path -ne $directory){$allowed=$allowed -bor 0x116}
            if($sid -notin @('S-1-5-18','S-1-5-32-544') -and ([int]$ace.FileSystemRights -band (-bnot $allowed)) -ne 0){throw 'Bootstrap writable by unrelated principal'}
        }
    }
    if(!(Get-Acl -LiteralPath $directory).AreAccessRulesProtected){throw 'Bootstrap ACL inheritance not protected'}
    foreach($path in @((Join-Path $directory 'bootstrap.json'),(Join-Path $stateRoot 'agent.json'))){
        if((Get-Item -LiteralPath $path -Force).Length -gt 65536){throw 'Bootstrap metadata too large'}
    }
    $state=Get-Content -LiteralPath (Join-Path $stateRoot 'agent.json') -Raw|ConvertFrom-Json
    if($state.disabled -ne $false -or $state.management_agent_enabled -ne $true){return $false}
    $manifest=Get-Content -LiteralPath (Join-Path $directory 'bootstrap.json') -Raw|ConvertFrom-Json
    if($manifest.schema -ne 1 -or $manifest.edition -ne $Edition -or $manifest.package_kind -ne 'msi' -or
        $manifest.normal_install_path -ne $NormalPath -or $state.normal_install_path -ne $NormalPath -or
        $state.target_key -ne "windows-x86_64-msi-$Edition"){throw 'Bootstrap identity mismatch'}
    $hash=(Get-FileHash -LiteralPath (Join-Path $Root 'ManagementAgent\rustdesk-management-agent.exe') -Algorithm SHA256).Hash
    if($manifest.agent_sha256 -ne $hash -or (Get-FileHash -LiteralPath (Join-Path $directory 'rustdesk-recovery-bootstrap.exe') -Algorithm SHA256).Hash -ne $hash){throw 'Bootstrap original integrity mismatch'}
    return $true
}
