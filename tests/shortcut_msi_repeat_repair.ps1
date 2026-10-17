param([string]$PackageDirectory,[string]$Receipt)
$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -ne 'RDLAB-WIN11'){throw 'Win11 only'}
$product='{D38BA039-2E82-4915-A8B1-B616520CCAB1}'
$root=Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'RDShortcutFixture-7BC91'
$link=Join-Path $root 'Desktop\RustDesk.lnk'
$foreign=Join-Path $root 'Desktop\OtherProduct.lnk'
$msi=Join-Path $PackageDirectory 'fixture-1.0.0.msi'
$new=Join-Path $PackageDirectory 'fixture-2.0.0.msi'
$results=@()
function Snapshot {
    $files=@{}
    foreach($dir in @('C:\Program Files\RustDesk','C:\ProgramData\RustDesk\ManagementAgent','C:\ProgramData\RustDesk\RecoveryBootstrap')) {
        if(Test-Path -LiteralPath $dir){foreach($f in Get-ChildItem -LiteralPath $dir -Recurse -File -Force){$files[$f.FullName]=(Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash}}
    }
    $tasks=@{}
    foreach($t in Get-ScheduledTask | Where-Object {$_.TaskName -like 'RustDesk*'}){$tasks[$t.TaskName]=Export-ScheduledTask -TaskName $t.TaskName -TaskPath $t.TaskPath}
    @{files=$files;tasks=$tasks;services=@(Get-Service RustDesk,RustDeskManagementAgent | ForEach-Object {"$($_.Name):$($_.Status)"})}
}
function Run($arguments,$label,$expected=0){
    $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "$arguments /qn /norestart /l*v `"$PackageDirectory\repeat-$label.log`"" -PassThru -Wait
    if($p.ExitCode -ne $expected){throw "$label exit $($p.ExitCode)"}
}
function Check($value,$label){$script:results+=@{case=$label;passed=[bool]$value};if(!$value){throw $label}}
$before=Snapshot
try{
    $installer=New-Object -ComObject WindowsInstaller.Installer
    Check ($installer.ProductState($product) -eq -1) 'fixture-unregistered-before'
    $null=New-Item -ItemType Directory -Path (Split-Path $link) -Force
    $shell=New-Object -ComObject WScript.Shell
    $other=$shell.CreateShortcut($foreign);$other.TargetPath="$env:SystemRoot\System32\notepad.exe";$other.Save()
    $foreignHash=(Get-FileHash -LiteralPath $foreign).Hash
    Run "/i `"$msi`"" 'install'
    Check (Test-Path $link) 'installed-link'
    Remove-Item -LiteralPath $link
    foreach($i in 1,2){
        Run "/famus $product" "repair-$i"
        Check (!(Test-Path $link)) "repair-$i-missing"
        Check ($installer.ProductState($product) -eq 5) "repair-$i-same-product"
        Check ((Get-FileHash -LiteralPath $foreign).Hash -eq $foreignHash) "repair-$i-foreign-preserved"
    }
    Run "/i `"$new`" FAILFIXTURE=1" 'rollback' 1603
    Check (!(Test-Path $link)) 'rollback-still-missing'
    Check ($installer.ProductState($product) -eq 5) 'rollback-old-product'
    Run "/x $product" 'cleanup'
    Check ($installer.ProductState($product) -eq -1) 'fixture-unregistered-after'
    Check ((Get-FileHash -LiteralPath $foreign).Hash -eq $foreignHash) 'cleanup-foreign-preserved'
    Remove-Item -LiteralPath $foreign
}catch{$failure=$_.Exception.Message}
$after=Snapshot
foreach($p in $before.files.Keys){if($before.files[$p] -ne $after.files[$p]){$failure="product file changed: $p"}}
if($before.files.Count -ne $after.files.Count){$failure='product file count changed'}
foreach($t in $before.tasks.Keys){if($before.tasks[$t] -ne $after.tasks[$t]){$failure="product task changed: $t"}}
if($before.tasks.Count -ne $after.tasks.Count -or ($before.services -join '|') -ne ($after.services -join '|')){$failure='product tasks/services changed'}
@{results=$results;failure=$failure;before=$before;after=$after} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Receipt
if($failure){throw $failure}
'PASS consecutive repair and rollback, product snapshot unchanged'
