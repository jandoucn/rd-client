import unittest
from pathlib import Path
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
NS = {"w": "http://wixtoolset.org/schemas/v4/wxs"}


class StoppedServicePreservationTests(unittest.TestCase):
    def test_maintenance_does_not_delete_a_service_whose_restart_is_suppressed(self):
        tree = ET.parse(ROOT / "res/msi/Package/Components/RustDesk.wxs")
        actions = {
            node.attrib["Action"]: node.attrib.get("Condition", "")
            for node in tree.findall(".//w:InstallExecuteSequence/w:Custom", NS)
        }
        for action in ("TryStopDeleteService", "TryStopDeleteService.SetParam"):
            with self.subTest(action=action):
                self.assertEqual(
                    actions[action],
                    'REMOVE="ALL" OR NOT STOP_SERVICE="\'Y\'"',
                    "维护时抑制重启必须保留现有停止服务；明确卸载仍可删除",
                )
        self.assertIn("NOT STOP_SERVICE=", actions["CreateStartService"])


if __name__ == "__main__":
    unittest.main()
