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


if __name__ == "__main__":
    unittest.main()
