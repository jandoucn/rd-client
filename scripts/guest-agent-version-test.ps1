param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference = 'Stop'
if (!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME) { throw 'Lab host required: set RUSTDESK_LAB_HOST to this COMPUTERNAME' }
$version = [Diagnostics.FileVersionInfo]::GetVersionInfo($Executable)
if ($version.FileVersion -ne '1.5.3+2026100502') { throw "Agent file version mismatch: $($version.FileVersion)" }
if ($version.OriginalFilename -ne 'rustdesk-management-agent.exe') { throw 'Agent original filename mismatch' }
if ($version.FileMajorPart -ne 1 -or $version.FileMinorPart -ne 5) { throw 'Agent numeric version mismatch' }
$build = [uint64]$version.FileBuildPart * 65536 + [uint64]$version.FilePrivatePart
if ($build -ne 2026100502) { throw "Agent numeric build mismatch: $build" }
@{status='PASS';file_version=$version.FileVersion;numeric_build=$build;computer=$env:COMPUTERNAME} | ConvertTo-Json
