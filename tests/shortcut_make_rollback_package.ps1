param([string]$Source,[string]$Destination)
$ErrorActionPreference='Stop'
if((Get-FileHash -LiteralPath $Source).Hash -ne 'D8771B1B0304DF6FFA63F28E99B6EBE08C1244CC63F60839134E5A1A9906D675'){throw 'source digest mismatch'}
if(Test-Path -LiteralPath $Destination){throw 'destination exists'}
Copy-Item -LiteralPath $Source -Destination $Destination
$i=New-Object -ComObject WindowsInstaller.Installer
$db=$i.OpenDatabase($Destination,1)
function Execute($sql){$v=$db.OpenView($sql);$null=$v.Execute();$null=$v.Close()}
$v=$db.OpenView('SELECT `Sequence` FROM `InstallExecuteSequence` WHERE `Action`=''InstallExecute''');$null=$v.Execute();$r=$v.Fetch();$sequence=[int]$r.StringData(1)+1;$null=$v.Close()
Execute 'INSERT INTO `CustomAction` (`Action`,`Type`,`Target`) VALUES (''ShortcutLabFail'',19,''Shortcut lab synchronous failure after InstallExecute'')'
Execute "INSERT INTO ``InstallExecuteSequence`` (``Action``,``Condition``,``Sequence``) VALUES ('ShortcutLabFail','NOT Installed',$sequence)"
$null=$db.Commit()
[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($r)
[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($v)
[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($db)
@{source_sha256=(Get-FileHash $Source).Hash;candidate_sha256=(Get-FileHash $Destination).Hash;action='ShortcutLabFail';type=19;sequence=$sequence;production_source_changed=$false}|ConvertTo-Json|Set-Content "$Destination.manifest.json"
