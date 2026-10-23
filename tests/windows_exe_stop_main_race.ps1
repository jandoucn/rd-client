param([string]$Source)
$ErrorActionPreference='Stop'
if((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win10'))){throw 'guest only'}
$text=[IO.File]::ReadAllText($Source)
Invoke-Expression ($text.Split(@('const SCRIPT: &str = r###"'),[StringSplitOptions]::None)[1].Split(@('"###;'),[StringSplitOptions]::None)[0])
$root=Join-Path $env:TEMP ('main-race-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $root | Out-Null
$r=@{passed=$false}
try {
    Add-Type -OutputAssembly "$root\RustDesk.exe" -OutputType ConsoleApplication -TypeDefinition 'using System;using System.IO;using System.Threading;class P{static void Main(){while(!File.Exists(AppDomain.CurrentDomain.BaseDirectory+"release"))Thread.Sleep(10);}}'
    $child=Start-Process "$root\RustDesk.exe" -PassThru
    $null=$child.Handle
    $script:stale=[pscustomobject]@{ProcessId=$child.Id;ExecutablePath="$root\RustDesk.exe";CreationDate=$child.StartTime}
    $script:calls=0
    function Stop-Service {param($Name,$ErrorAction)}
    function Get-Service {param($Name) $s=[pscustomobject]@{};$s|Add-Member ScriptMethod WaitForStatus {param($s,$t)};$s}
    function Get-CimInstance {param($ClassName,$Filter,$ErrorAction)
        $script:calls++
        if($script:calls -eq 1){
            [IO.File]::WriteAllText("$root\release",'release')
            if(!$child.WaitForExit(5000)){throw 'fixture failed to exit'}
            $script:stale
        }
    }
    Stop-Main "$root\RustDesk.exe"
    if(!$child.HasExited -or $script:calls -lt 2){throw 'exit not independently rechecked'}
    $r.passed=$true
}catch{$r.error=$_.Exception.GetType().FullName}
finally{if($child){$child.Dispose()};Remove-Item $root -Recurse -Force;$r|ConvertTo-Json|Set-Content "$PSScriptRoot\stop-main-race-result.json"}
if(!$r.passed){throw $r.error}
