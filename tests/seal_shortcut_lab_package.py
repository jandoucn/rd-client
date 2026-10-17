from pathlib import Path
import hashlib,json
base=Path.home()/'AppData/Local/Temp/opencode/shortcut-fullca-20261016c'
files=['lab-sos1501-full-ca.msi','res/msi/CustomActions/x64/Release/CustomActions.dll','payload1501.manifest.json','build.json','package.json']
result={'installed':False,'online_e2e':False,'official':False,'build_exit':0,'full_ca_build_exit':0,
        'uac':'RunAs issued; consent.exe observed; no probe receipt; screenshot E_FAIL 0x80004005; no keys sent',
        'snapshot':'b14a0a4f-8097-4984-9cf7-996ac820c70d',
        'sha256':{f:hashlib.sha256((base/f).read_bytes()).hexdigest() for f in files}}
(base/'continuation.json').write_text(json.dumps(result,indent=2))
print(json.dumps(result,indent=2))
