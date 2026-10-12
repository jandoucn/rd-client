import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("msi_preprocess", ROOT / "res/msi/preprocess.py")
PREPROCESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPROCESS)


class BootstrapPayloadTests(unittest.TestCase):
    def test_payload_digest_and_edition_come_from_the_packaged_agent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "payload"
            payload.mkdir()
            agent = payload / "rustdesk-management-agent.exe"
            agent.write_bytes(b"packaged agent")
            for edition in ("standard", "sos"):
                output = root / edition
                output.mkdir()
                path = PREPROCESS.generate_bootstrap_manifest(payload, edition, output)
                manifest = json.loads(path.read_text(encoding="utf-8"))
                self.assertEqual(manifest, {
                    "schema": 1,
                    "agent_sha256": hashlib.sha256(agent.read_bytes()).hexdigest(),
                    "edition": edition,
                    "package_kind": "msi",
                    "normal_install_path": r"C:\Program Files\RustDesk",
                })
                self.assertEqual(agent.read_bytes(), b"packaged agent")

    def test_missing_empty_or_invalid_input_preserves_existing_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root / "payload"
            payload.mkdir()
            output = root / "output"
            output.mkdir()
            path = output / "bootstrap.json"
            path.write_bytes(b"existing manifest")
            for contents, edition in ((None, "standard"), (b"", "standard"), (b"agent", "foreign")):
                agent = payload / "rustdesk-management-agent.exe"
                if contents is not None:
                    agent.write_bytes(contents)
                with self.subTest(contents=contents, edition=edition):
                    with self.assertRaises((ValueError, OSError)):
                        PREPROCESS.generate_bootstrap_manifest(payload, edition, output)
                    self.assertEqual(path.read_bytes(), b"existing manifest")


if __name__ == "__main__":
    unittest.main()
