param([ValidateSet('present','missing')][string]$Scenario='present',[string]$ExpectedHash='90051585121DF95802CC2B9E3934ACD3ECCCF4DBA9AF162B360F4496B0D7CB93')
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
if($env:COMPUTERNAME -ne 'RDLAB-WIN11' -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'fixed SYSTEM only'}
$base='C:\Users\Public\shortcut-machine';$receipt="$base\rollback-$Scenario.json";$package="$base\failure.msi"
if(Test-Path $receipt){throw 'existing receipt'}
function Snapshot {
 $hashes=@{};foreach($dir in @('C:\Program Files\RustDesk','C:\ProgramData\RustDesk\ManagementAgent','C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config')){if(Test-Path $dir){foreach($f in Get-ChildItem -LiteralPath $dir -Recurse -File -Force){$hashes[$f.FullName]=(Get-FileHash -LiteralPath $f.FullName).Hash}}}
 @{hashes=$hashes;services=@(Get-CimInstance Win32_Service|Where-Object {$_.Name -in @('RustDesk','RustDeskManagementAgent')}|Select-Object Name,State,StartName,PathName);code=(Get-ItemProperty 'HKLM:\Software\RustDesk\InstallState\RustDesk').MsiProductCode}
}
$r=@{scenario=$Scenario;sid='S-1-5-18';phase='preflight'}
try {
 if(Get-Process msiexec -ErrorAction SilentlyContinue){throw 'installer active'}
 if((Get-FileHash $package).Hash -ne $ExpectedHash){throw 'hash mismatch'}
 $i=New-Object -ComObject WindowsInstaller.Installer;$old='{C220D796-2FB4-45F5-A921-87CF071C2E49}'
 if($i.ProductState($old) -ne 5){throw 'old not installed'}
 $link=Join-Path ([Environment]::GetFolderPath('CommonDesktopDirectory')) 'RustDesk.lnk'
 if(!(Test-Path $link)){throw 'baseline link absent'}
 Copy-Item -LiteralPath $link -Destination "$base\rollback-$Scenario.original.lnk"
 if($Scenario -eq 'missing'){Remove-Item -LiteralPath $link}
 $r.before=Snapshot;$r.before_link=Test-Path $link
 $r.phase='installing';$r|ConvertTo-Json -Depth 8|Set-Content $receipt
 $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "/i `"$package`" /qn /norestart REBOOT=ReallySuppress DEFENDER_EXCLUSIONS=0 /l*v `"$base\rollback-$Scenario.log`"" -Wait -PassThru
 $r.exit=$p.ExitCode;$r.after=Snapshot;$r.old_state=$i.ProductState($old);$r.after_link=Test-Path $link
 $r.changed=@($r.before.hashes.Keys|Where-Object {$r.before.hashes[$_] -ne $r.after.hashes[$_]})
 $r.added=@($r.after.hashes.Keys|Where-Object {!$r.before.hashes.ContainsKey($_)})
 $r.phase='observed'
 if($p.ExitCode -ne 1603 -or $r.old_state -ne 5 -or $r.after.code -ne $old -or $r.before_link -ne $r.after_link -or $r.changed.Count -ne 0 -or $r.added.Count -ne 0){throw 'rollback postcondition mismatch'}
 if(@($r.after.services|Where-Object {$_.State -ne 'Running'}).Count){throw 'service not restored'}
 $r.phase='pass'
}catch{$r.error=$_.Exception.Message}
$r|ConvertTo-Json -Depth 10|Set-Content -LiteralPath $receipt -Encoding UTF8
