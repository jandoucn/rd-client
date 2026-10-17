"""Copy the real application and E2E entrypoints into a new isolated fixture."""
import hashlib
import json
import shutil
import sys
from pathlib import Path

source = Path(__file__).resolve().parents[3]
root = Path(sys.argv[1]).resolve()
if root == source or source in root.parents:
    raise SystemExit('隔离目录必须在仓库外')
root.mkdir(exist_ok=False)
records = []


def copy(path, relative):
    target = root / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    before = hashlib.sha256(path.read_bytes()).hexdigest()
    shutil.copy2(path, target)
    assert hashlib.sha256(target.read_bytes()).hexdigest() == before
    assert hashlib.sha256(path.read_bytes()).hexdigest() == before
    records.append({'source': path.relative_to(source).as_posix(),
                    'path': relative, 'sha256': before})


for directory in ['lib', 'assets', 'windows']:
    for path in sorted((source / 'flutter' / directory).rglob('*')):
        if path.is_file() and 'ephemeral' not in path.parts:
            copy(path, path.relative_to(source).as_posix())
for directory in ['integration_test', 'test_driver']:
    for path in sorted((Path(__file__).parent / directory).rglob('*.dart')):
        copy(path, 'flutter/' + path.relative_to(Path(__file__).parent).as_posix())
for relative in ['flutter/pubspec.yaml', 'flutter/pubspec.lock', 'target/debug/librustdesk.dll']:
    copy(source / relative, relative)
pubspec = root / 'flutter/pubspec.yaml'
text = pubspec.read_text(encoding='utf-8')
assert text.count('  ffigen: ^8.0.2') == 1
assert '  integration_test:' not in text
assert text.count('dev_dependencies:\n') == 1
text = text.replace('  ffigen: ^8.0.2', '  ffigen: 9.0.1')
text = text.replace('dev_dependencies:\n', 'dev_dependencies:\n  integration_test:\n    sdk: flutter\n', 1)
pubspec.write_text(text, encoding='utf-8', newline='\n')
(root / 'source-manifest.json').write_text(json.dumps({
    'source': str(source), 'fixture': str(root), 'records': records,
    'fixtureOnlyChanges': ['integration_test SDK', 'ffigen 9.0.1; 不运行 codegen'],
    'injectedPubspecSha256': hashlib.sha256(pubspec.read_bytes()).hexdigest(),
    'vmE2EExecuted': False,
}, ensure_ascii=False, indent=2), encoding='utf-8')
print(root / 'flutter')
