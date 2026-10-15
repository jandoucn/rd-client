"""Create a new isolated copy of the actual portable packer with a minimal lab payload.

Does not execute a Windows binary. The generated EXE must only run in the lab VM.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
FILES = {
    'rustdesk.exe': b'fixture-client-not-executed',
    'librustdesk.dll': b'fixture-dll',
    'rustdesk-management-agent.exe': b'fixture-agent-not-executed',
    'flutter_windows.dll': b'fixture-flutter',
    'window_manager_plugin.dll': b'fixture-plugin',
    'data/icudtl.dat': b'fixture-icu',
    'data/flutter_assets/AssetManifest.json': b'{"fixture":true}',
}

def generate(destination: Path, variant: str):
    if destination.exists():
        raise SystemExit('Refusing existing fixture root')
    destination.mkdir()
    packer = destination / 'packer'
    shutil.copytree(ROOT / 'libs/portable', packer, ignore=shutil.ignore_patterns('target', 'data.bin', 'app_metadata.toml', '__pycache__'))
    # Make the copy a standalone crate while resolving the same exact dependencies.
    with (packer / 'Cargo.toml').open('a', encoding='utf-8') as stream:
        stream.write('\n[workspace]\n')
    shutil.copy2(ROOT / 'Cargo.lock', packer / 'Cargo.lock')
    resource = destination / 'res'
    resource.mkdir(exist_ok=True)
    for name in ['icon.ico', 'manifest.xml']:
        target = resource / name
        if target.exists() and target.read_bytes() != (ROOT / 'res' / name).read_bytes():
            raise SystemExit('Conflicting fixture resource')
        if not target.exists():
            shutil.copy2(ROOT / 'res' / name, target)
    build = packer / 'build.rs'
    build.write_text(build.read_text(encoding='utf-8').replace('../../res/', '../res/'), encoding='utf-8')
    payload = destination / 'payload'
    for name, data in FILES.items():
        path = payload / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    spec = importlib.util.spec_from_file_location('actual_portable_generator', ROOT / 'libs/portable/generate.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    table=module.generate_md5_table(str(payload), 1)
    if variant == 'missing':
        table.pop(next(name for name in table if name.endswith('librustdesk.dll')))
    elif variant == 'escape':
        table['../escape-sentinel'] = (b'bad', b'a'*32)
    elif variant == 'corrupt':
        name=next(iter(table))
        table[name]=(b'not valid brotli',table[name][1])
    module.write_package_metadata(table, str(packer), './rustdesk.exe')
    (packer / 'app_metadata.toml').write_text('timestamp = 215\n', encoding='utf-8')
    (destination / 'fixture.json').write_text(json.dumps({'scope':'file extraction only, never execute payload', 'files':{k:hashlib.sha256(v).hexdigest() for k,v in FILES.items()}},indent=2),encoding='utf-8')

if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('destination',type=Path)
    parser.add_argument('--variant',choices=['good','missing','escape','corrupt'],default='good')
    args=parser.parse_args()
    generate(args.destination.resolve(),args.variant)
