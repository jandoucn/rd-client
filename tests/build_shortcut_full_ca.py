"""Freeze and compile ALL current MSI custom actions offline, single worker."""
from pathlib import Path
import shutil, subprocess, sys, hashlib, json
root=Path(__file__).resolve().parents[1]
out=Path(sys.argv[1]);out.mkdir(exist_ok=False)
shutil.copytree(root/'res/msi',out/'res/msi',ignore=shutil.ignore_patterns('obj','x64','bin'))
shutil.copy2(root/'res/defender-exclusions.ps1',out/'res/defender-exclusions.ps1')
packages=root/'res/msi/packages'
if not packages.exists():
    packages=Path.home()/'AppData/Local/Temp/opencode/agent-sos-msi-v187-2h8kcfj4/res/msi/packages'
shutil.copytree(packages,out/'res/msi/packages',dirs_exist_ok=True)
cmd='call "C:\\RustDeskBuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat" && msbuild res\\msi\\CustomActions\\CustomActions.vcxproj /t:Rebuild /p:Configuration=Release /p:Platform=x64 /m:1 /verbosity:minimal'
(out/'build.cmd').write_text('@echo off\n'+cmd+'\nexit /b %errorlevel%\n')
result=subprocess.run(['cmd.exe','/d','/c',str(out/'build.cmd')],cwd=out,capture_output=True)
(out/'build.stdout').write_bytes(result.stdout);(out/'build.stderr').write_bytes(result.stderr)
(out/'build.json').write_text(json.dumps({'exit':result.returncode,'sources':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in (root/'res/msi').rglob('*') if p.is_file() and p.suffix in ('.cpp','.h','.wxs','.vcxproj','.def')}},indent=2))
print('full CA build exit:',result.returncode);sys.exit(result.returncode)
