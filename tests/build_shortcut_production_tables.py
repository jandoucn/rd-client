"""Generate current production WXS table samples using explicitly OLD payloads.
Never execute or publish these inspection-only packages.
"""
from pathlib import Path
import shutil
import subprocess
import sys
import hashlib
import json

root=Path(__file__).resolve().parents[1]
out=Path(sys.argv[1]); out.mkdir(exist_ok=False)
temp=Path.home()/'AppData/Local/Temp/opencode'
historical=temp/'agent-sos-msi-v187-2h8kcfj4/res/msi/Package'
nuget=Path.home()/'.nuget/packages'
for edition in ('standard','sos'):
    package=out/edition
    shutil.copytree(root/'res/msi/Package',package)
    shutil.copytree(historical/'Resources',package/'Resources',dirs_exist_ok=True)
    includes=(historical/'Includes.wxi').read_text(encoding='utf-8-sig').replace('Edition="sos"',f'Edition="{edition}"')
    (package/'Includes.wxi').write_text(includes,encoding='utf-8')
    command=['dotnet',str(nuget/'wixtoolset.sdk/4.0.5/tools/net6.0/wix.dll'),'build','-arch','x64','-culture','en-us',
             '-d',f'CustomActions.TargetDir={temp}/','-d','CustomActions.TargetName=msi-desktop-shortcut-ca',
             '-loc','Language/Package.en-us.wxl','-o',str(out/f'{edition}-TABLES-ONLY.msi')]
    for ext in ('Firewall','Netfx','UI','Util'):
        command+=['-ext',str(nuget/f'wixtoolset.{ext.lower()}.wixext/4.0.5/wixext4/WixToolset.{ext}.wixext.dll')]
    command += [str(p.relative_to(package)) for p in package.rglob('*.wxs')]
    subprocess.run(command,cwd=package,check=True)
(out/'limitations.json').write_text(json.dumps({'purpose':'table inspection only; NEVER INSTALL',
    'payload':'OLD SOS v186 payload used for BOTH edition table samples',
    'ca':'current standalone desktop CA only; other exported actions are NOT supplied',
    'source_sha256':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in (root/'res/msi/Package').rglob('*.wxs')}},indent=2))
