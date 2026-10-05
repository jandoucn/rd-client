# Windows MSI standard 真机测试

先测这一包，通过后再测 SOS MSI、standard EXE、SOS EXE。四个包都来自 Release `v1.5.3-build-2026.10.05-06`。

本次要装的文件：

```text
rustdesk-1.5.3-2026100506-standard-windows-x86_64.msi
https://github.com/jandoucn/rustdesk/releases/download/v1.5.3-build-2026.10.05-06/rustdesk-1.5.3-2026100506-standard-windows-x86_64.msi
```

用一台 64 位 Windows，用管理员账户操作。不要和 SOS 或 EXE 装在同一台未清理的机器上，它们共用 `RustDeskManagementAgent`。不要用 `2026100505` 或更早的包。

## 1. 安装前清理

把仓库里的 `scripts/clean-rustdesk-windows.bat` 拷到测试机后双击。脚本会自己请求管理员权限。它会卸掉所有 RustDesk MSI，停掉并删除 `RustDesk`、`RustDesk Service`、`RustDeskManagementAgent`，再删除 `C:\Program Files\RustDesk`、`C:\ProgramData\RustDesk`、本机用户目录里的 RustDesk 数据、桌面和启动项快捷方式，以及卸载注册表项。

看脚本最后的核对输出。三个服务都应是 `SERVICE CLEAN`，上述目录的 `exists` 都是 `False`，`RustDesk uninstall entries left` 为 `0`。如果提示服务被锁住，先重启，再执行一次脚本，确认干净后再安装。

这一步只用于安装前和整轮测完。测卸载恢复时不要再跑这个脚本，它会把管理代理一起删掉。

## 2. 安装并核对服务

双击 MSI，按普通安装完成。安装后立刻执行：

```bat
sc qc RustDeskManagementAgent
sc query RustDesk
sc query RustDeskManagementAgent
```

`RustDesk` 和 `RustDeskManagementAgent` 都应是 `RUNNING`。管理代理的 `BINARY_PATH_NAME` 必须同时包含：

```text
C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe
--service
--edition standard
--package-kind msi
--normal-install-path
C:\Program Files\RustDesk
```

`edition` 不是 `standard`，或 `package-kind` 不是 `msi`，这一轮就失败，不用继续等恢复。

## 3. 等服务循环写状态

装好后等 1 到 2 分钟，打开：

```text
C:\ProgramData\RustDesk\ManagementAgent\state\agent.json
```

至少核对这四项，而且它们必须互相一致：

```json
{
  "edition": "standard",
  "package_kind": "msi",
  "target_key": "windows-x86_64-msi-standard",
  "recovery_install_path": "C:\\ProgramData\\RustDesk\\Client\\standard\\RustDesk"
}
```

`target_key` 不能变成 `windows-x86_64-exe-standard`，也不能带 `sos`。`last_error` 应为空。再等一两分钟看一次，服务循环不能把这四项改掉。

## 4. 卸载客户端，保留管理代理

应用和功能里的删除按钮被关掉了，所以用产品码卸载。先查出产品码：

```powershell
Get-ItemProperty HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*, HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\* |
  Where-Object { $_.DisplayName -like '*RustDesk*' } |
  Select-Object DisplayName, PSChildName
```

对查到的 `{...}` 执行：

```bat
msiexec /x {产品码} /qn /norestart
```

卸完后确认：

- `C:\Program Files\RustDesk` 已经不在。
- `RustDesk` 服务已经不在。
- `RustDeskManagementAgent` 仍在，并且是 `RUNNING`。
- `C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe` 还在。
- `C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe` 这时还不该出现。

## 5. 等自动恢复

不要手动执行 `check-now`，也不要再跑清理脚本。服务第一次自动检查排在约 5 分钟后，循环间隔约 30 秒，所以从卸载完成起至少等 6 分钟。

通过的标志是这几件事同时成立：

- `sc query RustDesk` 显示 `RUNNING`。
- `C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe` 存在。
- `sc query RustDeskManagementAgent` 仍显示 `RUNNING`。管理代理在恢复过程中会短暂停掉自己，但最终必须自己回来。
- `agent.json` 里 `edition` 仍是 `standard`，`package_kind` 仍是 `msi`，`target_key` 仍是 `windows-x86_64-msi-standard`，`last_error` 为空。

超过 8 分钟仍没有 `RustDesk.exe`，或 `last_error` 有内容，把 `agent.json`、`sc qc RustDeskManagementAgent` 和 `sc qc RustDesk` 的输出留下来。这一轮不要接着测 SOS。

## 6. 测完再清理

standard MSI 通过后，再双击 `scripts/clean-rustdesk-windows.bat`。确认服务和目录都干净，然后才能装下一个包。
