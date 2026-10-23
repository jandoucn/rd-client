$ErrorActionPreference='Stop'
if(!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME){throw 'Only the declared lab host may run this test: set RUSTDESK_LAB_HOST to this COMPUTERNAME'}
$folder='C:\RustDeskAgentLab'
$statePath='C:\ProgramData\RustDesk\ManagementAgent\state\agent.json'
$agentPath='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$mainPath='C:\Program Files\RustDesk\RustDesk.exe'
$result=[ordered]@{schema=1;computer=$env:COMPUTERNAME;status='FAIL';cases=@();checked_at=[DateTime]::UtcNow.ToString('o')}
function Snapshot {
 $state=Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
 $services=@(Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,StartName,StartMode,PathName,ProcessId)
 $agent=[Diagnostics.FileVersionInfo]::GetVersionInfo($agentPath)
 [ordered]@{
  state=$state
  services=$services
  agent_hash=(Get-FileHash -LiteralPath $agentPath -Algorithm SHA256).Hash
  agent_file_version=$agent.FileVersion
  main_present=(Test-Path -LiteralPath $mainPath -PathType Leaf)
 }
}
function Case($id,$ok,$data){$result.cases+=@{id=$id;status=$(if($ok){'PASS'}else{'FAIL'});data=$data}}
try {
 $first=Snapshot
 $state=$first.state
 $result.cases+=@{id='post-upgrade-agent-capabilities';status='NOT_RUN';data=@{reason='Requires actual signed agent status payload from this installed build; local agent.json and hardcoded arrays are not capability evidence'}}
 Case 'post-upgrade-agent-identity' (![string]::IsNullOrWhiteSpace($state.agent_installation_id) -and $state.agent_installation_id.Length -ge 16) @{installation_id=$state.agent_installation_id;client_id=$state.client_id;client_uuid=$state.client_uuid}
 Case 'post-upgrade-agent-target' ($state.edition -eq 'standard' -and $state.package_kind -eq 'msi' -and $state.normal_install_path -eq 'C:\Program Files\RustDesk') @{edition=$state.edition;package_kind=$state.package_kind;normal_install_path=$state.normal_install_path}
 $svc=@($first.services|Where-Object {$_.Name -eq 'RustDeskManagementAgent'})[0]
 Case 'post-upgrade-agent-service-contract' ($null -ne $svc -and $svc.State -eq 'Running' -and $svc.StartName -eq 'LocalSystem' -and $svc.StartMode -eq 'Auto' -and $svc.PathName -match 'rustdesk-management-agent.exe.*--service.*--edition standard.*--package-kind msi') @{service=$svc}
 Case 'post-upgrade-agent-files' ($first.main_present -and (Test-Path -LiteralPath $agentPath -PathType Leaf) -and $first.agent_hash -match '^[0-9A-F]{64}$') @{main_present=$first.main_present;agent_hash=$first.agent_hash;agent_version=$first.agent_file_version}
 $result.before_restart=$first
 $result.status=if(@($result.cases|Where-Object {$_.status -eq 'FAIL'}).Count){'FAIL'}elseif(@($result.cases|Where-Object {$_.status -ne 'PASS'}).Count){'INCOMPLETE'}else{'PASS'}
}catch{$result.error=$_.Exception.Message}
finally{$result.checked_at=[DateTime]::UtcNow.ToString('o');$result|ConvertTo-Json -Depth 12|Set-Content -LiteralPath (Join-Path $folder 'post-upgrade-capability-result.json') -Encoding utf8}
if($result.status -ne 'PASS'){exit 1};exit 0
