param([Parameter(Mandatory=$true)][ValidatePattern('^[a-z0-9-]{8,64}$')][string]$RunId)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if((!$env:RUSTDESK_LAB_HOST -or $env:RUSTDESK_LAB_HOST -ne $env:COMPUTERNAME -or ($env:RUSTDESK_LAB_OS -and $env:RUSTDESK_LAB_OS -ne 'win11')) -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18'){throw 'VM_SYSTEM_REQUIRED'}
$root='C:\RustDeskAgentLab'
$output=Join-Path $root "$RunId.tests.json"
if(Test-Path -LiteralPath $output){throw 'RUN_ID_REUSED'}
$results=New-Object System.Collections.ArrayList
$setupError=$null
function Test-Case([string]$Name,[scriptblock]$Body){
    try { & $Body; [void]$results.Add(@{name=$Name;state='pass'}) }
    catch { [void]$results.Add(@{name=$Name;state='fail';error=$_.Exception.Message}) }
}
function Equal($Actual,$Expected){if($Actual -cne $Expected){throw "expected <$Expected>, actual <$Actual>"}}
function Reject([scriptblock]$Body,[string]$Code){
    try { & $Body } catch {if($_.Exception.Message -like "*$Code*"){return};throw}
    throw "expected rejection: $Code"
}
try {
    $common=Join-Path $PSScriptRoot '..\scripts\guest-agent-acceptance-common.ps1'
    . $common
    Test-Case 'all persistent drivers parse under Windows PowerShell 5.1' {
        foreach($file in @(Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot '..\scripts') -Filter 'guest-agent-acceptance-*.ps1')){
            $tokens=$null;$errors=$null
            [void][Management.Automation.Language.Parser]::ParseFile($file.FullName,[ref]$tokens,[ref]$errors)
            if($errors.Count){throw ($errors|Out-String)}
        }
    }
    Test-Case 'guard rejects undeclared lab host even with SYSTEM' {
        Reject {Assert-AcceptanceContext 'YAN' 'S-1-5-18' ''} 'VM_SYSTEM_REQUIRED'
        Reject {Assert-AcceptanceContext 'YAN' 'S-1-5-18' $null} 'VM_SYSTEM_REQUIRED'
    }
    Test-Case 'guard rejects ordinary user and other declared host' {
        Reject {Assert-AcceptanceContext 'YAN' 'S-1-5-21' 'YAN'} 'VM_SYSTEM_REQUIRED'
        Reject {Assert-AcceptanceContext 'YAN' 'S-1-5-18' 'OTHER-HOST'} 'VM_SYSTEM_REQUIRED'
    }
    Test-Case 'guard accepts declared host as SYSTEM case-insensitively' {
        Assert-AcceptanceContext 'YAN' 'S-1-5-18' 'yan'
    }
    Test-Case 'unique run preserves old evidence byte for byte' {
        $prefix=New-AcceptanceRun $root "$RunId-reserve"
        $old=[IO.File]::ReadAllBytes("$prefix.claim.json")
        Reject {New-AcceptanceRun $root "$RunId-reserve"} 'RUN_ID_REUSED'
        Equal ([Convert]::ToBase64String([IO.File]::ReadAllBytes("$prefix.claim.json"))) ([Convert]::ToBase64String($old))
        Reject {New-AcceptanceRun $root '../bad-run'} 'INVALID_RUN_ID'
    }
    Test-Case 'stale result without claim is refused' {
        $file=Join-Path $root "$RunId-stale.result.json"
        [IO.File]::WriteAllText($file,'old-pass')
        Reject {New-AcceptanceRun $root "$RunId-stale"} 'RUN_ID_REUSED'
        Equal ([IO.File]::ReadAllText($file)) 'old-pass'
    }
    Test-Case 'Running does not imply local ready' {
        $d=Get-AcceptanceDimensions '{"service":{"state":"running"},"application":{"state":"unknown"}}'
        Equal $d.scm 'running'; Equal $d.local_application 'unknown'; Equal $d.remote_control 'not_tested'
    }
    Test-Case 'ready and registered false remain independent' {
        $d=Get-AcceptanceDimensions '{"service":{"state":"running"},"application":{"state":"ready","registered":false}}'
        Equal $d.local_application 'ready'; Equal $d.registration 'disconnected'; Equal $d.remote_control 'not_tested'
    }
    Test-Case 'missing null and invalid JSON cannot become ready' {
        foreach($text in @('{}','null','not-json','{"application":{"state":"healthy"}}')){
            Equal (Get-AcceptanceDimensions $text).local_application 'unknown'
        }
    }
    Test-Case 'string registration boolean is unknown' {
        Equal (Get-AcceptanceDimensions '{"application":{"state":"ready","registered":"false"}}').registration 'unknown'
    }
    Test-Case 'last Rust parent summary supplies real test count' {
        $s=Read-AcceptanceTestSummary "test result: ok. 1 passed; 0 failed; 0 ignored;`ntest result: ok. 2 passed; 0 failed; 1 ignored;" 0
        Equal $s.passed 2; Equal $s.ignored 1
    }
    Test-Case 'zero tests missing summary and nonzero exit fail' {
        Reject {Read-AcceptanceTestSummary 'test result: ok. 0 passed; 0 failed; 1 ignored;' 0} 'ZERO_TESTS'
        Reject {Read-AcceptanceTestSummary 'no tests' 0} 'MISSING_SUMMARY'
        Reject {Read-AcceptanceTestSummary 'test result: ok. 1 passed; 0 failed; 0 ignored;' 1} 'TEST_PROCESS_FAILED'
    }
    function Event($Record,$Id,$Instance,$Code='0'){
        [pscustomobject]@{record_id=$Record;id=$Id;instance=$Instance;result=$Code;task='\RustDeskManagementAgentRecovery'}
    }
    Test-Case 'first boot instance completed is attributed by RecordId' {
        $events=@((Event 101 118 'a'),(Event 102 200 'a'),(Event 103 201 'a'))
        $r=Resolve-AcceptanceBoot $events 100 $true
        Equal $r.state 'pass'; Equal $r.instance 'a'
    }
    Test-Case '329 wins over later success or retry' {
        $events=@((Event 101 118 'a'),(Event 102 200 'a'),(Event 103 329 'a'),(Event 104 201 'a'),(Event 105 118 'b'),(Event 106 200 'b'),(Event 107 201 'b'))
        Equal (Resolve-AcceptanceBoot $events 100 $true).state 'fail'
    }
    Test-Case 'periodic recovery cannot prove boot trigger' {
        Equal (Resolve-AcceptanceBoot @((Event 101 107 'a'),(Event 102 201 'a')) 100 $true).state 'unknown'
    }
    Test-Case 'old events different instance disabled log and incomplete evidence are unknown' {
        Equal (Resolve-AcceptanceBoot @((Event 1 118 'a'),(Event 2 201 'a')) 100 $true).state 'unknown'
        Equal (Resolve-AcceptanceBoot @((Event 101 118 'a'),(Event 102 201 'b')) 100 $true).state 'unknown'
        Equal (Resolve-AcceptanceBoot @((Event 101 118 'a'),(Event 102 200 'a'),(Event 103 201 'a')) 100 $false).state 'unknown'
        Equal (Resolve-AcceptanceBoot @((Event 101 118 'a')) 100 $true).state 'unknown'
    }
    Test-Case 'boot nonzero exit fails and unrelated task cannot pass' {
        Equal (Resolve-AcceptanceBoot @((Event 101 118 'a'),(Event 102 200 'a'),(Event 103 201 'a' '7')) 100 $true).state 'fail'
        $e=Event 101 118 'a';$e.task='\unrelated'
        Equal (Resolve-AcceptanceBoot @($e) 100 $true).state 'unknown'
    }
    Test-Case 'PID reused with different creation time is not same process' {
        Equal (Test-AcceptanceProcessIdentity 2 100 2 101) $false
        Equal (Test-AcceptanceProcessIdentity 2 100 2 100) $true
        Equal (Test-AcceptanceProcessIdentity 0 0 0 0) $false
    }
    Test-Case 'monotonic timeout preserves child output and records process identity' {
        $script=Join-Path $root "$RunId-slow.ps1"
        [IO.File]::WriteAllText($script,"Write-Output 'fixture-start'; Start-Sleep -Seconds 3")
        $r=Invoke-AcceptanceProcess "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" "-NoProfile -File $script" (Join-Path $root "$RunId-slow") 150
        Equal $r.state 'unknown'; Equal $r.error 'PROCESS_TIMEOUT'
        if($r.created_filetime -le 0 -or $r.pid -le 0){throw 'missing process identity'}
        if(!(Test-Path -LiteralPath $r.stdout)){throw 'missing failure output'}
        $p=Get-Process -Id $r.pid -ErrorAction SilentlyContinue
        if($p){[void]$p.WaitForExit(10000)}
    }
    Test-Case 'process nonzero and malformed payload remain failure evidence' {
        $script=Join-Path $root "$RunId-exit.ps1"
        [IO.File]::WriteAllText($script,"Write-Output 'not-json'; exit 7")
        $r=Invoke-AcceptanceProcess "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" "-NoProfile -File $script" (Join-Path $root "$RunId-exit") 10000
        Equal $r.state 'fail'; Equal $r.exit_code 7
        Equal (Get-AcceptanceDimensions ([IO.File]::ReadAllText($r.stdout))).local_application 'unknown'
    }
} catch { $setupError=$_.Exception.Message }
$failed=@($results | Where-Object state -eq 'fail').Count
@{schema=1;run_id=$RunId;kind='driver_fixture';executed=$results.Count;passed=$results.Count-$failed;failed=$failed;setup_error=$setupError;tests=@($results);product_tests=0} |
    ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $output -Encoding UTF8
if($setupError -or $failed -gt 0 -or $results.Count -eq 0){exit 1}
