"""Package current peer-online sources without running any client or test."""
import hashlib
import json
import sys
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve()
paths = [
    'lib/models/ab_model.dart',
    'lib/models/peer_model.dart',
    'lib/models/platform_model.dart',
    'test/ab_online_reload_test.dart',
    'test/peer_online_reload_test.dart',
]
records = []
with zipfile.ZipFile(output, 'x', zipfile.ZIP_DEFLATED) as bundle:
    for path in paths:
        data = (root / 'flutter' / path).read_bytes()
        records.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest()})
        bundle.writestr(path, data)
    for path in ['integration_test/peer_online_session_test.dart',
                 'test_driver/integration_test.dart', 'README.zh.md', 'prepare.py']:
        source = root / 'tests/fixtures/peer_online' / path
        data = source.read_bytes()
        destination = 'fixtures/peer_online/' + path
        records.append({'path': destination, 'sha256': hashlib.sha256(data).hexdigest()})
        bundle.writestr(destination, data)
    manifest = {
        'phase': 'baseline-pending-vm',
        'expectedModelWidgetCases': 23,
        'expectedE2ECasesSeparateRun': 1,
        'executedCasesThisRun': 0,
        'records': records,
        'vmCommand': 'flutter --no-version-check test --no-pub --reporter=json '
                     '--concurrency=1 test/ab_online_reload_test.dart '
                     'test/peer_online_reload_test.dart',
        'e2e': 'Use tests/fixtures/peer_online/prepare.py in the full repository; see fixtures/peer_online/README.zh.md; '
               'external driver and current source build must be verified by VM owner.',
    }
    bundle.writestr('peer-online-manifest.json', json.dumps(manifest, indent=2))
print(json.dumps({'artifact': str(output),
                  'sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
                  **manifest}, indent=2))
