import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FLUTTER_BUILD = ROOT / ".github/workflows/flutter-build.yml"


def job_block(workflow, job):
    match = re.search(rf"^  {re.escape(job)}:\n(.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)", workflow, re.M | re.S)
    if match is None:
        raise AssertionError(f"job not found: {job}")
    return match.group(1)


def step_block(job, name):
    match = re.search(
        rf"^      - name: {re.escape(name)}\n(.*?)(?=^      - |\Z)", job, re.M | re.S
    )
    if match is None:
        raise AssertionError(f"step not found: {name}")
    return match.group(1)


class WindowsMsiOnlyReleaseTest(unittest.TestCase):
    def setUp(self):
        self.job = job_block(FLUTTER_BUILD.read_text(), "build-for-windows-flutter")

    def test_windows_release_job_does_not_build_self_extracted_exe(self):
        self.assertNotIn("Build self-extracted executable", self.job)
        self.assertNotIn("pushd ./libs/portable", self.job)
        self.assertNotIn("generate.py", self.job)
        self.assertNotIn("rustdesk-portable-packer.exe", self.job)

    def test_windows_release_job_never_publishes_or_stages_exe(self):
        self.assertNotIn("SignOutput/rustdesk-*.exe", self.job)
        for name in ("Publish Release", "Stage Windows release assets"):
            step = step_block(self.job, name)
            self.assertIn("./SignOutput/rustdesk-*.msi", step)
            self.assertNotIn(".exe", step)

    def test_msi_inputs_are_still_produced(self):
        self.assertIn("--skip-portable-pack", self.job)
        msi = step_block(self.job, "Build msi")
        self.assertIn("-d ../../rustdesk", msi)
        self.assertLess(msi.index("SignOutput"), msi.index("pushd ./res/msi"))
        self.assertIn("New-Item -ItemType Directory -Force -Path ./SignOutput", msi)
        self.assertLess(self.job.index("--skip-portable-pack"), self.job.index("- name: Build msi"))

    def test_msi_is_still_signed(self):
        step = step_block(self.job, "Sign rustdesk MSI file")
        self.assertIn("sign_files ./SignOutput", step)


if __name__ == "__main__":
    unittest.main()
