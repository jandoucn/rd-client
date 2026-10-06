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

MSI 服务参数使用了 `[INSTALLFOLDER_INNER]`。这个目录值末尾自带反斜杠，再包进引号后，Windows 命令行把结尾的 `\"` 当成转义引号。解析出来的 `normal_install_path` 最后一个字符是 `"`，不是目录分隔符。

用这个带引号的路径去找 `RustDesk.exe` 必然失败。普通安装还在，管理代理也会下载并重装 MSI。重装失败码是 `1603/1316`，过程中会停掉 `RustDesk` 服务和窗口。服务停着时，双击不会拉起 `--tray`。

## 修复

MSI 服务参数改为 `[ProgramFiles6432Folder]RustDesk`，不再使用末尾带反斜杠的目录值。解析服务参数时去掉首尾引号和末尾斜杠，避免已经写入服务配置的错误引号继续触发重装。
