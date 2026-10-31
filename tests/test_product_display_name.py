import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
LEGACY_NAME = "RustDesk Yan"
PRODUCT_FILES = (
    "Cargo.toml",
    "build.rs",
    "build.py",
    ".github/workflows/flutter-build.yml",
    "flutter/macos/Runner/Configs/AppInfo.xcconfig",
    "flutter/windows/runner/Runner.rc",
    "flutter/windows/runner/main.cpp",
    "flutter/android/app/src/main/res/values/strings.xml",
    "res/rustdesk.desktop",
    "res/rustdesk-link.desktop",
    "res/rustdesk.service",
    "macos-screen-permission.sh",
    "macos-screen-permission-uninstall.sh",
)


class ProductDisplayNameTest(unittest.TestCase):
    def test_product_files_use_the_plain_rustdesk_name(self) -> None:
        for relative in PRODUCT_FILES:
            text = (REPO_ROOT / relative).read_text(encoding="utf-8")
            lines = [line for line in text.splitlines() if LEGACY_NAME in line]
            self.assertEqual(lines, [], f"{relative} still names {LEGACY_NAME}")

    def test_windows_version_resources_name_rustdesk(self) -> None:
        cargo = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
        runner = (REPO_ROOT / "flutter/windows/runner/Runner.rc").read_text(encoding="utf-8")
        agent = (REPO_ROOT / "build.rs").read_text(encoding="utf-8")

        self.assertIn('ProductName = "RustDesk"', cargo)
        self.assertIn('VALUE "ProductName", "RustDesk" "\\0"', runner)
        self.assertIn('VALUE "ProductName", "RustDesk\\0"', agent)


if __name__ == "__main__":
    unittest.main()
