import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MSI_ROOT = ROOT / "res" / "msi"


class ManagementAgentMsiContractTests(unittest.TestCase):
    def read(self, relative):
        return (ROOT / relative).read_text(encoding="utf-8")

    def test_arp_removal_is_disabled_without_hiding_registration(self):
        source = self.read("res/msi/Package/Fragments/AddRemoveProperties.wxs")
        self.assertIn('<Property Id="ARPNOREMOVE" Value="1" />', source)
        self.assertNotIn("ARPSYSTEMCOMPONENT", source)

    def test_start_menu_uninstall_shortcut_is_not_installed(self):
        source = self.read("res/msi/Package/Components/RustDesk.wxs")
        self.assertNotIn("App.StartMenu.ShortcutUninstall", source)

    def test_management_agent_is_permanent_programdata_component(self):
        source = self.read("res/msi/Package/Components/ManagementAgent.wxs")
        self.assertIn("<?include ../Includes.wxi?>", source)
        self.assertIn('DirectoryRef Id="App.Data.Folder"', source)
        self.assertIn('Directory Id="ManagementAgent.Folder" Name="ManagementAgent"', source)
        self.assertIn('Permanent="yes"', source)
        self.assertIn('Name="RustDeskManagementAgent"', source)
        self.assertIn('Start="auto"', source)
        self.assertIn('Stop="install"', source)
        self.assertIn('Remove="never"', source)
        self.assertIn('Account="LocalSystem"', source)
        self.assertIn('Hidden="yes"', source)
        self.assertIn('System="yes"', source)
        self.assertIn('--edition $(var.Edition)', source)
        self.assertIn('--normal-install-path', source)
        self.assertIn('"/qn".into()', self.read("src/management_agent.rs"))

    def test_management_agent_component_is_in_main_feature(self):
        source = self.read("res/msi/Package/Package.wxs")
        self.assertIn('<ComponentRef Id="ManagementAgent.Component" />', source)

    def test_normal_client_and_agent_use_separate_roots(self):
        folders = self.read("res/msi/Package/Components/Folders.wxs")
        agent = self.read("res/msi/Package/Components/ManagementAgent.wxs")
        self.assertIn('<StandardDirectory Id="ProgramFiles6432Folder">', folders)
        self.assertIn('<StandardDirectory Id="CommonAppDataFolder">', folders)
        self.assertIn('<DirectoryRef Id="App.Data.Folder">', agent)

    def test_preprocess_and_workflow_pass_edition(self):
        preprocess = self.read("res/msi/preprocess.py")
        workflow = self.read(".github/workflows/flutter-build.yml")
        self.assertIn('"--edition"', preprocess)
        self.assertIn('Edition="{edition}"', preprocess)
        self.assertIn("cargo build --locked --bin rustdesk-management-agent --release", workflow)
        self.assertIn('python preprocess.py --arp --edition "${{ env.EDITION }}"', workflow)

    def test_recovery_install_disables_all_user_entrypoints(self):
        agent = self.read("src/management_agent.rs")
        for value in (
            'STARTMENUSHORTCUTS=0',
            'DESKTOPSHORTCUTS=0',
            'STARTUPSHORTCUTS=0',
            'LAUNCH_TRAY_APP=N',
        ):
            self.assertIn(value, agent)

    def test_exe_recovery_extracts_portable_payload_before_service_registration(self):
        agent = self.read("src/management_agent.rs")
        self.assertIn("--management-agent-extract-path=", self.read("libs/portable/src/main.rs"))
        self.assertIn("portable EXE extraction failed", agent)
        self.assertIn("portable EXE did not extract RustDesk.exe", agent)
        self.assertIn("$extract=Start-Process -FilePath $src -ArgumentList", agent)
        self.assertIn("-Wait -PassThru -WindowStyle Hidden", agent)
        self.assertIn("throw \\\"portable EXE extraction failed:", agent)
        self.assertIn("$($extract.ExitCode)\\\"", agent)
        self.assertNotIn("& $src ('--management-agent-extract-path=' + $tmp)", agent)
        self.assertIn("Move-Item -LiteralPath $tmp -Destination $dir", agent)
        self.assertNotIn("Copy-Item -LiteralPath $src -Destination $dst -Force", agent)
        self.assertIn("exit $(if ($recovery_ok) {{0}} else {{1}})", agent)
        self.assertIn("fn windows_powershell_exe", agent)
        self.assertIn("WindowsPowerShell\\v1.0\\powershell.exe", agent)
        self.assertIn("fn wait_for_exe_recovery_result", agent)
        self.assertNotIn("EXE service installer exited with {status}", agent)
        exe_installer = agent.split("fn install_exe_package(", 1)[1].split(
            "fn windows_powershell_exe(", 1
        )[0]
        self.assertNotIn("Command::new(\"powershell.exe\")", exe_installer)
        self.assertIn("windows_powershell_exe()", exe_installer)
        self.assertIn("wait_for_exe_recovery_result(", exe_installer)
        self.assertIn("$result={result}", agent)
        self.assertIn("result = powershell_literal(&result_path)", agent)
        self.assertNotIn("rustdesk-exe-recovery-'+$PID", agent)

    def test_recovery_deletes_verified_download_before_and_after_install(self):
        agent = self.read("src/management_agent.rs")
        exe = agent.split("fn check_now_with_package(", 1)[1].split("fn run_service(", 1)[0]
        msi = agent.split("pub fn run_recovery_helper(", 1)[1].split("pub fn recovery_exe_args(", 1)[0]
        self.assertIn("fn replace_verified_download", agent)
        self.assertIn("remove_download_file(cache);", agent)
        self.assertIn("remove_download_file(&temporary);", agent)
        self.assertIn("remove_download_file(&cache);", exe)
        self.assertLess(exe.index("remove_download_file(&cache);"), exe.index("installed?"))
        self.assertIn("})();\n    remove_download_file(&cache);", msi)
        self.assertLess(msi.index("if !status.success()"), msi.index("remove_download_file(&cache);"))

    def test_exe_install_registers_management_agent_for_compiled_edition(self):
        source = self.read("src/platform/windows.rs")
        self.assertIn("rustdesk-management-agent.exe", source)
        self.assertIn("RustDeskManagementAgent", source)
        self.assertIn("--package-kind exe", source)
        self.assertIn('option_env!("RUSTDESK_SOS")', source)
        self.assertIn("WaitForStatus('Stopped'", source)
        self.assertIn("sc config RustDeskManagementAgent", source)
        self.assertIn("sc start RustDeskManagementAgent", source)

    def test_windows_agent_entry_passes_os_arguments_to_recovery_helper(self):
        source = self.read("src/management_agent_bin.rs")
        self.assertIn("std::env::args_os().skip(1)", source)
        self.assertNotIn("std::env::args().skip(1)", source)
        self.assertIn("run_recovery_helper(&arguments)", source)

    def test_msi_recovery_uses_detached_helper_and_restores_agent(self):
        source = self.read("src/management_agent.rs")
        self.assertIn("spawn_msi_recovery_helper", source)
        self.assertIn("--management-agent-recovery-helper", source)
        self.assertIn("failed to stop management agent", source)
        self.assertIn("failed to restart management agent", source)
        self.assertIn("msiexec exited with exit code:", source)
        self.assertIn('const RECOVERY_DEFERRED: &str', source)
        self.assertIn('if error == RECOVERY_DEFERRED', source)


if __name__ == "__main__":
    unittest.main()
