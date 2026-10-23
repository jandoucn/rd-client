param([string]$PackageDirectory, [string]$Receipt)
$ErrorActionPreference='Stop'
if ((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win11'))) { throw 'Win11 fixture only' }
$root=Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'RDShortcutFixture-7BC91'
$desktop=Join-Path $root 'Desktop'
$link=Join-Path $desktop 'RustDesk.lnk'
$key='HKCU:\Software\RDShortcutFixture-7BC91'
$results=@()
function RunMsi($Arguments,$Label,$Expected=0) {
    $log=Join-Path $PackageDirectory "$Label.log"
    $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "$Arguments /qn /norestart /l*v `"$log`"" -Wait -PassThru
    if ($p.ExitCode -ne $Expected) { throw "$Label exit=$($p.ExitCode), expected=$Expected" }
}
function Check($Value,$Label) {
    $script:results += @{case=$Label;passed=[bool]$Value}
    if (!$Value) { throw "FAIL $Label" }
}
$old=Join-Path $PackageDirectory 'fixture-1.0.0.msi'
$new=Join-Path $PackageDirectory 'fixture-2.0.0.msi'
try {
    if (Test-Path $key) { throw 'fixture registration exists; inspect before retry' }
    $null=New-Item -ItemType Directory -Path $desktop -Force
    RunMsi "/i `"$old`"" 'install'
    Check (Test-Path $link) 'first-install-default'
    Remove-Item -LiteralPath $link
    Check (Test-Path $key) 'keypath-remains'
    RunMsi "/famus `"$old`"" 'repair-missing'
    Check (!(Test-Path $link)) 'repair-missing-not-recreated'
    RunMsi "/i `"$new`"" 'upgrade-missing'
    Check (!(Test-Path $link)) 'upgrade-missing-not-recreated'
    RunMsi "/x `"$new`"" 'uninstall-new'
    RunMsi "/i `"$old`"" 'install-existing'
    RunMsi "/famus `"$old`"" 'repair-existing'
    Check (Test-Path $link) 'repair-existing-retained'
    RunMsi "/i `"$new`" FAILFIXTURE=1" 'rollback-existing' 1603
    Check (Test-Path $link) 'rollback-existing-retained'
    Remove-Item -LiteralPath $link
    RunMsi "/i `"$new`" FAILFIXTURE=1" 'rollback-missing' 1603
    Check (!(Test-Path $link)) 'rollback-missing-retained'
    RunMsi "/x `"$old`"" 'uninstall-old'
    RunMsi "/i `"$old`" DESKTOPSHORTCUTS=0" 'install-optout'
    Check (!(Test-Path $link)) 'first-install-optout'
    RunMsi "/x `"$old`"" 'uninstall-optout'
    RunMsi "/i `"$old`"" 'install-for-upgrade'
    RunMsi "/i `"$new`"" 'upgrade-existing'
    Check (Test-Path $link) 'upgrade-existing-retained'
    RunMsi "/x `"$new`"" 'cleanup'
} catch { $failure=$_.Exception.Message }
@{results=$results;failure=$failure;machine=$env:COMPUTERNAME;identity=[Security.Principal.WindowsIdentity]::GetCurrent().Name} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Receipt
if ($failure) { throw $failure }
