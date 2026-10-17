param([string]$Output = "$PSScriptRoot\windows_exe_stop_identity_readonly.result.json")
$ErrorActionPreference = 'Stop'
if ($env:COMPUTERNAME -ne 'RDLAB-WIN10') { throw 'guest only' }
$installed = 'C:\Program Files\RustDesk\RustDesk.exe'
$rows = @(foreach ($candidate in Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'" -ErrorAction Stop) {
    $row = [ordered]@{ pid=$candidate.ProcessId; cimPath=$candidate.ExecutablePath; creationRaw=[string]$candidate.CreationDate; creationKind=$null; creationUtc=$null; creationTicks=$null; startRaw=$null; startKind=$null; startUtc=$null; startTicks=$null; mainModule=$null; missingCreation=(!$candidate.CreationDate); timeMismatch=$null; pathMismatch=$null; hasExited=$null; error=$null }
    $owned = $null
    try {
        if ($candidate.CreationDate) {
            $row.creationRaw=$candidate.CreationDate.ToString('O')
            $row.creationKind=[string]$candidate.CreationDate.Kind
            $row.creationUtc=$candidate.CreationDate.ToUniversalTime().ToString('O')
            $row.creationTicks=[string]$candidate.CreationDate.ToUniversalTime().Ticks
        }
        $owned=[Diagnostics.Process]::GetProcessById([int]$candidate.ProcessId)
        $null=$owned.Handle
        $start=$owned.StartTime
        $row.startRaw=$start.ToString('O'); $row.startKind=[string]$start.Kind
        $row.startUtc=$start.ToUniversalTime().ToString('O'); $row.startTicks=[string]$start.ToUniversalTime().Ticks
        $row.mainModule=$owned.MainModule.FileName
        $row.hasExited=$owned.HasExited
        $row.timeMismatch=([Math]::Abs(($start.ToUniversalTime()-$candidate.CreationDate.ToUniversalTime()).TotalMilliseconds) -ge 1)
        $row.pathMismatch=(![string]::Equals($row.mainModule,$installed,[StringComparison]::OrdinalIgnoreCase))
    } catch { $row.error=@{type=$_.Exception.GetType().FullName; message=$_.Exception.Message; inner=[string]$_.Exception.InnerException} }
    finally { if ($owned) { $owned.Dispose() } }
    [pscustomobject]$row
})
$result=@{machine=$env:COMPUTERNAME; sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; processes=$rows; services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,ProcessId)} | ConvertTo-Json -Depth 6
if ($Output) { [IO.File]::WriteAllText($Output,$result) }
$result
