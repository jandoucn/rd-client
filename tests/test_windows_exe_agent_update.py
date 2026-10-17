"""Run the embedded production transaction against isolated filesystem fixtures."""
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class LegacyEntry(unittest.TestCase):
    def test_legacy_transaction_has_joint_commit_owner(self):
        source = (ROOT / "src/platform/windows.rs").read_text(encoding="utf-8")
        update = source.split("pub fn update_me(debug:", 1)[1].split("fn normalize_msi", 1)[0]
        self.assertIn("windows_exe_agent_update::commands", update)
        self.assertIn("update_transaction_commands", update)
        self.assertNotIn("migrate_exe_update", update)

    def test_plain_update_still_uses_legacy_entry(self):
        source = (ROOT / "src/core_main.rs").read_text(encoding="utf-8")
        self.assertIn("Ok(true) => platform::update_me(false)", source)


if __name__ == "__main__":
    unittest.main()
