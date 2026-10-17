$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\..\src\platform\windows\desktop_shortcuts.ps1"
$root = Join-Path ([IO.Path]::GetTempPath()) ('rustdesk-shortcut-fixture-' + [Guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory($root)
$shell = New-Object -ComObject WScript.Shell
$count = 0
function Assert($Condition, $Message) { if (!$Condition) { throw $Message } }
function Link($Path, $Target) {
    $link = $shell.CreateShortcut($Path)
    $link.TargetPath = $Target
    $link.Arguments = '--connect 123456'
    $link.Description = 'preserve user content'
    $link.Save()
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($link) | Out-Null
}
try {
    foreach ($edition in @('standard','sos')) {
        foreach ($desktop in @('common','user')) {
            $dir = Join-Path $root "$edition-$desktop"
            $null = [IO.Directory]::CreateDirectory($dir)
            $target = Join-Path $root "$edition\RustDesk.exe"
            $path = Join-Path $dir 'RustDesk.lnk'
            $missing = Get-RustDeskShortcutSnapshot $path $target
            Assert ($missing.State -eq 'Missing') 'absence misclassified'
            Assert ((Restore-RustDeskShortcutSnapshot $missing) -eq 'Missing') 'absence result'
            Assert (![IO.File]::Exists($path)) 'deleted shortcut recreated'
            $count++
            Link $path $target
            $bytes = [IO.File]::ReadAllBytes($path)
            $snapshot = Get-RustDeskShortcutSnapshot $path $target
            Assert ($snapshot.State -eq 'Present') 'real COM shortcut not captured'
            Assert ((Restore-RustDeskShortcutSnapshot $snapshot) -eq 'Unchanged') 'existing shortcut rewritten'
            $count++
            [IO.File]::Delete($path)
            Assert ((Restore-RustDeskShortcutSnapshot $snapshot) -eq 'Restored') 'old uninstall recovery failed'
            Assert ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -eq [Convert]::ToBase64String($bytes)) 'shortcut content changed'
            $readback = $shell.CreateShortcut($path)
            Assert ($readback.TargetPath -eq $target -and $readback.Arguments -eq '--connect 123456') 'COM readback differs'
            [Runtime.InteropServices.Marshal]::FinalReleaseComObject($readback) | Out-Null
            Assert (![IO.File]::Exists((Join-Path $dir 'RustDesk-SOS.lnk'))) 'another pathname created'
            $count++
            Link $path (Join-Path $root 'foreign.exe')
            $foreignBytes = [IO.File]::ReadAllBytes($path)
            Assert ((Restore-RustDeskShortcutSnapshot $snapshot) -eq 'Conflict') 'concurrent replacement overwritten'
            Assert ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -eq [Convert]::ToBase64String($foreignBytes)) 'foreign data lost'
            $foreign = Get-RustDeskShortcutSnapshot $path $target
            Assert ($foreign.State -eq 'Foreign') 'foreign target accepted'
            $count++
            [IO.File]::WriteAllText($path, 'corrupt link')
            $unknown = Get-RustDeskShortcutSnapshot $path $target
            Assert ($unknown.State -eq 'Unknown') 'corrupt link accepted'
            Assert ((Restore-RustDeskShortcutSnapshot $unknown) -eq 'Unknown') 'unknown restored'
            Assert ([IO.File]::ReadAllText($path) -eq 'corrupt link') 'corrupt original overwritten'
            $count++
            [IO.File]::Delete($path)
            Link $path $target
            $locked = [IO.File]::Open($path,'Open','ReadWrite','None')
            try {
                $unknown = Get-RustDeskShortcutSnapshot $path $target
                Assert ($unknown.State -eq 'Unknown' -and $null -ne $unknown.Error) 'read failure not unknown'
            } finally { $locked.Dispose() }
            Assert ((Restore-RustDeskShortcutSnapshot $unknown) -eq 'Unknown') 'read failure restored'
            Assert ([Convert]::ToBase64String([IO.File]::ReadAllBytes($path)) -eq [Convert]::ToBase64String((Get-RustDeskShortcutSnapshot $path $target).Bytes)) 'read failure lost original'
            $count++
            $legacy = Join-Path $dir "RustDesk-$edition-1.5.2.lnk"
            Link $legacy $target
            $legacySnapshot = Get-RustDeskShortcutSnapshot $legacy $target
            [IO.File]::Delete($legacy)
            Assert ((Restore-RustDeskShortcutSnapshot $legacySnapshot) -eq 'Restored') 'legacy pathname not retained'
            Assert (![IO.File]::Exists((Join-Path $dir "RustDesk-$edition-1.5.3.lnk"))) 'new version pathname created'
            $count++
        }
    }
    "PASS: $count actual shortcut/absence cases"
} finally {
    [Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) | Out-Null
    [IO.Directory]::Delete($root, $true)
}
