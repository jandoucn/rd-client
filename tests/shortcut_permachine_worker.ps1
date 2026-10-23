param([ValidateSet('present','missing')][string]$Scenario='present')
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
if((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win11')) -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'fixed Win11 SYSTEM required'}
$base='C:\Users\Public\shortcut-machine'
$receipt="$base\$Scenario.json"
if(Test-Path $receipt){throw 'receipt exists; do not repeat'}
$package="$base\candidate.msi"
$report=@{scenario=$Scenario;sid='S-1-5-18';started=[DateTime]::UtcNow.ToString('o');phase='preflight'}
function Property($db,$key){$v=$db.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property``='$key'");$null=$v.Execute();$r=$v.Fetch();$value=$r.StringData(1);$null=$v.Close();return $value}
function State {
    $hashes=@{}
    foreach($dir in @('C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config','C:\ProgramData\RustDesk\ManagementAgent\state')){
        if(Test-Path $dir){foreach($f in Get-ChildItem -LiteralPath $dir -Recurse -File){$hashes[$f.FullName]=(Get-FileHash -LiteralPath $f.FullName).Hash}}
    }
    @{hashes=$hashes;services=@(Get-CimInstance Win32_Service | Where-Object {$_.Name -in @('RustDesk','RustDeskManagementAgent')} | Select-Object Name,State,PathName,StartName);install=Get-ItemProperty 'HKLM:\Software\RustDesk\InstallState\RustDesk' | Select-Object MsiProductCode,InstallLocation}
}
try{
    if(Get-Process msiexec -ErrorAction SilentlyContinue){throw 'installer running'}
    if((Get-FileHash $package).Hash -ne 'D8771B1B0304DF6FFA63F28E99B6EBE08C1244CC63F60839134E5A1A9906D675'){throw 'package digest mismatch'}
    $i=New-Object -ComObject WindowsInstaller.Installer
    $db=$i.OpenDatabase($package,0)
    $newCode=Property $db 'ProductCode';$newVersion=Property $db 'ProductVersion';$upgrade=Property $db 'UpgradeCode'
    $oldCode=(Get-ItemProperty 'HKLM:\Software\RustDesk\InstallState\RustDesk').MsiProductCode
    if($oldCode -ne '{C220D796-2FB4-45F5-A921-87CF071C2E49}' -or $i.ProductState($oldCode) -ne 5){throw 'wrong old baseline'}
    $oldVersion=$i.ProductInfo($oldCode,'VersionString')
    $oldDb=$i.OpenDatabase($i.ProductInfo($oldCode,'LocalPackage'),0)
    if((Property $oldDb 'UpgradeCode') -ne $upgrade -or [version]$newVersion -lt [version]$oldVersion){throw 'upgrade identity/version mismatch'}
    $report.identity=@{old=$oldCode;old_version=$oldVersion;new=$newCode;new_version=$newVersion;upgrade=$upgrade}
    $report.before=State
    $desktop=[Environment]::GetFolderPath('CommonDesktopDirectory');$link=Join-Path $desktop 'RustDesk.lnk'
    $shell=New-Object -ComObject WScript.Shell
    if(!(Test-Path $link) -or $shell.CreateShortcut($link).TargetPath -ine 'C:\Program Files\RustDesk\RustDesk.exe'){throw 'baseline shortcut mismatch'}
    Copy-Item -LiteralPath $link -Destination "$base\$Scenario.original.lnk"
    if($Scenario -eq 'missing'){Remove-Item -LiteralPath $link}
    $report.phase='installing';$report|ConvertTo-Json -Depth 8|Set-Content $receipt
    $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "/i `"$package`" /qn /norestart REBOOT=ReallySuppress DEFENDER_EXCLUSIONS=0 /l*v `"$base\$Scenario.log`"" -PassThru -Wait
    $report.exit=$p.ExitCode;$report.after=State;$report.new_state=$i.ProductState($newCode)
    $report.shortcut_exists=Test-Path $link
    if($report.shortcut_exists){$report.target=$shell.CreateShortcut($link).TargetPath}
    $report.phase='observed'
    if($p.ExitCode -notin @(0,3010) -or $report.new_state -ne 5){throw 'installer failed or product not installed'}
    if($report.shortcut_exists -ne ($Scenario -eq 'present')){throw 'shortcut existence mismatch'}
    if($Scenario -eq 'present' -and $report.target -ine 'C:\Program Files\RustDesk\RustDesk.exe'){throw 'target mismatch'}
    $report.phase='pass'
}catch{$report.error=$_.Exception.Message}
$report|ConvertTo-Json -Depth 10|Set-Content -LiteralPath $receipt -Encoding UTF8
