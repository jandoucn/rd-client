$ErrorActionPreference='Stop'
if($env:COMPUTERNAME -ne 'RDLAB-WIN11'){throw 'Win11 only'}
$installer=New-Object -ComObject WindowsInstaller.Installer
foreach($code in '{D38BA039-2E82-4915-A8B1-B616520CCAB1}','{D38BA039-2E82-4915-A8B1-B616520CCAB2}'){
    $state=$installer.ProductState($code)
    "before $code state=$state"
    if($state -ne -1){
        $p=Start-Process "$env:SystemRoot\System32\msiexec.exe" -ArgumentList "/x $code /qn /norestart" -Wait -PassThru
        "uninstall exit=$($p.ExitCode)"
    }
    if($installer.ProductState($code) -ne -1){throw "fixture remains $code"}
}
$foreign=Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'RDShortcutFixture-7BC91\Desktop\OtherProduct.lnk'
if(Test-Path -LiteralPath $foreign){Remove-Item -LiteralPath $foreign}
'fixture registrations absent; test-owned sentinel removed'
