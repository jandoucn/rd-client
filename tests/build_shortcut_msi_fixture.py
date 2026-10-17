"""Build isolated MSI fixtures from a frozen production CA; never install on host."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
ca = (ROOT / 'res/msi/CustomActions/DesktopShortcut.cpp').read_text()
header = (ROOT / 'res/msi/CustomActions/DesktopShortcut.h').read_bytes()
# Only this frozen test translation unit changes folder bindings. No production
# property, environment override or test namespace is compiled into release CA.
old = 'if (FAILED(SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, nullptr, &value))) return false;'
assert ca.count(old) == 1
ca = ca.replace(old, 'if (FAILED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, nullptr, &value))) return false;')
ca = ca.replace('result = value; CoTaskMemFree(value);',
    'result = value; CoTaskMemFree(value); result += L"\\\\RDShortcutFixture-7BC91"; result += id == FOLDERID_PublicDesktop ? L"\\\\Desktop" : L"\\\\AppRoot";')
ca = ca.replace('extern "C" UINT __stdcall InspectDesktopShortcut(MSIHANDLE install) {',
    'extern "C" UINT __stdcall InspectDesktopShortcut(MSIHANDLE install) { std::wstring fixtureDesktop; if (!folder(FOLDERID_PublicDesktop, fixtureDesktop) || MsiSetPropertyW(install, L"DesktopFolder", (fixtureDesktop + L"\\\\").c_str()) != ERROR_SUCCESS) return ERROR_INSTALL_FAILURE;')
(OUT / 'DesktopShortcut.cpp').write_text(ca)
(OUT / 'DesktopShortcut.h').write_bytes(header)
subprocess.run(['cmd.exe', '/d', '/c', 'call C:\\RustDeskBuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat && cl /nologo /EHsc /std:c++17 /MT /LD DesktopShortcut.cpp /link /EXPORT:InspectDesktopShortcut'], cwd=OUT, check=True)
component = (ROOT / 'res/msi/Package/Components/RustDesk.wxs').read_text(encoding='utf-8-sig')
import xml.etree.ElementTree as ET
ns = {'w': 'http://wixtoolset.org/schemas/v4/wxs'}
node = ET.fromstring(component).find('.//w:Component[@Id="App.Desktop.Shortcut"]', ns)
condition = node.attrib['Condition'].replace('&', '&amp;').replace('"', '&quot;')
transitive = ' Transitive="yes"' if node.attrib.get('Transitive') == 'yes' else ''
(OUT / 'DesktopShortcut.wxs').write_bytes((ROOT / 'res/msi/Package/Fragments/DesktopShortcut.wxs').read_bytes())
(OUT / 'payload.txt').write_text('inert fixture payload, never executed')
for version, code in [('1.0.0','{D38BA039-2E82-4915-A8B1-B616520CCAB1}'), ('2.0.0','{D38BA039-2E82-4915-A8B1-B616520CCAB2}')]:
    wxs = f'''<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
<Package Name="RDShortcutFixture-7BC91" Manufacturer="LocalFixture" Version="{version}" ProductCode="{code}" UpgradeCode="{{977C2314-A294-4ED1-B422-29733B5F198F}}" Scope="perUser">
<MajorUpgrade Schedule="afterInstallInitialize" DowngradeErrorMessage="fixture downgrade" />
<MediaTemplate EmbedCab="yes" />
<Property Id="DESKTOPSHORTCUTS" Value="1" />
<Binary Id="Custom_Actions_Dll" SourceFile="DesktopShortcut.dll" />
<CustomActionRef Id="InspectDesktopShortcut" />
<CustomAction Id="FailFixture" Error="fixture rollback requested" />
<InstallExecuteSequence><InstallExecute After="RemoveExistingProducts" /><Custom Action="FailFixture" After="InstallExecute" Condition="FAILFIXTURE=1" /></InstallExecuteSequence>
<StandardDirectory Id="LocalAppDataFolder"><Directory Id="ROOT" Name="RDShortcutFixture-7BC91">
<Directory Id="DesktopFolder" Name="Desktop">
<Component Id="App.Desktop.Shortcut" Guid="{{50DCB096-13B2-4AC6-8E1F-6B32C5704149}}"{transitive} Condition="{condition}">
<Shortcut Id="App.Desktop.Shortcut" Name="RustDesk" Target="[!Payload]" WorkingDirectory="APP" />
<RegistryValue Root="HKCU" Key="Software\\RDShortcutFixture-7BC91" Name="Shortcut" Type="integer" Value="1" KeyPath="yes" />
</Component></Directory>
<Directory Id="APPROOT" Name="AppRoot"><Directory Id="APP" Name="RustDesk"><Component Id="Payload" Guid="{{3134FBFC-6C36-416B-8A4C-389CE56C6C62}}"><File Id="Payload" Name="RustDesk.exe" Source="payload.txt" /></Component></Directory></Directory>
</Directory></StandardDirectory>
<Feature Id="Main"><ComponentRef Id="Payload" /><ComponentRef Id="App.Desktop.Shortcut" /></Feature>
</Package></Wix>'''
    (OUT / 'fixture.wxs').write_text(wxs)
    subprocess.run(['dotnet', str(Path.home()/'.nuget/packages/wixtoolset.sdk/4.0.5/tools/net6.0/wix.dll'), 'build', '-arch','x64','fixture.wxs','DesktopShortcut.wxs','-o',f'fixture-{version}.msi'], cwd=OUT, check=True)
files = ['res/msi/CustomActions/DesktopShortcut.cpp','res/msi/CustomActions/DesktopShortcut.h','res/msi/Package/Fragments/DesktopShortcut.wxs','res/msi/Package/Components/RustDesk.wxs']
(OUT/'sourcehash.json').write_text(json.dumps({p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in files},indent=2))
