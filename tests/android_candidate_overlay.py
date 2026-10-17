"""Build a source-only Android overlay; never run tests or resolve dependencies."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import zipfile

root = Path(__file__).resolve().parents[1]
manifest_path = root / 'tests/android_candidate_testmanifest.json'
manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
destination = Path(sys.argv[1]).resolve()
if not destination.parent.is_dir() or destination.exists():
    raise SystemExit('Destination parent must exist and archive must be absent')

def digest(data):
    return hashlib.sha256(data).hexdigest()

dependencies = []
for name in manifest['dependency_preconditions']:
    original = subprocess.check_output(['git', 'show', 'HEAD:' + name], cwd=root)
    current = (root / name).read_bytes()
    if original.replace(b'\r\n', b'\n') != current.replace(b'\r\n', b'\n'):
        raise SystemExit('Dependency differs from HEAD: ' + name)
    dependencies.append({'path': name, 'sha256': digest(current),
                         'normalized_sha256': digest(current.replace(b'\r\n', b'\n'))})

payload = {name: (root / name).read_bytes() for name in manifest['sources']}
receipt = {
    'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root).decode().strip(),
    'sources': [{'path': name, 'bytes': len(data), 'sha256': digest(data)}
                for name, data in payload.items()],
    'dependencies_not_in_archive': dependencies,
    'tests_executed': False,
    'application': 'Apply paths only to a main-owner frozen test checkout; verify hashes first. No dependency files included.',
}
with zipfile.ZipFile(destination, 'x', compression=zipfile.ZIP_DEFLATED) as archive:
    for name, data in payload.items():
        archive.writestr(name, data)
    archive.writestr('tests/android_candidate_testmanifest.json', manifest_path.read_bytes())
    archive.writestr('overlay-receipt.json', json.dumps(receipt, ensure_ascii=False, indent=2))
with zipfile.ZipFile(destination) as archive:
    for name, data in payload.items():
        if archive.read(name) != data:
            raise SystemExit('Archive verification failed: ' + name)
print(json.dumps({'archive': str(destination), 'sha256': digest(destination.read_bytes()),
                  'source_count': len(payload), 'tests_executed': False}, indent=2))
