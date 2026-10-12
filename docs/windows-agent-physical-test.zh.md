# Windows agent/主程序物理机验收说明

状态：等待物理机验收。

本说明用于补齐虚拟机无法完成的 agent 和主程序故障恢复测试。所有涉及服务停止、进程终止、服务配置修改、文件暂时移走和在线升级的操作，都必须在专用物理测试机执行，不能在生产机执行。

## 当前已经完成

- Defender mock 测试 7 项全部通过。
- recovery 决策测试 13 项全部通过。
- agent Windows 目标 `cargo check` 通过。
- 新 MSI 编译通过。
- MSI 旧包安装、Debug MSI 升级、双服务 LocalSystem/Auto/Running 及 agent 文件替换在既有 VM 记录中通过。
- 新 Defender 逻辑在 VM 旧升级现场验证过五项排除项和两组防火墙规则。
- agent status 协议固定声明：

  ```text
  agent.status@1
  service.inspect@1
  ```

- 代码已经接入以下恢复策略：

  ```text
  可信服务路径可用       -> 启动现有服务
  主程序文件仍在但服务异常 -> 重建/修复主程序服务
  主程序不可用、fallback 存在 -> 重建/修复 fallback 服务
  两者都不可用             -> 进入签名 manifest/安装包恢复
  ```

## VM 未完成的原因

VirtualBox VM 中的 `rdlab` 是受 UAC 限制的普通令牌。Guest Control 可以复制文件和读取服务，但不能稳定创建、启动或运行新的 SYSTEM 计划任务。多次 UAC 安全桌面确认没有形成可靠的管理员执行证据，因此以下最新代码故障矩阵没有被伪报为通过。

另外，VM 中使用的 MSI 保持相同产品版本。重复安装返回 `0`，但 Windows Installer 没有真正替换已经安装的 agent。VM 中实际 agent SHA-256 与最新本机构建 SHA-256 不一致，因此那次升级只能证明安装器返回成功，不能证明最新 fallback 代码已经在 VM 中运行。

物理机验收必须使用本次最终构建出的新包，并先核对安装后的 agent SHA-256，确认运行的确实是这次构建。

## 物理机前置条件

1. 使用专用 Windows 10 或 Windows 11 物理测试机，管理员 PowerShell。
2. 先保存以下信息：

   ```powershell
   Get-ComputerInfo | Select-Object WindowsProductName,WindowsVersion,OsBuildNumber
   Get-FileHash .\rustdesk-management-agent.exe -Algorithm SHA256
   Get-FileHash .\Package.msi -Algorithm SHA256
   ```

3. 确认不是生产机，并准备可恢复的系统备份或磁盘快照。
4. 关闭真实远控会话后再开始故障注入。
5. 记录当前配置文件和服务状态：

   ```powershell
   $state = 'C:\ProgramData\RustDesk\ManagementAgent\state\agent.json'
   Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" |
     Select-Object Name,State,StartMode,StartName,PathName,ProcessId | Format-List
   Get-Content $state -Raw -ErrorAction SilentlyContinue
   ```

## 安装/升级后核对

安装或升级完成后必须核对：

```powershell
$agent = 'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe'
$main = 'C:\Program Files\RustDesk\RustDesk.exe'
$state = 'C:\ProgramData\RustDesk\ManagementAgent\state\agent.json'

Get-FileHash $agent -Algorithm SHA256
Get-Item $main,$agent -Force | Select-Object FullName,Length,LastWriteTime
Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" |
  Select-Object Name,State,StartMode,StartName,PathName,ProcessId | Format-List
Get-Content $state -Raw
Test-Path 'C:\ProgramData\RustDesk\ManagementAgent\state\service-recovery-enabled'
```

必须满足：

- `RustDesk` 和 `RustDeskManagementAgent` 都是 `Running`；
- 两个服务均为 `LocalSystem`、`Auto`；
- agent binpath 含 `--service --edition standard --package-kind msi`；
- `agent_installation_id` 升级前后保持不变；
- `normal_install_path` 为实际主程序目录；
- agent SHA-256 与本次最终构建记录一致；
- `service-recovery-enabled` 存在；
- `last_error`、`service_recovery_error` 没有新的错误。

## 故障恢复矩阵

每个用例都要保存操作前、故障期间、恢复后的服务状态、agent.json、事件日志和 SHA-256。

### 1. 正常停止主服务

```powershell
Stop-Service RustDesk
```

等待最多 240 秒。断言：

- agent 服务仍为 Running；
- 主服务恢复为 Running；
- 主程序进程重新存在；
- `agent_installation_id` 不变；
- 没有启动未知路径程序。

### 2. 强制终止主程序进程

```powershell
Get-CimInstance Win32_Process -Filter "Name='RustDesk.exe'" |
  Where-Object { $_.ExecutablePath -eq 'C:\Program Files\RustDesk\RustDesk.exe' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
```

等待最多 240 秒，核对主程序进程重新出现，服务仍保持正常。

### 3. 修改为不存在的服务路径

先保存原始 binpath，再执行：

```powershell
sc.exe qc RustDesk
sc.exe config RustDesk binPath= "C:\RustDesk\missing\RustDesk.exe --service"
Stop-Service RustDesk -Force
```

断言：

- agent 不执行任意路径；
- 主程序实际文件存在时，服务 binpath 被修复到固定的 `C:\Program Files\RustDesk\RustDesk.exe --service`；
- 服务恢复后再核对 LocalSystem/Auto；
- 测试结束恢复原始 binpath。

### 4. 暂时移走主程序

```powershell
Stop-Service RustDesk -Force
Move-Item 'C:\Program Files\RustDesk\RustDesk.exe' 'C:\Program Files\RustDesk\RustDesk.exe.fault-backup'
```

如果 fallback 客户端存在：

```text
C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe
```

断言 agent 将服务切换到该固定 fallback 路径并启动。测试结束恢复主程序和原服务路径。

### 5. 主程序和 fallback 同时不可用

暂时移走两个文件，等待有界恢复周期：

```text
C:\Program Files\RustDesk\RustDesk.exe
C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe
```

断言：

- agent 自身仍保持 Running；
- 不执行未知程序；
- 不无限快速重试；
- `service_recovery_failures`、`service_recovery_next_at` 或 `last_error` 能反映失败；
- 网络可用时进入签名 manifest/包恢复流程；
- 网络隔离时保持有界失败并保留旧状态。

### 6. agent 自身重启

```powershell
Restart-Service RustDeskManagementAgent
```

断言 marker、身份、generation、服务契约保持，agent 重启后仍能观察主服务。

### 7. 物理机重启

```powershell
shutdown.exe /r /t 0
```

重启完成后再次核对双服务、agent hash、marker、agent.json 和主程序进程。

## 事件日志留证

每个故障用例执行后保存：

```powershell
Get-WinEvent -FilterHashtable @{LogName='System';ProviderName='Service Control Manager';StartTime=(Get-Date).AddMinutes(-15)} |
  Where-Object { $_.Message -like '*RustDesk*' } |
  Select-Object TimeCreated,Id,LevelDisplayName,Message |
  Format-List
```

同时保存：

- `C:\ProgramData\RustDesk\ManagementAgent\state\agent.json`；
- agent 日志目录；
- MSI 日志；
- 故障前后 `sc qc RustDesk` 和 `sc qc RustDeskManagementAgent`；
- 故障前后两个 EXE 的 SHA-256；
- 每个用例的开始/结束时间和恢复耗时。

## 通过标准

只有同时满足以下条件才算该用例通过：

- 主程序最终能启动并保持服务 Running；
- agent 始终保持可观察；
- 恢复使用固定、受信任路径；
- 主程序和 agent 身份没有被替换或重置；
- 失败时有界、可读、可恢复；
- 没有手工 `Start-Service`、`sc start` 或人工复制文件参与恢复动作；
- 测试结束后机器和配置恢复到干净状态。

不能仅凭 MSI exit code、服务短暂 Running 或主程序文件存在宣布通过。
