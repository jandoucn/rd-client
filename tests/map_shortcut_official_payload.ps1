param([string]$Msi,[string]$Export,[string]$Output)
$ErrorActionPreference='Stop'
$installer=New-Object -ComObject WindowsInstaller.Installer
$db=$installer.OpenDatabase($Msi,0)
function Rows($sql,$count){$v=$db.OpenView($sql);$null=$v.Execute();while($r=$v.Fetch()){$a=@();for($n=1;$n -le $count;$n++){$a+=$r.StringData($n)};Write-Output -NoEnumerate $a};$null=$v.Close()}
$dirs=@{};foreach($r in (Rows 'SELECT `Directory`,`Directory_Parent`,`DefaultDir` FROM `Directory`' 3)){$dirs[$r[0]]=@($r[1],$r[2])}
$components=@{};foreach($r in (Rows 'SELECT `Component`,`Directory_` FROM `Component`' 2)){$components[$r[0]]=$r[1]}
function Relative($id){if($id -eq 'INSTALLFOLDER_INNER'){return ''};if(!$dirs.ContainsKey($id)){throw 'unknown directory'};$d=$dirs[$id];if(!$d[0]){throw 'outside main payload'};$name=($d[1] -split ':')[0];$name=($name -split '\|')[-1];$parent=Relative $d[0];if($name -eq '.') {return $parent};return "$parent\$name".TrimStart('\')}
$null=New-Item -ItemType Directory -Path $Output
$manifest=@();foreach($r in (Rows 'SELECT `File`,`Component_`,`FileName`,`FileSize` FROM `File`' 4)){
    $dir=$components[$r[1]]
    if($dir -in @('ManagementAgent.Folder','RecoveryBootstrap.Folder')){continue}
    $relative=Relative $dir
    $name=($r[2] -split '\|')[-1]
    $source=Join-Path "$Export\File" $r[0];$dest=Join-Path $Output "$relative\$name".TrimStart('\')
    if((Get-Item -LiteralPath $source).Length -ne [long]$r[3]){throw 'size mismatch'}
    $null=New-Item -ItemType Directory -Path (Split-Path $dest) -Force
    Copy-Item -LiteralPath $source -Destination $dest
    $manifest+=@{file=$r[0];relative="$relative\$name";sha256=(Get-FileHash -LiteralPath $dest).Hash}
}
$props=@{};foreach($r in (Rows 'SELECT `Property`,`Value` FROM `Property`' 2)){if($r[0] -in @('ProductCode','ProductVersion','UpgradeCode','ProductName')){$props[$r[0]]=$r[1]}}
@{msi_sha256=(Get-FileHash -LiteralPath $Msi).Hash;properties=$props;files=$manifest}|ConvertTo-Json -Depth 6|Set-Content "$Output.manifest.json" -Encoding UTF8
