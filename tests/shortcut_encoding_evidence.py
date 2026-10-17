"""Record hashes for this isolated encoding regression, without running products."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
out = Path(sys.argv[1])
phase = sys.argv[2]
files = [root / 'src/platform/windows/exe_desktop_shortcut.rs',
         root / 'tests/exe_desktop_adapter/cases.rs',
         Path.home() / 'AppData/Local/Temp/opencode/exe-desktop-adapter-target/debug/deps/exe_desktop_adapter_fixture-32f86fd77353bf7c.exe']
data = {'phase': phase, 'files': {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}}
with out.open('x', encoding='utf-8') as f:
    json.dump(data, f, indent=2)
