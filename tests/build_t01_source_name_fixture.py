"""Build only: inert per-user MSI and byte-identical renamed copy."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

out = Path(sys.argv[1]).resolve()
out.mkdir(exist_ok=False)
wxs = '''<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
<Package Name="T01SourceNameFixture-A731" Manufacturer="LocalFixture" Version="1.0.0"
 ProductCode="{127981AF-9DA4-4B73-A731-9E07CCCB0901}"
 UpgradeCode="{127981AF-9DA4-4B73-A731-9E07CCCB0902}" Scope="perUser">
<StandardDirectory Id="LocalAppDataFolder">
<Component Id="Marker" Guid="{127981AF-9DA4-4B73-A731-9E07CCCB0903}">
<RegistryValue Root="HKCU" Key="Software\\T01SourceNameFixture-A731" Name="Installed" Type="integer" Value="1" KeyPath="yes" />
</Component></StandardDirectory>
<Feature Id="Main"><ComponentRef Id="Marker" /></Feature>
</Package></Wix>'''
(out / 'fixture.wxs').write_text(wxs, encoding='utf-8')
wix = Path.home() / '.nuget/packages/wixtoolset.sdk/4.0.5/tools/net6.0/wix.dll'
subprocess.run(['dotnet', str(wix), 'build', '-arch', 'x64', str(out/'fixture.wxs'),
                '-o', str(out/'t01-source-original.msi')], check=True)
original = (out/'t01-source-original.msi').read_bytes()
(out/'package.msi').write_bytes(original)
driver = Path(__file__).with_name('t01_source_name_fixture.ps1')
(out/driver.name).write_bytes(driver.read_bytes())
receipt = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
           for p in out.iterdir() if p.is_file()}
(out/'sha256.json').write_text(json.dumps(receipt, indent=2), encoding='utf-8')
print(json.dumps(receipt, indent=2))
