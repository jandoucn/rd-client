param([string]$PackageDirectory)
$ErrorActionPreference='Stop'
if ($env:COMPUTERNAME -ne 'RDLAB-WIN11') { throw 'Win11 only' }
$installer=New-Object -ComObject WindowsInstaller.Installer
$db=$installer.OpenDatabase((Join-Path $PackageDirectory 'fixture-2.0.0.msi'),0)
function Rows($Sql) {
    $view=$db.OpenView($Sql); $null=$view.Execute()
    $items=@()
    while ($record=$view.Fetch()) { $items += [pscustomobject]@{Name=$record.StringData(1);Value=$record.StringData(2)} }
    $null=$view.Close(); return $items
}
$sequence=@{}
foreach($row in (Rows 'SELECT `Action`, `Sequence` FROM `InstallExecuteSequence`')) { $sequence[$row.Name]=[int]$row.Value }
if (!($sequence.FindRelatedProducts -lt $sequence.InspectDesktopShortcut -and $sequence.InspectDesktopShortcut -lt $sequence.CostFinalize -and $sequence.CostFinalize -lt $sequence.RemoveExistingProducts)) { throw 'table sequence invalid' }
$component=Rows 'SELECT `Component`, `Attributes` FROM `Component`'
$desktop=$component | Where-Object { $_.Name -eq 'App.Desktop.Shortcut' }
if (([int]$desktop.Value -band 64) -ne 64) { throw 'not transitive in native table' }
foreach($label in @('upgrade-existing','upgrade-missing')) {
    $text=Get-Content -LiteralPath (Join-Path $PackageDirectory "$label.log") -Raw
    $related=$text.IndexOf('PROPERTY CHANGE: Adding WIX_UPGRADE_DETECTED')
    $inspect=$text.IndexOf('Doing action: InspectDesktopShortcut')
    $cost=$text.IndexOf('Doing action: CostFinalize')
    $remove=$text.IndexOf('Doing action: RemoveExistingProducts')
    if (!($related -ge 0 -and $related -lt $inspect -and $inspect -lt $cost -and $cost -lt $remove)) { throw "$label logged sequence invalid" }
}
'PASS native Component Transitive bit and two upgrade log sequences'
