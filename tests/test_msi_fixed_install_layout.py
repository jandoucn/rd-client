"""安装目录锁定在系统盘的 Program Files，Defender 默认执行且不在界面显示。

管理代理、救援包和本地修复都假定客户端在系统 Program Files 下的 RustDesk 目录；
允许用户改目录会让这些兜底全部失效，所以安装包不再提供任何改目录的入口。
"""

import re
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "res" / "msi" / "Package"
NS = {"w": "http://wixtoolset.org/schemas/v4/wxs"}


def parse(relative):
    return ET.parse(PACKAGE / relative).getroot()


def text(relative):
    return (PACKAGE / relative).read_text(encoding="utf-8")


class InstallDirectoryIsLocked(unittest.TestCase):
    def test_dialog_has_no_way_to_change_the_folder(self):
        dialog = parse("UI/MyInstallDirDlg.wxs").find(".//w:Dialog[@Id='MyInstallDirDlg']", NS)
        controls = {c.get("Id"): c for c in dialog.findall("w:Control", NS)}
        self.assertNotIn("ChangeFolder", controls)
        self.assertFalse(
            [c for c in controls.values() if c.get("Type") == "PathEdit"],
            "folder must be displayed read-only, not editable",
        )
        folder = controls["Folder"]
        self.assertEqual(folder.get("Type"), "Text")
        self.assertEqual(folder.get("Text"), "[INSTALLFOLDER_INNER]")

    def test_dialog_text_does_not_offer_a_folder_change(self):
        dialog = parse("UI/MyInstallDirDlg.wxs").find(".//w:Dialog[@Id='MyInstallDirDlg']", NS)
        description = {c.get("Id"): c for c in dialog.findall("w:Control", NS)}["Description"]
        match = re.fullmatch(r"!\(loc\.(\w+)\)", description.get("Text"))
        self.assertIsNotNone(match, "description must come from our own localization file")
        strings = {
            s.get("Id"): s.get("Value")
            for s in parse("Language/Package.en-us.wxl").iter("{http://wixtoolset.org/schemas/v4/wxl}String")
        }
        self.assertIn(match.group(1), strings, "WixUI's stock text mentions a Change button")
        self.assertNotIn("change", strings[match.group(1)].lower())

    def test_dialog_publishes_no_folder_change_events(self):
        source = text("UI/MyInstallDlg.wxs")
        self.assertNotIn('Control="ChangeFolder"', source)
        self.assertNotIn('Property="INSTALLFOLDER_INNER"', source)
        self.assertNotIn('Event="SetTargetPath"', source)

    def test_registry_and_command_line_folder_are_ignored(self):
        source = text("Components/Folders.wxs")
        self.assertNotIn("InstallFolderSearch", source)
        self.assertNotIn("SavedInstallFolderCmdLineValue", source)
        self.assertNotIn("RegistrySearch", source)
        # INSTALLFOLDER 只用来识别旧式恢复安装，任何时候都不会直接成为安装目录。
        for action in parse("Components/Folders.wxs").iter("{%s}SetProperty" % NS["w"]):
            self.assertNotIn("[INSTALLFOLDER]", action.get("Value"))

    def folder_actions(self):
        root = parse("Components/Folders.wxs")
        return {
            s.get("Action"): s
            for s in root.iter("{%s}SetProperty" % NS["w"])
            if s.get("Id") == "INSTALLFOLDER_INNER"
        }

    def test_install_folder_is_forced_to_system_program_files_before_costing(self):
        actions = self.folder_actions()
        self.assertEqual(set(actions), {"LockInstallFolder", "LockRecoveryInstallFolder"})
        action = actions["LockInstallFolder"]
        # ProgramFiles64Folder 由 Windows Installer 在启动时按系统盘解析，系统盘是 D 盘时就在 D 盘。
        self.assertEqual(action.get("Value"), "[ProgramFiles64Folder]$(var.Product)\\")
        self.assertEqual(action.get("Before"), "CostFinalize")
        self.assertEqual(action.get("Sequence"), "both")
        self.assertIsNone(action.get("Condition"), "the folder must be forced unconditionally")
        self.assertNotIn("C:\\", text("Components/Folders.wxs"))

    def test_agent_recovery_install_goes_to_the_fixed_fallback_folder(self):
        # 没有签名救援包时，管理代理用 RUSTDESK_AGENT_RECOVERY=1 把客户端装进保底目录；
        # 这条恢复链要保留，但目录同样由系统解析，命令行传入的路径一律不用。
        root = parse("Components/Folders.wxs")
        computed = {
            s.get("Id"): s
            for s in root.iter("{%s}SetProperty" % NS["w"])
            if s.get("Id") in ("RdFallbackFolder", "RdFallbackFolderSlash")
        }
        fallback = "[CommonAppDataFolder]$(var.Product)\\Client\\$(var.Edition)\\$(var.Product)"
        self.assertEqual(computed["RdFallbackFolder"].get("Value"), fallback)
        self.assertEqual(computed["RdFallbackFolderSlash"].get("Value"), "[RdFallbackFolder]\\")
        for item in computed.values():
            self.assertEqual(item.get("Sequence"), "both")
        # 顺序必须是一条明确的链：保底目录 → 带斜杠版本 → 锁定 Program Files → 按条件改成保底目录。
        self.assertEqual(computed["RdFallbackFolder"].get("Before"), "SetRdFallbackFolderSlash")
        self.assertEqual(computed["RdFallbackFolderSlash"].get("Before"), "LockInstallFolder")
        action = self.folder_actions()["LockRecoveryInstallFolder"]
        self.assertEqual(action.get("Value"), "[RdFallbackFolderSlash]")
        self.assertEqual(action.get("After"), "LockInstallFolder")
        self.assertEqual(action.get("Sequence"), "both")
        # RUSTDESK_AGENT_RECOVERY=1 也出现在代理发起的正常升级与本地修复里，单凭它会把
        # 正常升级装进保底目录；必须同时看到传入的目录正是保底目录才算旧式恢复安装。
        self.assertEqual(
            action.get("Condition"),
            'RUSTDESK_AGENT_RECOVERY="1" AND (INSTALLFOLDER~=RdFallbackFolder OR INSTALLFOLDER~=RdFallbackFolderSlash)',
        )

    def test_agent_upgrade_and_repair_never_match_the_fallback_condition(self):
        installer = (ROOT / "src" / "management_agent" / "installer_process.rs").read_text(encoding="utf-8")
        body = installer.split("fn msi_arguments", 1)[1].split("\nfn ", 1)[0]
        self.assertIn("RUSTDESK_AGENT_RECOVERY=1", body)
        self.assertNotIn("INSTALLFOLDER", body)
        agent = (ROOT / "src" / "management_agent.rs").read_text(encoding="utf-8")
        recovery = agent.split("pub fn recovery_msi_args", 1)[1].split("\n}\n", 1)[0]
        self.assertIn('format!("INSTALLFOLDER={}", paths.client.display())', recovery)
        self.assertIn("RUSTDESK_AGENT_RECOVERY=1", recovery)

    def test_install_folder_stays_under_program_files_directory_tree(self):
        root = parse("Components/Folders.wxs")
        standard = root.find(".//w:StandardDirectory[@Id='ProgramFiles6432Folder']", NS)
        self.assertIsNotNone(standard.find("w:Directory[@Id='INSTALLFOLDER_INNER']", NS))

    def test_management_agent_receives_the_same_locked_folder(self):
        source = text("Components/ManagementAgent.wxs")
        self.assertIn("--normal-install-path &quot;[ProgramFiles6432Folder]RustDesk&quot;", source)


class DefenderIsSilentlyDefault(unittest.TestCase):
    def test_dialog_shows_no_defender_option(self):
        source = text("UI/MyInstallDirDlg.wxs")
        self.assertNotIn("DEFENDER_CHECKBOX", source)
        self.assertNotIn("DefenderFileExclusions", source)

    def test_defender_runs_by_default_and_ui_cannot_turn_it_off(self):
        source = text("Fragments/Defender.wxs")
        self.assertIn('<Property Id="DEFENDER_EXCLUSIONS" Value="1" Secure="yes" />', source)
        self.assertNotIn("DEFENDER_CHECKBOX", source)
        self.assertNotIn("<Publish", source)
        # 命令行显式传 DEFENDER_EXCLUSIONS=0 仍会原样交给脚本，由脚本跳过配置。
        self.assertIn('Value="[DEFENDER_EXCLUSIONS]|[INSTALLFOLDER_INNER]', source)

    def test_defender_action_still_runs_before_files_on_install_and_upgrade(self):
        source = text("Fragments/Defender.wxs")
        self.assertIn(
            '<Custom Action="ConfigureDefender" Before="InstallFiles" Condition="NOT REMOVE~=&quot;ALL&quot;" />',
            source,
        )


class ShortcutsDefaultOn(unittest.TestCase):
    def test_shortcut_checkboxes_are_shown_and_default_checked(self):
        dialog = text("UI/MyInstallDirDlg.wxs")
        self.assertIn('Property="STARTMENUSHORTCUTS" CheckBoxValue="1"', dialog)
        self.assertIn('Property="DESKTOPSHORTCUTS" CheckBoxValue="1"', dialog)
        props = text("Fragments/ShortcutProperties.wxs")
        self.assertIn('<Property Id="STARTMENUSHORTCUTS" Value="1" Secure="yes">', props)
        self.assertIn('<Property Id="DESKTOPSHORTCUTS" Value="1" Secure="yes">', props)

    def test_upgrade_keeps_a_previous_shortcut_choice(self):
        props = text("Fragments/ShortcutProperties.wxs")
        self.assertIn('Id="CreateStartMenuShortcutsSearch"', props)
        self.assertIn('Id="CreateDesktopShortcutsSearch"', props)


class CustomActionsFollowTheSystemDrive(unittest.TestCase):
    def test_custom_actions_do_not_hardcode_the_system_drive(self):
        for name in ["Bootstrap.cpp", "Admission.cpp"]:
            source = (ROOT / "res" / "msi" / "CustomActions" / name).read_text(encoding="utf-8")
            hits = [
                line.strip()
                for line in source.splitlines()
                if re.search(r"C:\\\\|C:\\(?!\\)", line) and not line.strip().startswith("//")
            ]
            self.assertEqual(hits, [], name)


if __name__ == "__main__":
    unittest.main()
