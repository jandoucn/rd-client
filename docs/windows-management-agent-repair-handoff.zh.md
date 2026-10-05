# Windows Management Agent 修复交接

修复已随 `v1.5.3-build-2026.10.05-06` 发布。真机先按 `docs/windows-msi-standard-test.zh.md` 测 MSI standard，清理用 `scripts/clean-rustdesk-windows.bat`。下面保留的是修复前的现场记录。

## 当前状态

当前先做 MSI standard 真机验收。清理工具为：

```text
scripts/clean-rustdesk-windows.bat
```

它可以清理 standard/SOS、MSI/EXE、RustDesk 服务、Management Agent、ProgramData 状态、恢复缓存、安装目录和卸载注册表项。

## 真机证据

### SOS MSI 服务注册正确

安装文件确认是：

```text
rustdesk-1.5.3-2026100505-sos-windows-x86_64.msi
```

Windows 服务命令行为：

```text
C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk\"
```

因此 MSI 构建和 WiX 的 `Edition` 注入是正确的。

### 手动初始化正确

执行：

```powershell
& "$env:ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe" check-now --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk"
```

会写出正确状态：

```json
{
  "edition": "sos",
  "package_kind": "msi",
  "target_key": "windows-x86_64-msi-sos",
  "recovery_install_path": "C:\\ProgramData\\RustDesk\\Client\\sos\\RustDesk",
  "last_error": ""
}
```

### Agent 服务运行后覆盖状态

等待 Agent 服务运行一段时间后，曾出现：

```json
{
  "edition": "sos",
  "package_kind": "msi",
  "target_key": "windows-x86_64-msi-standard",
  "recovery_install_path": "C:\\ProgramData\\RustDesk\\Client\\sos\\RustDesk"
}
```

这证明手动 `check-now` 没有问题，问题在 Agent 服务循环。

### SOS MSI 卸载后未自动恢复

卸载 RustDesk MSI、保留 Agent，并等待超过 5 分钟后，现场只有：

```text
RustDeskManagementAgent Running
```

没有恢复客户端，也没有恢复后的 `RustDesk` 服务。需要在修复后重新做 MSI SOS 卸载恢复验收。

## 根因一：服务配置回退 standard

相关文件：

```text
src/management_agent.rs
```

关键代码：

```rust
pub fn run_service(config: ServiceConfig, paths: AgentPaths) -> Result<(), String> {
    let _ = (config, paths);
    service_dispatcher::start("RustDeskManagementAgent", ffi_service_main)
}
```

`run_service()` 丢弃了调用方解析出的 `config` 和 `paths`。

随后 `service_main()` 重新使用 Windows SCM 回调参数解析：

```rust
fn service_main(_arguments: Vec<std::ffi::OsString>) -> windows_service::Result<()> {
    let config = parse_service_arguments(_arguments ...)?;
}
```

SCM 回调参数不等于服务 `ImagePath` 中的完整命令行，因此 SOS 参数没有可靠传入，解析结果回到默认值：

```rust
Edition::Standard
PackageKind::Msi
```

服务每 30 秒进入一次循环，重新写入 standard target，导致：

```text
edition = sos
target_key = windows-x86_64-msi-standard
```

## 根因二：状态同步不完整

服务循环当前只同步：

```rust
state.package_kind = config.package_kind;
state.target_key = config.edition.target_key_for(config.package_kind).to_owned();
```

缺少：

```rust
state.edition = config.edition;
state.recovery_install_path = paths.client.to_string_lossy().into_owned();
```

`check_now_with_package()` 也应同步 `recovery_install_path`，避免切换 edition 后遗留旧路径。

## 第一部分修复要求：Agent 服务配置

建议增加统一状态同步函数：

```rust
fn apply_service_config(
    state: &mut AgentState,
    config: &ServiceConfig,
    paths: &AgentPaths,
) {
    state.edition = config.edition;
    state.package_kind = config.package_kind;
    state.target_key = config.edition.target_key_for(config.package_kind).to_owned();
    state.recovery_install_path = paths.client.to_string_lossy().into_owned();
}
```

服务启动时必须使用当前进程的完整命令行解析参数，不能只依赖 SCM 回调参数：

```rust
std::env::args().skip(1)
```

服务循环每次加载状态后调用统一同步函数。

需要增加回归测试：

1. SOS 服务配置不能回退 standard。
2. `target_key` 必须是 `windows-x86_64-msi-sos`。
3. SOS 恢复路径必须是 `Client\\sos\\RustDesk`。
4. 状态刷新后 `edition`、`package_kind`、`target_key`、`recovery_install_path` 一致。

## 根因三：EXE 双击安装没有注册 Agent

相关文件：

```text
src/platform/windows.rs
```

相关函数：

```text
install_me()
get_after_install()
get_create_service()
```

当前 EXE 安装会复制：

```text
C:\Program Files\RustDesk\rustdesk-management-agent.exe
```

但不会自动：

1. 复制 Agent 到 `C:\ProgramData\RustDesk\ManagementAgent`。
2. 创建 `RustDeskManagementAgent` 服务。
3. 启动 Agent 服务。

MSI 由以下 WiX 文件负责注册 Agent，不能破坏现有 MSI 行为：

```text
res/msi/Package/Components/ManagementAgent.wxs
```

## 第二部分修复要求：EXE 全自动注册 Agent

在 EXE 安装的 elevated command script 中增加：

```text
复制安装目录中的 rustdesk-management-agent.exe 到：
C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe

创建并启动：
RustDeskManagementAgent
```

服务参数必须根据编译 edition 写入：

```text
--service --edition standard --package-kind exe --normal-install-path "C:\Program Files\RustDesk\"
```

或：

```text
--service --edition sos --package-kind exe --normal-install-path "C:\Program Files\RustDesk\"
```

编译 edition 可由：

```rust
option_env!("RUSTDESK_SOS") == Some("1")
```

决定。

EXE 卸载时要保留 ProgramData Agent，方便卸载后恢复；完整清理仍由测试 BAT 或 Agent purge 负责。

## 根因四：EXE 恢复把 portable wrapper 直接注册成服务

相关函数：

```text
src/management_agent.rs::install_exe_package
```

当前逻辑：

1. 下载 EXE。
2. 复制到 `ProgramData\\RustDesk\\Client\\<edition>\\RustDesk\\RustDesk.exe`。
3. 注册：

```text
RustDesk.exe --service
```

但发布的 EXE 是 portable wrapper。wrapper 会：

1. 解包内部 RustDesk。
2. 启动内部 RustDesk。
3. wrapper 自己退出。

Windows SCM 因此记录：

```text
7009: 等待 RustDesk 服务连接超时
7000: RustDesk 服务启动失败
```

现场事件中的服务路径：

```text
"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe" --service
```

## 第三部分修复要求：EXE 稳定解包后注册真实服务

不能继续把 portable wrapper 直接作为 SCM 服务进程。推荐：

1. 为 portable packer 增加稳定解包入口，或增加专用 extraction 参数。
2. 将内部真实 RustDesk 可执行文件释放到：

```text
C:\ProgramData\RustDesk\Client\<edition>\RustDesk\
```

3. 注册真正的内部 RustDesk 可执行文件为 `RustDesk` 服务。
4. 服务启动后进程本身必须持续向 SCM 报告服务状态。
5. 恢复安装失败时删除临时目录，不破坏旧状态。

## 时间行为

Agent 服务循环当前每 30 秒唤醒一次。

首次发现普通客户端缺失时，当前逻辑会先设置初始检查时间，初始 retry delay 是 5 分钟：

```rust
0 => Duration::from_secs(5 * 60)
```

因此自动恢复观察窗口应按 5 分钟加最多 30-60 秒计算。手动 `check-now` 可跳过等待。

## 修复后验收

### SOS MSI

安装后：

```text
RustDesk                  Running
RustDeskManagementAgent   Running
```

Agent 服务路径包含：

```text
--edition sos --package-kind msi
```

状态持续保持：

```json
"edition": "sos",
"target_key": "windows-x86_64-msi-sos"
```

卸载 MSI、保留 Agent 后，恢复结果必须是：

```text
C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe 存在
RustDesk 服务 Running
RustDeskManagementAgent 服务 Running
last_primary_url 以 -sos-windows-x86_64.msi 结尾
last_error 为空
```

### EXE

双击 EXE 安装后应自动出现：

```text
RustDesk
RustDeskManagementAgent
```

Agent 服务参数包含：

```text
--package-kind exe
```

卸载主程序后 Agent 保留，并能恢复 EXE；恢复后的 RustDesk 服务不能再出现 7009/7000。

### 本地检查

```bash
cargo fmt --all -- --check
cargo test --locked --bin rustdesk-management-agent
python -m unittest tests/test_management_agent_msi.py
```

Windows 真机检查：

```powershell
Get-CimInstance Win32_Service -ErrorAction SilentlyContinue | Where-Object {$_.Name -match '^RustDesk'} | Select-Object Name,State,StartMode,PathName
```

## 当前交接结论

修复优先级：

```text
1. 修复 Agent 服务参数解析，避免 SOS 回退 standard。
2. 同步 AgentState 的 edition、target_key、recovery_install_path。
3. EXE 首次安装自动注册 Management Agent。
4. EXE 恢复改为稳定解包后注册真实 RustDesk 服务。
5. 重新构建 Windows standard/SOS EXE 和 MSI。
6. 在 Windows 真机完成 MSI SOS、EXE standard/SOS 的卸载恢复验收。

## 最新真机证据：standard MSI 自动恢复失败

### 测试步骤

在 Windows 真机上安装 standard MSI，然后执行静默卸载并等待：

```powershell
$p=Get-ChildItem 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue|ForEach-Object{Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue}|Where-Object{$_.DisplayName -eq 'RustDesk' -and $_.WindowsInstaller -eq 1}|Select-Object -First 1;if(!$p){throw '未找到 RustDesk MSI'};$r=Start-Process msiexec.exe -ArgumentList @('/x',$p.PSChildName,'/qn','/norestart') -Wait -PassThru;Write-Host "MSI uninstall exit=$($r.ExitCode)";Start-Sleep -Seconds 310
```

卸载结果：

```text
MSI uninstall exit=0
```

卸载后立即观察到：

```text
RustDeskManagementAgent Running Automatic
```

状态文件：

```json
{
  "edition": "standard",
  "package_kind": "msi",
  "target_key": "windows-x86_64-msi-standard",
  "next_check_at": "1791205922",
  "retry_stage": 0,
  "last_version": "",
  "last_error": ""
}
```

### 自动恢复确实被触发

`next_check_at` 到期时间约为 `21:12:02`。随后发现两个 MSI 进程：

```text
7108  C:\Windows\System32\MsiExec.exe       StartTime 21:12:04
19304 C:\Windows\system32\msiexec.exe       StartTime 21:12:03
```

这证明 Agent 的 30 秒轮询、5 分钟延迟和自动启动 MSI 都已经工作，问题不在调度、manifest 下载或恢复入口。

### MSI 最终失败日志

Windows Application 日志：

```text
2026/10/5 21:16:07  11921  错误
Error 1921. Service 'RustDesk Management Agent' (RustDeskManagementAgent) could not be stopped.

2026/10/5 21:16:08  11708  信息
Product: RustDesk -- Installation failed.

2026/10/5 21:16:08  1033   信息
Windows Installer 已安装产品。产品版本: 1.5.3.29853299。
```

注意：`1033` 只是产品注册/安装事件，不能作为成功判据；本次最终结果由 `11708` 和 `Error 1921` 确认是失败。没有出现成功事件 `11707`。

最终 Agent 状态：

```json
{
  "retry_stage": 1,
  "last_version": "",
  "last_error": "msiexec exited with exit code: 1603"
}
```

Windows 服务状态：

```text
RustDeskManagementAgent Stopped Automatic
```

恢复目录没有生成 RustDesk 客户端，RustDesk 服务也没有恢复。

## 新确认根因：MSI 自停止死锁

当前恢复调用在 `src/management_agent.rs::check_now_with_package()` 中同步等待 MSI：

```rust
std::process::Command::new("msiexec.exe")
    .arg("/i")
    .arg(&cache)
    .args(recovery_msi_args(paths))
    .status()
```

而 MSI 的 Agent 组件配置了：

```xml
<ServiceControl
    Name="RustDeskManagementAgent"
    Start="install"
    Stop="install"
    Remove="never"
    Wait="yes" />
```

因此实际链路是：

```text
RustDeskManagementAgent
  -> 同步启动 msiexec
  -> MSI 安装时尝试停止 RustDeskManagementAgent
  -> Agent 正在等待 msiexec 返回，无法完成自停止
  -> MSI Error 1921 / exit code 1603
  -> Agent 被留下为 Stopped
```

这不是等待时间不足，也不是 manifest、下载、SHA-256 或签名问题。当前版本的 MSI 自动恢复必须判定为失败。

## 修复要求：解开 Agent 自升级冲突

修复必须满足以下行为：

1. Agent 启动 MSI 恢复时，安装事务不能要求当前 Agent 同步自停止。
2. MSI 安装完成后，`RustDeskManagementAgent` 必须回到 `Running/Automatic`。
3. MSI 失败时，Agent 也必须能够恢复运行，不能留下 `Stopped` 状态。
4. 成功安装后才允许写入 `last_version`、`last_primary_url` 并清空 `last_error`。
5. 失败时必须保留 `last_error=msiexec exited with exit code: 1603`，并按 retry policy 安排下次重试。

实现方向由修复窗口评估，至少应选择并测试一种完整方案：

- 由独立、脱离 Agent 生命周期的 helper 启动 MSI，Agent 先安全退出/停止，再由 helper 安装并恢复 Agent；或
- 为恢复事务增加专用 MSI 属性/条件，避免恢复安装阶段停止自身 Agent，并验证 Agent 文件替换和服务重启；或
- 使用一次性计划任务/RunOnce/helper 完成服务停止、MSI 安装和服务启动，避免 Agent 进程同步等待自己的停止操作。

不能只把 `msiexec` 改成异步启动而不处理进程生命周期，否则可能出现 MSI 尚未完成、Agent 提前继续运行、状态提前写成功或服务文件锁定等竞态。

## 修复验收命令

安装后卸载并等待自动恢复，查看 MSI 事件：

```powershell
Get-WinEvent -FilterHashtable @{LogName='Application';StartTime=(Get-Date).AddMinutes(-15)} -ErrorAction SilentlyContinue|Where-Object{$_.ProviderName -eq 'MsiInstaller'}|Sort-Object TimeCreated -Descending|Select-Object -First 20 TimeCreated,Id,LevelDisplayName,Message
```

成功必须同时满足：

```text
没有 Error 1921
没有事件 11708
存在事件 11707
存在事件 1033 且安装状态为 0
```

最终状态：

```powershell
Get-Item "$env:ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe" -ErrorAction SilentlyContinue|Select-Object FullName,Length,LastWriteTime;Get-Service RustDesk,RustDeskManagementAgent -ErrorAction SilentlyContinue|Select-Object Name,Status,StartType;Get-Content "$env:ProgramData\RustDesk\ManagementAgent\state\agent.json" -ErrorAction SilentlyContinue
```

预期：

```text
C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe 存在
RustDesk Running Automatic
RustDeskManagementAgent Running Automatic
last_error 为空
last_version = 1.5.3
```

修复后还要重复执行同一流程验证 SOS，确认 `windows-x86_64-msi-sos` 不回退，并分别验证 EXE 的安装、卸载和恢复链路。
```
