param([string]$Directory)
$ErrorActionPreference='Stop'
$installer=New-Object -ComObject WindowsInstaller.Installer
$output=@{}
foreach($edition in 'standard','sos'){
    $db=$installer.OpenDatabase((Join-Path $Directory "$edition-TABLES-ONLY.msi"),0)
    $tables=@{}
    foreach($table in 'Component','Shortcut','Registry','Property','InstallExecuteSequence','Directory','CustomAction'){
        $v=$db.OpenView("SELECT * FROM ``$table``");$null=$v.Execute();$rows=@()
        while($r=$v.Fetch()){
            $count=$r.GetType().InvokeMember('FieldCount','GetProperty',$null,$r,$null)
            $row=@();for($i=1;$i -le $count;$i++){$row+= $r.StringData($i)}
            if(($row -join '|') -match 'App.Desktop.Shortcut|InspectDesktopShortcut|RustDeskDesktopAllowed|DESKTOPSHORTCUTS|ALLUSERS|DesktopFolder|CostFinalize|RemoveExistingProducts|FindRelatedProducts'){$rows+=,@($row)}
        }
        $null=$v.Close();if($rows.Count -eq 0){throw "no matching rows: $table"};$tables[$table]=$rows
    }
    $output[$edition]=$tables
}
$output | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $Directory 'tables.json')
if((ConvertTo-Json $output.standard -Depth 10 -Compress) -ne (ConvertTo-Json $output.sos -Depth 10 -Compress)){throw 'shortcut table differences'}
'PASS standard/SOS selected shortcut tables identical'
