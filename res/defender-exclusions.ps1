function Get-RdDefenderRegistry {
    [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
}
function Get-RdDefenderConsent {
    $root = Get-RdDefenderRegistry
    try {
        $key = $root.OpenSubKey('SOFTWARE\RustDesk\Defender')
        if ($null -eq $key) { return $null }
        try { $key.GetValue('Consent', $null) } finally { $key.Dispose() }
    } finally { $root.Dispose() }
}
function Set-RdDefenderConsent {
    param([string]$Choice)
    $root = Get-RdDefenderRegistry
    try {
        $key = $root.CreateSubKey('SOFTWARE\RustDesk\Defender')
        try { $key.SetValue('Consent', $Choice, [Microsoft.Win32.RegistryValueKind]::String) } finally { $key.Dispose() }
    } finally { $root.Dispose() }
}
function Get-RdDefenderOwnedPaths {
    $root = Get-RdDefenderRegistry
    try {
        $key = $root.OpenSubKey('SOFTWARE\RustDesk\Defender')
        if ($null -eq $key) { return @() }
        try { @($key.GetValue('OwnedPaths', @())) } finally { $key.Dispose() }
    } finally { $root.Dispose() }
}
function Set-RdDefenderOwnedPaths {
    param([string[]]$Paths)
    $root = Get-RdDefenderRegistry
    try {
        $key = $root.CreateSubKey('SOFTWARE\RustDesk\Defender')
        try { $key.SetValue('OwnedPaths', [string[]]$Paths, [Microsoft.Win32.RegistryValueKind]::MultiString) } finally { $key.Dispose() }
    } finally { $root.Dispose() }
}
function Set-RdDefenderResult {
    param($Result)
    $root = Get-RdDefenderRegistry
    try {
        $key = $root.CreateSubKey('SOFTWARE\RustDesk\Defender')
        try { $key.SetValue('LastResult', ($Result | ConvertTo-Json -Depth 5 -Compress), [Microsoft.Win32.RegistryValueKind]::String) } finally { $key.Dispose() }
    } finally { $root.Dispose() }
}
function Complete-RdDefenderResult {
    param($Result)
    try { Set-RdDefenderResult -Result $Result } catch { $Result.PersistenceError = $_.Exception.Message }
    return $Result
}
function Test-RdLocalPath {
    param([string]$Path, [bool]$File)
    if ([string]::IsNullOrWhiteSpace($Path) -or ![IO.Path]::IsPathRooted($Path) -or
        $Path -notmatch '^[A-Za-z]:[\\/]' -or $Path.Substring(2).Contains(':') -or
        $Path.StartsWith('\\') -or $Path -match '[\x00-\x1f*?]' -or $Path -match '(?:^|[\\/])\.\.(?:[\\/]|$)') { return $false }
    if ($File -and ![IO.Path]::GetExtension($Path).Equals('.exe', [StringComparison]::OrdinalIgnoreCase)) { return $false }
    if (!$File -and ($Path.TrimEnd('\','/').Length -le 2 -or [IO.Path]::GetFileName($Path.TrimEnd('\','/')).Length -eq 0)) { return $false }
    return $true
}
function Add-RdDefenderItem {
    param($Result, [string]$Kind, [string]$Path, [string[]]$Existing, [string[]]$Owners)
    $status = 'verified'; $detail = '原有配置已确认。'; $added = $false
    if ($Existing -notcontains $Path) {
        try {
            if ($Kind -eq 'ExclusionPath') { Add-MpPreference -ExclusionPath $Path -ErrorAction Stop }
            else { Add-MpPreference -ControlledFolderAccessAllowedApplications $Path -ErrorAction Stop }
            try {
                $readback = Get-MpPreference -ErrorAction Stop
                $field = if ($Kind -eq 'ExclusionPath') { 'ExclusionPath' } else { 'ControlledFolderAccessAllowedApplications' }
                if ($null -eq $readback.PSObject.Properties[$field]) { throw '写入后配置不可读取。' }
                $read = @($readback.$field)
                if ($Kind -eq 'ExclusionPath') { $script:RdExisting = $read } else { $script:RdAllowed = $read }
                if ($read -contains $Path) {
                    $added = $true; $detail = '已增量添加并读回确认。'
                    if ($Owners -notcontains $Path) { $script:RdOwners += $Path }
                    try { Set-RdDefenderOwnedPaths -Paths $script:RdOwners } catch { $Result.OwnershipError = $_.Exception.Message }
                } else { $status = 'unverified'; $detail = '写入未能读回，不能确认生效。' }
            } catch { $status = 'unverified'; $detail = $_.Exception.Message }
        } catch { $status = 'failed'; $detail = $_.Exception.Message }
    }
    $Result.Items += [pscustomobject]@{ Kind = $Kind; Path = $Path; Status = $status; Added = $added; Detail = $detail }
}
function Invoke-RdDefenderConfiguration {
    param([string]$Choice = 'inherit', [string]$MainPath, [string]$AgentPath)
    $result = [pscustomobject]@{ Schema = 2; Status = 'not_selected'; Items = @(); Error = ''; OwnershipError = ''; PersistenceError = ''; Automated360 = $false; Firewall = @(); ControlledFolderAccess = ''; CheckedAt = [DateTime]::UtcNow.ToString('o') }
    try {
        foreach ($path in @($MainPath, $AgentPath)) {
            if (!(Test-RdLocalPath -Path $path -File $true)) { throw '目标必须是本机精确 EXE 文件路径。' }
        }
        if ($Choice -eq 'inherit') { $Choice = Get-RdDefenderConsent }
        elseif ($Choice -eq '0' -or $Choice -eq '1') { Set-RdDefenderConsent -Choice $Choice }
        else { throw 'Defender 选择无效。' }
    } catch {
        $result.Status = 'consent_failed'; $result.Error = $_.Exception.Message
        return Complete-RdDefenderResult -Result $result
    }
    if (![string]::IsNullOrEmpty($Choice) -and $Choice -ne '0' -and $Choice -ne '1') {
        $result.Status = 'consent_failed'; $result.Error = '已保存的 Defender 选择无效。'
        return Complete-RdDefenderResult -Result $result
    }
    if ($Choice -ne '1') { return Complete-RdDefenderResult -Result $result }
    foreach ($path in @($MainPath, $AgentPath)) {
        $name = if ($path -eq $MainPath) { 'RustDesk Service' } else { 'RustDesk Management Agent Service' }
        $shown = & netsh.exe advfirewall firewall show rule name="$name" verbose
        $matched = @($shown | Select-String -SimpleMatch -Pattern $path).Count
        $result.Firewall += [pscustomobject]@{ Name = $name; Path = $path; Status = $(if ($matched -ge 2) {'verified'} else {'unverified'}); Matched = $matched }
    }
    $result.Status = 'configuring'
    try { Set-RdDefenderResult -Result $result } catch { $result.PersistenceError = $_.Exception.Message }
    if (!(Get-Command Get-MpPreference -ErrorAction SilentlyContinue) -or !(Get-Command Add-MpPreference -ErrorAction SilentlyContinue)) {
        $result.Status = 'unavailable'; $result.Error = 'Defender 官方命令不可用。'
    } else {
        $install = [IO.Path]::GetDirectoryName($MainPath).TrimEnd('\')
        $agent = [IO.Path]::GetDirectoryName($AgentPath).TrimEnd('\')
        $client = [IO.Path]::GetFullPath((Join-Path $agent '..\Client'))
        $folders = @($install, $agent, $client)
        try {
            foreach ($folder in $folders) { if (!(Test-RdLocalPath -Path $folder -File $false)) { throw '目录排除项不是本机精确目录。' } }
            $preferences = Get-MpPreference -ErrorAction Stop
            foreach ($name in @('ExclusionPath','EnableControlledFolderAccess','ControlledFolderAccessAllowedApplications')) {
                if ($null -eq $preferences -or $null -eq $preferences.PSObject.Properties[$name]) { throw 'Defender 配置不可核验。' }
            }
            $script:RdExisting = @($preferences.ExclusionPath)
            $script:RdOwners = @(Get-RdDefenderOwnedPaths)
            $cfa = [int]$preferences.EnableControlledFolderAccess
            $script:RdAllowed = @($preferences.ControlledFolderAccessAllowedApplications)
        } catch {
            $result.Status = 'read_failed'; $result.Error = $_.Exception.Message
        }
        if ($result.Status -ne 'read_failed') {
            foreach ($path in (@($MainPath, $AgentPath) + $folders) | Select-Object -Unique) {
                Add-RdDefenderItem -Result $result -Kind 'ExclusionPath' -Path $path -Existing $script:RdExisting -Owners $script:RdOwners
            }
            if ($cfa -in 1,2,3,4) {
                foreach ($path in @($MainPath, $AgentPath) | Select-Object -Unique) {
                    Add-RdDefenderItem -Result $result -Kind 'ControlledFolderAccessAllowedApplication' -Path $path -Existing $script:RdAllowed -Owners $script:RdOwners
                }
            } else {
                $result.ControlledFolderAccess = 'disabled'
            }
            $counted = @($result.Items | Where-Object { $_.Status -ne 'skipped' })
            $verified = @($counted | Where-Object { $_.Status -eq 'verified' }).Count
            $unverified = @($counted | Where-Object { $_.Status -eq 'unverified' }).Count
            $result.Status = if ($verified -eq $counted.Count) { 'verified' } elseif ($verified -gt 0) { 'partial' } elseif ($unverified -gt 0) { 'unverified' } else { 'failed' }
        }
    }
    return Complete-RdDefenderResult -Result $result
}
