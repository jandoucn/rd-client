# Windows 正常安装被误判为缺失

## 现象

新装的 standard MSI 双击后主窗口能打开，但没有托盘图标。窗口有时自己关掉。两台机器都出现。

## 现场

- 桌面快捷方式目标是 `C:\Program Files\RustDesk\RustDesk.exe`，参数为空。
- `RustDesk` 服务当时是 `Stopped`。
- 双击后只有不带参数的 `RustDesk.exe`，没有 `--service`、`--server`、`--tray`。
- 服务手动启动后出现 `--service` 和 `--server`，仍没有自动出现 `--tray`。
- `agent.json` 的 `normal_install_path` 是 `C:\Program Files\RustDesk\`，末尾有反斜杠。
- `last_error` 是 `msiexec exited with exit code: 1603`。
- 事件日志是 `1316 指定的账户已存在`。安装包是 `C:\ProgramData\RustDesk\ManagementAgent\cache\rustdesk-standard-latest.msi`。

## 原因

`normal_client_present` 直接把 `normal_install_path` 和 `RustDesk.exe` 拼起来。路径末尾有反斜杠时，检查路径变成 `C:\Program Files\RustDesk\\RustDesk.exe`。文件实际存在，但判断结果是不存在。

管理代理因此在普通安装还在时下载并重装 MSI。重装失败码是 `1603/1316`，过程中会停掉 `RustDesk` 服务和窗口。服务停着时，双击不会拉起 `--tray`。

## 修复

判断正常安装前去掉路径首尾空白和末尾斜杠。MSI 服务参数里的末尾反斜杠不能再让正常客户端被当成缺失。
