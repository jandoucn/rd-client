# T01/T02 Windows Installer 来源命名实测交接

宿主只构建，未安装或运行driver。现有T02 JSON检索未发现PackageName/LastUsedSource直接回执，未据历史日志猜当前注册值。

制品目录：`C:\Users\Administrator\AppData\Local\Temp\opencode\t01-source-name-fixture-20261016`。
原名`t01-source-original.msi`与重命名`package.msi`为完全相同字节，SHA256均`85b2aa30a22d3a14cbfdb25981dd22974ff90e323984834d29f03bc33ca71d46`。
driver `t01_source_name_fixture.ps1` SHA256=`a25a366f61f6579a5534a4689ae66bc1ef76cd692bfd346b007a3965659736e4`。PowerShell5.1 ParseFile语法检查通过，非运行测试。

固定测试产品T01SourceNameFixture-A731，ProductCode={127981AF-9DA4-4B73-A731-9E07CCCB0901}，UpgradeCode尾号0902，Component尾号0903。perUser，只写HKCU\Software\T01SourceNameFixture-A731\Installed=1，无文件payload、服务、任务、CustomAction、升级/删除生产产品动作。测试标记或注册预存即拒绝。正常/x仅指向此专属GUID，异常finally也只卸载该GUID，不手删注册表。

Win11 owner在已安排的唯一VM窗口，以交互用户运行（非SYSTEM），复制完整目录，调用：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\t01_source_name_fixture.ps1 -ExpectedMsiSha256 85b2aa30a22d3a14cbfdb25981dd22974ff90e323984834d29f03bc33ca71d46
```

顺序：原名安装→真实MsiSourceListGetInfoW(context2)取PackageName/LastUsedSource/LastUsedType→核实际注册源哈希→正常卸载并确认1605及marker缺失→package.msi重名安装→同样查询→卸载。安装/卸载Start-Process -Wait -PassThru，stdout/stderr分别落盘，verbose MSI log独立保留。finally输出result，失败仍尝试正常卸载并记录失败，禁止无回执判成功。

预期PackageName跟随调用basename；这是待实测假设。如果观测不同，driver保留原值并失败，作为反证。result同时记录production_basename_comparison，表示将真实PackageName与原manifest式basename严格相等比较的结果。生产package_gate::cache_name返回target.primary原basename，producer直接比较registered_name；本fixture不伪造签名manifest也不修改生产ProductName=RustDesk合同，因此不调用必然拒绝测试ProductName的production installed_source，而用相同MSI API真实取值。生产context4仍需真实安装正向验收，此peruser实验不能冒充它。

保全范围：现有Agent state、LocalService identity/config、主EXE和Agent EXE的存在性/哈希及双服务状态前后比较；不声称涵盖全部生产文件、SCM配置或注册表。测试文件本身只留MSI包/日志，不在应用目录部署文件。新增run-UUID证据保留供主审查；正常卸载后仅测试注册和marker应消失。已有fixture/旧RED现场不清理。未运行VM/CI、未提交、无子代理。
