import base64
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import re


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "res" / "defender-exclusions.ps1"

# Never allow a fixture to resolve the real Defender provider accidentally.
MOCK_GUARD = r'''
function Get-MpPreference { throw 'real Defender access forbidden in this test' }
function Add-MpPreference { throw 'real Defender write forbidden in this test' }
function Set-MpPreference { throw 'real Defender write forbidden in this test' }
function Remove-MpPreference { throw 'real Defender write forbidden in this test' }
'''


class DefenderInstallerTests(unittest.TestCase):
    def execute(self, case):
        source = SCRIPT.read_text(encoding="utf-8")
        harness = r'''
$ErrorActionPreference='Stop'
$script:Consent=$null
$script:Owners=@()
$script:Exclusions=@('C:\Existing Tool\existing.exe')
$script:Writes=0
$script:Reads=0
$main='D:\Custom 中文 Folder\RustDesk.exe'
$agent='C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
function Get-RdDefenderConsent { $script:Consent }
function Set-RdDefenderConsent { param($Choice); $script:Consent=$Choice }
function Get-RdDefenderOwnedPaths { @($script:Owners) }
function Set-RdDefenderOwnedPaths { param($Paths); $script:Owners=@($Paths) }
function Set-RdDefenderResult { param($Result); $script:SavedResult=$Result }
$script:CfaAllowed=@()
$script:CfaEnabled=$false
function Get-MpPreference {
    $script:Reads++
    if($Case -eq 'read-failed'){throw 'read denied'}
    [pscustomobject]@{ExclusionPath=$script:Exclusions;EnableControlledFolderAccess=$(if($script:CfaEnabled){1}else{0});ControlledFolderAccessAllowedApplications=$script:CfaAllowed}
}
function Add-MpPreference {
    param($ExclusionPath,$ControlledFolderAccessAllowedApplications,$ErrorAction)
    $script:Writes++
    if($ExclusionPath){
        if($Case -eq 'denied' -or ($Case -eq 'partial' -and $ExclusionPath -eq $agent)){throw 'policy denied'}
        if($Case -ne 'ignored'){$script:Exclusions+=@($ExclusionPath)}
    }
    if($ControlledFolderAccessAllowedApplications){
        if($Case -eq 'cfa-denied'){throw 'cfa policy denied'}
        if($Case -ne 'cfa-ignored'){$script:CfaAllowed+=@($ControlledFolderAccessAllowedApplications)}
    }
}
$choice='1'
switch($Case){
 'declined' {$choice='0'}
 'fresh-silent' {$choice='inherit'}
 'upgrade-yes' {$choice='inherit';$script:Consent='1'}
 'upgrade-no' {$choice='inherit';$script:Consent='0'}
 'already' {$script:Exclusions+=@($main,$agent,(Split-Path $main),(Split-Path $agent),'C:\ProgramData\RustDesk\Client')}
 'cfa-on' {$script:CfaEnabled=$true}
 'cfa-existing' {$script:CfaEnabled=$true;$script:CfaAllowed=@($main,$agent)}
 'cfa-denied' {$script:CfaEnabled=$true}
 'cfa-ignored' {$script:CfaEnabled=$true}
 'unavailable' { function Get-Command {param($Name,$ErrorAction); $null} }
 'consent-read-failed' {$choice='inherit';function Get-RdDefenderConsent {throw 'read denied'}}
 'owner-write-failed' {function Set-RdDefenderOwnedPaths {param($Paths);throw 'owner store denied'}}
 'readback-failed' {function Get-MpPreference {if($script:Writes -gt 0){throw 'readback denied'};[pscustomobject]@{ExclusionPath=$script:Exclusions;EnableControlledFolderAccess=0;ControlledFolderAccessAllowedApplications=@()}}}
 'bad-path' {$main='D:\Tools\RustDesk.exe:payload.exe'}
  'consent-write-failed' {function Set-RdDefenderConsent {param($Choice);throw 'save denied'}}
  'result-write-failed' {function Set-RdDefenderResult {param($Result);throw 'result denied'}}
  'invalid-saved-choice' {$choice='inherit';$script:Consent='true'}
  'missing-provider-field' {function Get-MpPreference {[pscustomobject]@{Other=@()}}}
}
$result=Invoke-RdDefenderConfiguration -Choice $choice -MainPath $main -AgentPath $agent
if($script:Exclusions -notcontains 'C:\Existing Tool\existing.exe'){throw 'pre-existing exclusion lost'}
if($result.Automated360){throw '360 falsely configured'}
switch($Case){
 'selected' {
   if($result.Status -ne 'verified' -or $script:Writes -ne 5 -or $script:Consent -ne '1'){throw 'selected failed'}
   if($script:Owners.Count -ne 5 -or @($script:CfaAllowed).Count -ne 0){throw 'ownership or cfa scope wrong'}
  $again=Invoke-RdDefenderConfiguration -Choice inherit -MainPath $main -AgentPath $agent
   if($again.Status -ne 'verified' -or $script:Writes -ne 5){throw 'idempotence failed'}
 }
 'declined' {if($script:Writes -ne 0 -or $script:Reads -ne 0 -or $script:Consent -ne '0' -or $result.Status -ne 'not_selected'){throw 'decline ignored'}}
 'fresh-silent' {if($script:Writes -ne 0 -or $result.Status -ne 'not_selected'){throw 'silent unexpectedly allowed'}}
 'upgrade-yes' {if($script:Writes -ne 5 -or $result.Status -ne 'verified'){throw 'upgrade lost selection'}}
 'upgrade-no' {if($script:Writes -ne 0 -or $result.Status -ne 'not_selected'){throw 'upgrade ignored rejection'}}
 'already' {if($script:Writes -ne 0 -or $script:Owners.Count -ne 0 -or $result.Status -ne 'verified'){throw 'claimed user exclusions'}}
 'unavailable' {if($script:Writes -ne 0 -or $result.Status -ne 'unavailable'){throw 'unavailable misreported'}}
 'read-failed' {if($script:Writes -ne 0 -or $result.Status -ne 'read_failed'){throw 'failed read wrote preferences'}}
 'denied' {if($result.Status -ne 'failed' -or $script:Owners.Count -ne 0){throw 'denial misreported'}}
 'partial' {if($result.Status -ne 'partial' -or $script:Owners.Count -ne 4){throw 'partial lost'}}
 'ignored' {if($result.Status -ne 'unverified' -or $script:Owners.Count -ne 0){throw 'unverified write claimed'}}
 'consent-read-failed' {if($script:Writes -ne 0 -or $result.Status -ne 'consent_failed'){throw 'consent fail-open'}}
 'owner-write-failed' {if($result.Status -ne 'verified' -or $result.OwnershipError -notmatch 'owner store denied'){throw 'ownership failure hidden'}}
 'readback-failed' {if($result.Status -ne 'unverified' -or $result.Items[0].Status -ne 'unverified'){throw 'readback unknown misreported'}}
 'bad-path' {if($script:Writes -ne 0 -or $result.Status -ne 'consent_failed'){throw 'alternate stream accepted'}}
  'consent-write-failed' {if($script:Writes -ne 0 -or $result.Status -ne 'consent_failed'){throw 'consent save fail-open'}}
  'result-write-failed' {if($result.Status -ne 'verified' -or $result.PersistenceError -notmatch 'result denied'){throw 'result persistence failure hidden'}}
  'invalid-saved-choice' {if($script:Writes -ne 0 -or $script:Reads -ne 0 -or $result.Status -ne 'consent_failed'){throw 'invalid saved consent accepted'}}
  'missing-provider-field' {if($script:Writes -ne 0 -or $result.Status -ne 'read_failed'){throw 'missing provider field accepted'}}
  'cfa-on' {if($result.Status -ne 'verified' -or @($script:CfaAllowed).Count -ne 2 -or $script:CfaAllowed -notcontains $main -or $script:CfaAllowed -notcontains $agent){throw 'cfa allow missing'}}
  'cfa-existing' {if($result.Status -ne 'verified' -or $script:Writes -ne 5){throw 'existing cfa allow rewritten'}}
  'cfa-denied' {if($result.Status -ne 'partial' -or @($result.Items|Where-Object Kind -eq 'ControlledFolderAccessAllowedApplication'|Where-Object Status -eq 'failed').Count -ne 2){throw 'cfa denial hidden'}}
  'cfa-ignored' {if($result.Status -ne 'partial' -or @($result.Items|Where-Object Kind -eq 'ControlledFolderAccessAllowedApplication'|Where-Object Status -eq 'unverified').Count -ne 2){throw 'cfa readback skipped'}}
}
if($Case -ne 'result-write-failed' -and $script:SavedResult.Status -ne $result.Status){throw 'final result not persisted'}
if($result.Items.Count -gt 7){throw 'scope broadened'}
'PASS '+$Case
'''
        command = MOCK_GUARD + source + "\n$Case='" + case + "'\n" + harness
        with tempfile.TemporaryDirectory(prefix="rustdesk-defender-test-") as temp:
            script = Path(temp) / "case.ps1"
            script.write_text(command, encoding="utf-8-sig")
            result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-File", str(script)],
                                    capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(("PASS " + case).encode(), result.stdout)

    @unittest.skipUnless(os.name == "nt", "Windows PowerShell mock harness requires Windows")
    def test_consent_and_defender_failures(self):
        for case in ["selected", "declined", "fresh-silent", "upgrade-yes", "upgrade-no", "already",
                     "unavailable", "read-failed", "denied", "partial", "ignored", "consent-read-failed",
                       "owner-write-failed", "readback-failed", "bad-path", "consent-write-failed", "result-write-failed", "invalid-saved-choice", "missing-provider-field",
                       "cfa-on", "cfa-existing", "cfa-denied", "cfa-ignored"]:
            with self.subTest(case=case):
                self.execute(case)

    def test_exe_msi_and_silent_paths_are_explicit_and_before_file_copy(self):
        windows=(ROOT/'src/platform/windows.rs').read_text(encoding='utf-8')
        install=windows.split('pub fn install_me(',1)[1].split('pub fn run_after_install(',1)[0]
        self.assertLess(install.index('{defender_configuration}'),install.index('{copy_exe}'))
        update=windows.split('pub fn update_me(',1)[1].split('fn normalize_msi_product_code',1)[0]
        self.assertLess(update.index('{defender_configuration}'),update.index('{update_transaction}'))
        core=(ROOT/'src/core_main.rs').read_text(encoding='utf-8')
        self.assertIn('defender_install_option(&args)',core)
        wxs=(ROOT/'res/msi/Package/Fragments/Defender.wxs').read_text(encoding='utf-8')
        self.assertIn('Before="InstallFiles"',wxs)
        self.assertIn('Execute="deferred"',wxs)
        self.assertIn('Impersonate="no"',wxs)
        self.assertIn('Return="ignore"',wxs)
        # 交互安装默认勾选、静默安装（后台更新、Agent 新装与修复）默认配置 Defender。
        self.assertIn('<Property Id="DEFENDER_EXCLUSIONS" Value="1" Secure="yes" />',wxs)
        self.assertIn('[INSTALLFOLDER_INNER]',wxs)
        self.assertIn('Bitness="always64"',wxs)
        self.assertIn('DEFENDER_SAVED_CONSENT=&quot;1&quot;',wxs)
        self.assertNotIn('RUSTDESK_AGENT_RECOVERY=&quot;1&quot;',wxs)
        self.assertIn('<Custom Action="ConfigureDefender" Before="InstallFiles" Condition="NOT REMOVE~=&quot;ALL&quot;" />',wxs)
        self.assertNotIn('Remove-MpPreference',wxs)
        package=(ROOT/'res/msi/Package/Package.wxs').read_text(encoding='utf-8')
        self.assertIn('<PropertyRef Id="DEFENDER_EXCLUSIONS" />',package)

    def test_script_never_removes_exclusions_or_disables_security(self):
        source=SCRIPT.read_text(encoding='utf-8')
        for forbidden in ['Remove-MpPreference','Set-MpPreference','DisableRealtimeMonitoring','DisableTamperProtection']:
            self.assertNotIn(forbidden,source)
        self.assertIn('Add-MpPreference -ExclusionPath',source)
        self.assertIn('-ControlledFolderAccessAllowedApplications',source)
        self.assertIn("'RustDesk Management Agent Service'",source)
        self.assertNotIn('ControlledFolderAccessProtectedFolders',source)
        self.assertNotIn('Set-MpPreference',source)
        self.assertNotIn('-EnableControlledFolderAccess',source)
        self.assertIn("OwnedPaths",source)

    def test_msi_script_embedding_is_single_source_and_roundtrips_unicode(self):
        source=SCRIPT.read_text(encoding='utf-8')
        with tempfile.TemporaryDirectory(prefix='rustdesk-defender-embed-') as temp:
            output=Path(temp)/'DefenderScript.inc'
            completed=subprocess.run(['python',str(ROOT/'res/msi/embed-defender-script.py'),
                '--source',str(SCRIPT),'--output',str(output)],capture_output=True,timeout=10)
            self.assertEqual(completed.returncode,0,completed.stderr)
            generated=output.read_text(encoding='utf-8')
            literals = re.findall(r'LR"RDDEFENDER\((.*?)\)RDDEFENDER"', generated, re.DOTALL)
            self.assertGreater(len(literals), 1)
            self.assertEqual(
                ''.join(literals),
                source,
            )
            # UTF-16/base64 is the actual CA transport; assert current payload fits
            # CreateProcess's limit and roundtrips the entire shared script.
            encoded=base64.b64encode(source.encode('utf-16le')).decode()
            self.assertLess(len(encoded)+2048,32760)
            self.assertEqual(base64.b64decode(encoded).decode('utf-16le'),source)

    def test_msi_xml_and_project_are_parseable_and_wire_exact_final_paths(self):
        import xml.etree.ElementTree as ET
        for relative in ['res/msi/Package/Fragments/Defender.wxs',
            'res/msi/Package/UI/MyInstallDirDlg.wxs','res/msi/CustomActions/CustomActions.vcxproj']:
            ET.parse(ROOT/relative)
        source=(ROOT/'res/msi/Package/Fragments/Defender.wxs').read_text(encoding='utf-8')
        self.assertIn('[ManagementAgent.Folder]rustdesk-management-agent.exe',source)
        self.assertIn('NOT REMOVE~=&quot;ALL&quot;',source)
        cpp=(ROOT/'res/msi/CustomActions/Defender.cpp').read_text(encoding='utf-8')
        self.assertIn('CreateProcessW',cpp)
        self.assertIn('WcaFinalize(ERROR_SUCCESS)',cpp)
        self.assertNotIn('Remove-MpPreference',cpp)

    def test_firewall_rules_keep_agent_name_and_remove_both_directions(self):
        windows=(ROOT/'src/platform/windows.rs').read_text(encoding='utf-8')
        cpp=(ROOT/'res/msi/CustomActions/CustomActions.cpp').read_text(encoding='utf-8')
        wxs=(ROOT/'res/msi/Package/Components/RustDesk.wxs').read_text(encoding='utf-8')
        self.assertEqual(windows.count('name=\\"RustDesk Management Agent Service\\"'), 3)
        self.assertIn('delete rule name=\\"RustDesk Management Agent Service\\"',windows)
        self.assertIn('Value="0[INSTALLFOLDER_INNER]$(var.Product).exe|[ManagementAgent.Folder]rustdesk-management-agent.exe"',wxs)
        self.assertIn('AddFirewallRule(exeFile[0] == L\'1\', agentRuleName, extra)',cpp)

    @unittest.skipUnless(os.name == "nt", "Windows PowerShell mock harness requires Windows")
    def test_invalid_scope_does_not_call_defender(self):
        source=SCRIPT.read_text(encoding='utf-8')
        fixture=r'''
$ErrorActionPreference='Stop'
$script:ConsentWrites=0
function Set-RdDefenderResult {param($Result)}
function Set-RdDefenderConsent {$script:ConsentWrites++;throw 'must reject before saving consent'}
foreach($path in @('C:\','C:\Tools\*\RustDesk.exe','\\server\share\RustDesk.exe',
 'C:\Tools\..\RustDesk.exe','C:RustDesk.exe','C:\Tools\x.dll','C:\Tools\x.exe:stream.exe')){
 $r=Invoke-RdDefenderConfiguration -Choice 1 -MainPath $path -AgentPath 'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
 if($r.Status -ne 'consent_failed'){throw ('scope accepted: '+$path)}
}
if($script:ConsentWrites -ne 0){throw 'invalid scope reached consent write'}
'PASS scope'
'''
        with tempfile.TemporaryDirectory(prefix='rustdesk-defender-scope-') as temp:
            script=Path(temp)/'scope.ps1'
            script.write_text(MOCK_GUARD+source+fixture, encoding='utf-8-sig')
            result=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-File',str(script)],capture_output=True,timeout=30)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertIn(b'PASS scope',result.stdout)


if __name__ == "__main__":
    unittest.main()
