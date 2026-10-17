param([Parameter(Mandatory=$true)][string]$Source)
$ErrorActionPreference='Stop'
if ($env:COMPUTERNAME -ne 'RDLAB-WIN10') { throw 'guest required' }
$text=[IO.File]::ReadAllText($Source)
Invoke-Expression ($text.Split(@('const SCRIPT: &str = r###"'),[StringSplitOptions]::None)[1].Split(@('"###;'),[StringSplitOptions]::None)[0])
$root=Join-Path $env:TEMP ('legacy-hidden-'+[Guid]::NewGuid().ToString('N'))
$r=@{passed=$false;source_sha256=(Get-FileHash $Source).Hash}
try {
    New-Item -ItemType Directory "$root\state" -Force | Out-Null
    [IO.File]::WriteAllText("$root\state\agent.json",'{"opaque":null,"id":"original"}')
    [IO.File]::SetAttributes("$root\state\agent.json",[IO.FileAttributes]::Hidden -bor [IO.FileAttributes]::System)
    $before=Get-Digest "$root\state\agent.json"
    Copy-TreeChecked "$root\state" "$root\backup"
    if ((Get-Digest "$root\backup\agent.json") -ne $before -or ((Get-Item "$root\backup\agent.json" -Force).Attributes -band 6) -ne 6) { throw 'hidden backup attributes/bytes lost' }
    [IO.File]::SetAttributes("$root\state\agent.json",[IO.FileAttributes]::Normal)
    [IO.File]::WriteAllText("$root\state\agent.json",'changed')
    & "$env:SystemRoot\System32\robocopy.exe" "$root\backup" "$root\state" /MIR /IS /IT /COPY:DAT /DCOPY:DAT /R:1 /W:1 | Out-Null
    if ($LASTEXITCODE -ge 8 -or (Get-Digest "$root\state\agent.json") -ne $before -or ((Get-Item "$root\state\agent.json" -Force).Attributes -band 6) -ne 6) { throw 'hidden rollback attributes/bytes lost' }
    $r.passed=$true
} catch { $r.error=$_.Exception.Message }
finally {
    if (Test-Path $root) { Remove-Item $root -Recurse -Force }
    $r | ConvertTo-Json | Set-Content "$PSScriptRoot\legacy-hidden-result.json" -Encoding UTF8
}
if (!$r.passed) { throw $r.error }
