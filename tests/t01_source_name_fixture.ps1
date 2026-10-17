param([Parameter(Mandatory=$true)][string]$ExpectedMsiSha256,[switch]$ElevatedSnapshots,[string]$CollectSnapshot)
$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -ne 'RDLAB-WIN11'){throw 'Win11 owner only'}
$code='{127981AF-9DA4-4B73-A731-9E07CCCB0901}'
$marker='HKCU:\Software\T01SourceNameFixture-A731'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
if($sid -eq 'S-1-5-18'){throw 'Use interactive per-user owner, not SYSTEM'}
$paths=@('C:\ProgramData\RustDesk\ManagementAgent\state\agent.json','C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml','C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk2.toml','C:\Program Files\RustDesk\RustDesk.exe','C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe')
$elevated=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if($CollectSnapshot){
 $snapshot=@{complete=$false;hashes=@{};services=@();error=$null;sid=$sid;identity=[Security.Principal.WindowsIdentity]::GetCurrent().Name;elevated=$elevated;computer=$env:COMPUTERNAME;utc=[DateTime]::UtcNow.ToString('o')}
 try {
  if(!$elevated){throw 'Collector requires visible administrator elevation'}
  foreach($p in $paths){if(Test-Path -LiteralPath $p -ErrorAction Stop){$snapshot.hashes[$p]=(Get-FileHash -LiteralPath $p -ErrorAction Stop).Hash}else{$snapshot.hashes[$p]='ABSENT'}}
  $snapshot.services=@(Get-Service -ErrorAction Stop | Where-Object {$_.Name -in @('RustDesk','RustDeskManagementAgent')} | Sort-Object Name | ForEach-Object {$_.Name+':'+$_.Status})
  $snapshot.complete=$true
 } catch {$snapshot.error=$_.Exception.Message}
 $snapshot | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $CollectSnapshot -Encoding UTF8
 if(!$snapshot.complete){exit 1};exit 0
}
if($elevated){throw 'MSI must run with normal user token'}
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class T01SourceApi {
 [DllImport("msi.dll", CharSet=CharSet.Unicode)] public static extern uint MsiSourceListGetInfoW(string code,string sid,uint context,uint options,string property,StringBuilder value,ref uint length);
 [DllImport("msi.dll", CharSet=CharSet.Unicode)] public static extern uint MsiGetProductInfoExW(string code,string sid,uint context,string property,StringBuilder value,ref uint length);
 public static string Source(string code,string property) {
  var value=new StringBuilder(32768); uint size=32768;
  uint error=MsiSourceListGetInfoW(code,null,2,0,property,value,ref size);
  if(error!=0)throw new Exception("Source query "+property+" error="+error);
  return value.ToString();
 }
 public static uint Registered(string code) {
  var value=new StringBuilder(32768); uint size=32768;
  return MsiGetProductInfoExW(code,null,2,"InstalledProductName",value,ref size);
 }
}
'@
if((Test-Path $marker) -or [T01SourceApi]::Registered($code) -ne 1605){throw 'Existing fixture registration/marker must be preserved'}
foreach($name in @('t01-source-original.msi','package.msi')){
 if((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $name)).Hash -ne $ExpectedMsiSha256){throw 'MSI hash mismatch'}
}
$evidence=Join-Path $PSScriptRoot ('run-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $evidence | Out-Null
$paths=@('C:\ProgramData\RustDesk\ManagementAgent\state\agent.json','C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml','C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk2.toml','C:\Program Files\RustDesk\RustDesk.exe','C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe')
function Snapshot {
 if($ElevatedSnapshots){
  $output=Join-Path $evidence ('snapshot-'+[guid]::NewGuid().ToString('N')+'.json')
  $collector=Start-Process "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -Verb RunAs -PassThru -ArgumentList ('-NoProfile -ExecutionPolicy Bypass -File "'+$PSCommandPath+'" -ExpectedMsiSha256 '+$ExpectedMsiSha256+' -CollectSnapshot "'+$output+'"')
  if(!$collector.WaitForExit(60000)){throw 'Collector pending; inspect elevation and receipt before retry'}
  if(!(Test-Path -LiteralPath $output)){throw 'Collector receipt missing'}
  $s=Get-Content -LiteralPath $output -Raw | ConvertFrom-Json
  if(!$s.complete -or !$s.elevated -or $s.error -or $s.computer -ne 'RDLAB-WIN11'){throw 'Collector incomplete; preservation unknown'}
  $hashes=@{};foreach($p in $paths){$v=$s.hashes.PSObject.Properties[$p].Value;if($v -ne 'ABSENT' -and $v -notmatch '^[A-Fa-f0-9]{64}$'){throw 'Missing or invalid snapshot hash'};$hashes[$p]=$v}
  return @{hashes=$hashes;services=@($s.services);collector_receipt=$output}
 }
 $hashes=@{};foreach($p in $paths){if(Test-Path -LiteralPath $p){$hashes[$p]=(Get-FileHash -LiteralPath $p).Hash}else{$hashes[$p]='ABSENT'}}
 $services=@(Get-Service -Name RustDesk,RustDeskManagementAgent -ErrorAction SilentlyContinue | Sort-Object Name | ForEach-Object { $_.Name+':'+$_.Status })
 return @{hashes=$hashes;services=$services}
}
function Run-Msi([string]$arguments,[string]$lane) {
 $p=Start-Process -FilePath "$env:WINDIR\System32\msiexec.exe" -ArgumentList $arguments -Wait -PassThru -RedirectStandardOutput (Join-Path $evidence ($lane+'.stdout')) -RedirectStandardError (Join-Path $evidence ($lane+'.stderr'))
 return $p.ExitCode
}
$before=$null
$receipt=@{state='started';product_code=$code;context='USERUNMANAGED(2)';sid=$sid;identity=[Security.Principal.WindowsIdentity]::GetCurrent().Name;session=(Get-Process -Id $PID).SessionId;elevated=$elevated;before=$null;old_data_preserved=$null;services_preserved=$null;cases=@();error=$null;cleanup_error=$null}
$owned=$false
try {
  $before=Snapshot;$receipt.before=$before
 foreach($name in @('t01-source-original.msi','package.msi')) {
  $owned=$true
  $package=Join-Path $PSScriptRoot $name
  $log=Join-Path $evidence ($name+'.install.log')
   $exit=Run-Msi ('/i "'+$package+'" /qn /norestart /L*v "'+$log+'"') ($name+'.install')
   $receipt.cases+=@{invoked_basename=$name;install_exit=$exit;uninstall_exit=$null}
  if($exit -ne 0){throw "Fixture install exit=$exit"}
  if([T01SourceApi]::Registered($code) -ne 0 -or !(Test-Path $marker)){throw 'Fixture install registration missing'}
  $observed=[T01SourceApi]::Source($code,'PackageName')
  $source=[T01SourceApi]::Source($code,'LastUsedSource')
  $type=[T01SourceApi]::Source($code,'LastUsedType')
   $receipt.cases[-1].PackageName=$observed;$receipt.cases[-1].LastUsedSource=$source;$receipt.cases[-1].LastUsedType=$type;$receipt.cases[-1].production_basename_comparison=($observed -eq 't01-source-original.msi')
  if($observed -ne $name){throw 'Observed PackageName differs from invoked basename; retain evidence as counterexample'}
  if($type -ne 'n'){throw 'Expected local source type n'}
  $registered=Join-Path $source $observed
  if((Get-FileHash -LiteralPath $registered).Hash -ne $ExpectedMsiSha256){throw 'Registered source bytes differ'}
  $log=Join-Path $evidence ($name+'.uninstall.log')
  $exit=Run-Msi ('/x '+$code+' /qn /norestart /L*v "'+$log+'"') ($name+'.uninstall')
  $receipt.cases[-1].uninstall_exit=$exit
   if($exit -ne 0){throw "Fixture uninstall exit=$exit"}
   $receipt.cases[-1].registration_after_uninstall=[T01SourceApi]::Registered($code)
   $receipt.cases[-1].marker_after_uninstall=Test-Path $marker
  if([T01SourceApi]::Registered($code) -ne 1605 -or (Test-Path $marker)){throw 'Fixture uninstall left registration or marker'}
  $owned=$false
 }
 $receipt.state='observed'
} catch { $receipt.state='failed';$receipt.error=$_.Exception.Message }
finally {
 if($owned){
  try {
   $log=Join-Path $evidence 'finally-uninstall.log'
   $exit=Run-Msi ('/x '+$code+' /qn /norestart /L*v "'+$log+'"') 'finally-uninstall'
   $receipt.finally_uninstall_exit=$exit
   if($exit -notin @(0,1605)){throw "Cleanup uninstall exit=$exit"}
  } catch {$receipt.cleanup_error=$_.Exception.Message}
 }
  try {
   $receipt.registration_after=[T01SourceApi]::Registered($code)
   $receipt.fixture_absent=($receipt.registration_after -eq 1605 -and !(Test-Path $marker))
   if($null -eq $before){throw 'Before snapshot unavailable; preservation unknown'}
   $after=Snapshot;$receipt.after=$after
  $same=$true;foreach($p in $paths){if($before.hashes[$p] -ne $after.hashes[$p]){$same=$false}}
  $receipt.old_data_preserved=$same
  $receipt.services_preserved=(($before.services -join '|') -eq ($after.services -join '|'))
  $receipt.fixture_absent=([T01SourceApi]::Registered($code) -eq 1605 -and !(Test-Path $marker))
 } catch {$receipt.cleanup_error=$_.Exception.Message}
 $receipt.finished=[DateTime]::UtcNow.ToString('o')
 $receipt|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $evidence 'result.json') -Encoding UTF8
 Write-Output "T01 source-name evidence: $evidence"
}
if($receipt.state -ne 'observed' -or !$receipt.old_data_preserved -or !$receipt.services_preserved -or !$receipt.fixture_absent -or $receipt.cleanup_error){exit 1}
exit 0
