"""Offline full-CA package with declared historical payload, never run on host."""
from pathlib import Path
import importlib.util, shutil, subprocess, sys, hashlib, json
root=Path(__file__).resolve().parents[1]
out=Path(sys.argv[1]).resolve()
package=out/'res/msi/Package'
temp=Path.home()/'AppData/Local/Temp/opencode'
payload=Path(sys.argv[2]).resolve() if len(sys.argv)>2 else temp/'agent-sos-v186/payload'
historical=temp/'agent-sos-msi-v187-2h8kcfj4/res/msi/Package'
shutil.copytree(historical/'Resources',package/'Resources',dirs_exist_ok=True)
(package/'Includes.wxi').write_bytes((historical/'Includes.wxi').read_bytes())
if len(sys.argv)>2:
    includes=(package/'Includes.wxi').read_text(encoding='utf-8-sig')
    includes=includes.replace('Version="1.5.3.20261005"','Version="1.5.3.29867639"').replace(str(temp/'agent-sos-v186/payload').replace('/','\\'),str(payload))
    (package/'Includes.wxi').write_text(includes,encoding='utf-8')
spec=importlib.util.spec_from_file_location('prep',root/'res/msi/preprocess.py');prep=importlib.util.module_from_spec(spec);spec.loader.exec_module(prep)
prep.sys.argv[0]=str(out/'res/msi/preprocess.py')
shutil.copy2(root/'res/msi/Package/Components/RustDesk.wxs',package/'Components/RustDesk.wxs')
assert prep.gen_auto_component('RustDesk',payload)
prep.generate_bootstrap_manifest(payload,'sos',package/'Resources')
nuget=Path.home()/'.nuget/packages'
dll=out/'res/msi/CustomActions/x64/Release/CustomActions.dll'
command=['dotnet',str(nuget/'wixtoolset.sdk/4.0.5/tools/net6.0/wix.dll'),'build','-arch','x64','-culture','en-us','-d',f'CustomActions.TargetDir={dll.parent}/','-d','CustomActions.TargetName=CustomActions','-loc','Language/Package.en-us.wxl','-o',str(out/('lab-sos1501-full-ca.msi' if len(sys.argv)>2 else 'historical-sos-full-ca.msi'))]
for ext in ('Firewall','Netfx','UI','Util'):command+=['-ext',str(nuget/f'wixtoolset.{ext.lower()}.wixext/4.0.5/wixext4/WixToolset.{ext}.wixext.dll')]
command += [str(p.relative_to(package)) for p in package.rglob('*.wxs')]
r=subprocess.run(command,cwd=package,capture_output=True)
(out/'package.stdout').write_bytes(r.stdout);(out/'package.stderr').write_bytes(r.stderr)
(out/'package.json').write_text(json.dumps({'exit':r.returncode,'payload':str(payload),'official_package':False,'dll_sha256':hashlib.sha256(dll.read_bytes()).hexdigest(),'payload_hashes':{str(p.relative_to(payload)):hashlib.sha256(p.read_bytes()).hexdigest() for p in payload.rglob('*') if p.is_file()}},indent=2))
print('package exit',r.returncode);sys.exit(r.returncode)
