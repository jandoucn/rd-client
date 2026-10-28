"""MSI 安装时关闭 UAC：以 SYSTEM 在提交阶段写四个策略值，失败不阻断安装，不请求重启，纯卸载不执行。"""

import re
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MSI = ROOT / "res" / "msi"
NS = {"w": "http://wixtoolset.org/schemas/v4/wxs"}
UAC_VALUES = (
    "EnableLUA",
    "ConsentPromptBehaviorAdmin",
    "PromptOnSecureDesktop",
    "FilterAdministratorToken",
)


def custom_action_body(name):
    source = (MSI / "CustomActions" / "CustomActions.cpp").read_text(encoding="utf-8")
    start = source.index(f"UINT __stdcall {name}(")
    end = source.index("\nUINT __stdcall ", start + 1)
    return source[start:end]


def schedule(action):
    root = ET.parse(MSI / "Package" / "Components" / "RustDesk.wxs").getroot()
    return root.find(f".//w:InstallExecuteSequence/w:Custom[@Action='{action}']", NS)


class DisableUacCustomAction(unittest.TestCase):
    def test_entry_point_is_exported(self):
        exports = (MSI / "CustomActions" / "CustomActions.def").read_text(encoding="utf-8").split()
        self.assertIn("DisableUacPolicy", exports)

    def test_runs_as_system_in_deferred_phase_and_never_fails_the_install(self):
        root = ET.parse(MSI / "Package" / "Fragments" / "CustomActions.wxs").getroot()
        action = root.find(".//w:CustomAction[@Id='DisableUacPolicy']", NS)
        self.assertIsNotNone(action)
        self.assertEqual(action.get("DllEntry"), "DisableUacPolicy")
        self.assertEqual(action.get("Execute"), "deferred")
        self.assertEqual(action.get("Impersonate"), "no")
        self.assertEqual(action.get("Return"), "ignore")

    def test_scheduled_like_sas_so_install_and_upgrade_write_but_pure_uninstall_does_not(self):
        uac = schedule("DisableUacPolicy")
        sas = schedule("AddRegSoftwareSASGeneration")
        self.assertIsNotNone(uac)
        self.assertEqual(uac.get("Before"), "InstallFinalize")
        self.assertEqual(uac.get("Condition"), sas.get("Condition"))
        self.assertIn("NOT (Installed AND REMOVE AND NOT UPGRADINGPRODUCTCODE)", uac.get("Condition"))

    def test_writes_all_four_policy_values_as_zero_dword(self):
        body = custom_action_body("DisableUacPolicy")
        self.assertIn(r"Policies\\System", body)
        for name in UAC_VALUES:
            self.assertIn(f'L"{name}"', body)
        self.assertIn("REG_DWORD", body)
        self.assertRegex(body, r"DWORD\s+zero\s*=\s*0\s*;")

    def test_failure_is_logged_and_the_action_still_succeeds(self):
        body = custom_action_body("DisableUacPolicy")
        self.assertIn("WcaLog", body)
        self.assertRegex(body, r"LExit:(.|\n)*er\s*=\s*ERROR_SUCCESS\s*;(.|\n)*WcaFinalize\(er\)")

    def test_never_requests_or_performs_a_reboot(self):
        body = custom_action_body("DisableUacPolicy")
        for forbidden in ("MsiSetMode", "MSIRUNMODE_REBOOT", "InitiateSystemShutdown", "ExitWindowsEx", "shutdown"):
            self.assertNotIn(forbidden, body)
        sequence = (MSI / "Package" / "Components" / "RustDesk.wxs").read_text(encoding="utf-8")
        self.assertIsNone(re.search(r"<ScheduleReboot\b", sequence))

    def test_uninstall_has_no_action_that_restores_uac(self):
        source = (MSI / "Package").rglob("*.wxs")
        for path in source:
            text = path.read_text(encoding="utf-8")
            self.assertNotRegex(text, r"(?i)(Restore|Enable)Uac", str(path))


if __name__ == "__main__":
    unittest.main()
