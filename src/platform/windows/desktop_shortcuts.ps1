function Get-RustDeskShortcutSnapshot {
    param([string]$Path, [string]$ExpectedTarget)
    $result = [pscustomobject]@{ State = 'Unknown'; Path = $Path; Bytes = $null; Target = $ExpectedTarget; Error = $null }
    $stream = $null
    $shell = $null
    $link = $null
    try {
        if (![IO.Path]::IsPathRooted($Path) -or [IO.Path]::GetExtension($Path) -ine '.lnk') { throw 'invalid shortcut path' }
        # Refuse reparse ancestors as well as a reparse leaf. Callers must supply
        # only OS-resolved common desktop or the original user's desktop.
        $parent = [IO.DirectoryInfo]::new([IO.Path]::GetDirectoryName($Path))
        while ($null -ne $parent) {
            if (($parent.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'reparse directory refused' }
            $parent = $parent.Parent
        }
        try { $attributes = [IO.File]::GetAttributes($Path) }
        catch [IO.FileNotFoundException] { $result.State = 'Missing'; return $result }
        if (($attributes -band ([IO.FileAttributes]::ReparsePoint -bor [IO.FileAttributes]::Directory)) -ne 0) { throw 'non-regular shortcut refused' }
        $stream = [IO.File]::Open($Path,'Open','Read','Read')
        if ($stream.Length -lt 76 -or $stream.Length -gt 1048576) { throw 'invalid shortcut size' }
        $bytes = New-Object byte[] ([int]$stream.Length)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $n = $stream.Read($bytes,$offset,$bytes.Length-$offset)
            if ($n -eq 0) { throw 'incomplete shortcut read' }
            $offset += $n
        }
        if ([BitConverter]::ToUInt32($bytes,0) -ne 76 -or [Convert]::ToBase64String($bytes[4..19]) -ne 'ARQCAAAAAADAAAAAAAAARg==') { throw 'invalid shell link header' }
        $shell = New-Object -ComObject WScript.Shell
        $link = $shell.CreateShortcut($Path)
        if (![string]::Equals($link.TargetPath,$ExpectedTarget,[StringComparison]::OrdinalIgnoreCase)) {
            $result.State = 'Foreign'
            return $result
        }
        $result.Bytes = $bytes
        $result.State = 'Present'
    } catch { $result.Error = $_.Exception.Message }
    finally {
        if ($link) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($link) | Out-Null }
        if ($shell) { [Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) | Out-Null }
        if ($stream) { $stream.Dispose() }
    }
    return $result
}

function Restore-RustDeskShortcutSnapshot {
    param($Snapshot)
    if ($Snapshot.State -ne 'Present') { return $Snapshot.State }
    $current = Get-RustDeskShortcutSnapshot $Snapshot.Path $Snapshot.Target
    if ($current.State -eq 'Present' -and [Convert]::ToBase64String($current.Bytes) -eq [Convert]::ToBase64String($Snapshot.Bytes)) { return 'Unchanged' }
    if ($current.State -ne 'Missing') { return 'Conflict' }
    # CreateNew never overwrites a file appearing after the absence check.
    $file = [IO.File]::Open($Snapshot.Path,'CreateNew','Write','None')
    try { $file.Write($Snapshot.Bytes,0,$Snapshot.Bytes.Length); $file.Flush($true) }
    finally { $file.Dispose() }
    return 'Restored'
}
