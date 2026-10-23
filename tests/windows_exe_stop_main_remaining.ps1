param([Parameter(Mandatory=$true)][string]$Source)
$ErrorActionPreference='Stop'
if((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10'))){throw 'guest only'}
$text=[IO.File]::ReadAllText($Source)
$body=$text.Substring($text.IndexOf('function Save-MainIdentityDiagnostic('))
$body=$body.Substring(0,$body.IndexOf('function Invoke-Bounded('))
Invoke-Expression $body
if (-not (Get-Command Save-MainIdentityDiagnostic -ErrorAction SilentlyContinue)) { throw 'diagnostic helper missing from fixture extraction' }
$root=Join-Path $env:TEMP ('remaining-main-'+[Guid]::NewGuid().ToString('N'))
$results=@();$children=@()
function Stop-Service {param($Name,$ErrorAction)}
function Get-Service {param($Name) $v=[pscustomobject]@{};$v|Add-Member ScriptMethod WaitForStatus {param($s,$t)};$v}
function Get-CimInstance {
 param($ClassName,$Filter,$ErrorAction)
 $script:calls++
  if($Filter -match '^ProcessId=(\d+)$'){
   $processId=[int]$Matches[1]
   $process=@($first,$second|Where-Object Id -eq $processId)
   if(!$process -or $process.HasExited){return}
   return ,(CimCmdlets\Get-CimInstance Win32_Process -Filter "ProcessId=$processId")
  }
 if($script:calls -eq 1){
  $one=CimCmdlets\Get-CimInstance Win32_Process -Filter "ProcessId=$($first.Id)"
  $two=CimCmdlets\Get-CimInstance Win32_Process -Filter "ProcessId=$($second.Id)"
  [IO.File]::WriteAllText("$root\one.release",'release')
  if(!$first.WaitForExit(5000)){throw 'first exit not confirmed'}
  if($case -eq 'new_fixed'){return ,$one}
  return @($one,$two)
 }
 if($script:calls -eq 2 -and $case -eq 'same_pid'){
  return [pscustomobject]@{ProcessId=$first.Id;ExecutablePath="$root\RustDesk.exe";CreationDate=[DateTime]::Now}
 }
  if($script:calls -eq 2 -and $case -eq 'unknown'){
   return [pscustomobject]@{ProcessId=$second.Id;ExecutablePath=$null;CreationDate=[DateTime]::Now}
  }
  if($script:calls -eq 2 -and $case -eq 'stale_pid_with_other_name'){
   return [pscustomobject]@{ProcessId=$second.Id;ExecutablePath=$null;CreationDate=[DateTime]::Now}
  }
  if($case -eq 'stale_pid_with_other_name' -and $script:calls -eq 3){return}
 if($case -eq 'new_fixed' -and $script:calls -eq 2){return}
 if(!$second.HasExited){CimCmdlets\Get-CimInstance Win32_Process -Filter "ProcessId=$($second.Id)"}
}
try {
 New-Item -ItemType Directory $root|Out-Null
 Add-Type -OutputAssembly "$root\RustDesk.exe" -OutputType ConsoleApplication -TypeDefinition 'using System;using System.IO;using System.Threading;class P{static void Main(string[] a){while(!File.Exists(a[0]))Thread.Sleep(10);}}'
 foreach($script:case in @('original_exit_second_fixed','stale_pid_with_other_name','same_pid','unknown','new_fixed')){
  foreach($n in @('one','two')){Remove-Item "$root\$n.release" -ErrorAction SilentlyContinue}
  $children=@();$script:calls=0
  foreach($n in @('one','two')){$p=Start-Process "$root\RustDesk.exe" -ArgumentList "$root\$n.release" -PassThru;$null=$p.Handle;$children+=$p}
  $script:first=$children[0];$script:second=$children[1]
  [IO.File]::WriteAllText("$root\old-data",'retained')
  $errorText='';try{Stop-Main "$root\RustDesk.exe"}catch{$errorText=$_.Exception.Message}
  $passed=if($case -eq 'original_exit_second_fixed'){!$errorText -and $second.HasExited -and $calls -ge 4}elseif($case -eq 'stale_pid_with_other_name'){$errorText.StartsWith('[update-pending]') -and !$second.HasExited}else{$errorText.StartsWith('[update-pending]') -and !$second.HasExited}
  $passed=$passed -and [IO.File]::ReadAllText("$root\old-data") -eq 'retained'
  $results+=@{case=$case;passed=$passed;error=$errorText;enumerations=$calls;first_exited=$first.HasExited;second_exited=$second.HasExited}
  foreach($n in @('one','two')){[IO.File]::WriteAllText("$root\$n.release",'cleanup')}
  foreach($p in $children){if(!$p.WaitForExit(5000)){throw 'fixture cleanup timeout'};$p.Dispose()};$children=@()
 }
}finally{
 foreach($n in @('one','two')){if(Test-Path $root){[IO.File]::WriteAllText("$root\$n.release",'cleanup')}}
 foreach($p in $children){$null=$p.WaitForExit(5000);$p.Dispose()}
 Remove-Item $root -Recurse -Force
 @{source_sha256=(Get-FileHash $Source).Hash;results=$results}|ConvertTo-Json -Depth 5|Set-Content "$PSScriptRoot\remaining-main-result.json" -Encoding UTF8
}
if(@($results|Where-Object{!$_.passed}).Count -or $results.Count -ne 5){throw 'remaining process behavior failed'}
