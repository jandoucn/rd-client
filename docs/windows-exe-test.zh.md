# Windows EXE standard / SOS 真机测试

standard EXE 和 SOS EXE 分开测。两台机器可以并行，一台机器必须测完并清理干净再测另一个。两个 edition 共用 `RustDeskManagementAgent`，不能叠在同一台未清理的机器上。

本次对照包来自 Release `v1.5.3-build-2026.10.05-06`：

```text
rustdesk-1.5.3-2026100506-standard-windows-x86_64.exe
https://github.com/jandoucn/rustdesk/releases/download/v1.5.3-build-2026.10.05-06/rustdesk-1.5.3-2026100506-standard-windows-x86_64.exe

rustdesk-1.5.3-2026100506-sos-windows-x86_64.exe
https://github.com/jandoucn/rustdesk/releases/download/v1.5.3-build-2026.10.05-06/rustdesk-1.5.3-2026100506-sos-windows-x86_64.exe
```

用 64 位 Windows，管理员 PowerShell 操作。不要用 `2026100505` 或更早的包。EXE 不是 MSI，全程不要用 `msiexec /x`。

`2026100506` 这一版的 EXE 恢复已知会失败。卸载后客户端能消失，管理代理也还在，但恢复安装会停在 `last_error=EXE service installer exited with exit code: 1`，隐藏副本不会自动注册成服务。下面的通过标准按修复后的包执行；拿 `2026100506` 复测时，第 5 步出现这个错误就是复现当前缺陷，不要当成环境问题。

## 1. 安装前清理

把仓库里的 `scripts/clean-rustdesk-windows.bat` 拷到测试机后双击。脚本会自己请求管理员权限。它会卸掉 RustDesk，停掉并删除 `RustDesk`、`RustDesk Service`、`RustDeskManagementAgent`，再删除 `C:\Program Files\RustDesk`、`C:\ProgramData\RustDesk`、本机用户目录里的 RustDesk 数据、桌面和启动项快捷方式，以及卸载注册表项。

看脚本最后的核对输出。三个服务都应是 `SERVICE CLEAN`，上述目录的 `exists` 都是 `False`，`RustDesk uninstall entries left` 为 `0`。如果提示服务被锁住，先重启，再执行一次脚本，确认干净后再安装。

这一步只用于安装前和整轮测完。测卸载恢复时不要再跑这个脚本，它会把管理代理一起删掉。

## 2. 安装并核对服务

双击对应的 EXE，按普通安装完成。standard 和 SOS 各走各的机器，安装后立刻执行同一行：

```powershell
Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,StartMode,PathName | Format-List
```

`RustDesk` 和 `RustDeskManagementAgent` 都应是 `Running`，`StartMode` 都是 `Auto`。

standard 的管理代理 `PathName` 必须同时包含：

```text
C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe
--service
--edition standard
--package-kind exe
--normal-install-path
C:\Program Files\RustDesk
```

SOS 的管理代理 `PathName` 必须同时包含：

```text
C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe
--service
--edition sos
--package-kind exe
--normal-install-path
C:\Program Files\RustDesk
```

`edition` 或 `package-kind` 不对，这一轮就失败，不用继续等恢复。

## 3. 等服务循环写状态

装好后等 1 到 2 分钟，执行：

```powershell
Get-Content "$env:ProgramData\RustDesk\ManagementAgent\state\agent.json"
```

standard 至少核对这四项，而且它们必须互相一致：

```json
{
  "edition": "standard",
  "package_kind": "exe",
  "target_key": "windows-x86_64-exe-standard",
  "recovery_install_path": "C:\\ProgramData\\RustDesk\\Client\\standard\\RustDesk"
}
```

SOS 对应四项必须是：

```json
{
  "edition": "sos",
  "package_kind": "exe",
  "target_key": "windows-x86_64-exe-sos",
  "recovery_install_path": "C:\\ProgramData\\RustDesk\\Client\\sos\\RustDesk"
}
```

`package_kind` 不能是 `msi`。`target_key` 不能串到另一个 edition。`last_error` 应为空。再等一两分钟看一次，服务循环不能把这四项改掉。

## 4. 卸载客户端，保留管理代理

应用和功能里如果没有可用的删除入口，用 EXE 自己的卸载注册表。不要查 `WindowsInstaller=1`，那是 MSI。执行这一行：

```powershell
$p=Get-ChildItem 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue|ForEach-Object{Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue}|Where-Object{$_.DisplayName -eq 'RustDesk' -and $_.WindowsInstaller -ne 1 -and $_.UninstallString}|Select-Object -First 1;if(!$p){throw '未找到 RustDesk EXE'};$cmd=$p.UninstallString.Trim();if($cmd.StartsWith('"')){$exe=$cmd.Split('"')[1];$arg=$cmd.Substring($cmd.IndexOf('"',1)+1).Trim()}else{$sp=$cmd.IndexOf(' ');$exe=$cmd.Substring(0,$sp);$arg=$cmd.Substring($sp+1).Trim()};$r=Start-Process -FilePath $exe -ArgumentList $arg -Verb RunAs -Wait -PassThru;Write-Host "EXE uninstall exit=$($r.ExitCode)"
```

`EXE uninstall exit` 必须是 `0`。退出码不是 0 就停止，不要过 5 分钟再补卸一次。客户端已经不在时再卸会返回 `1`，那个 `1` 没有诊断价值。

卸完后执行：

```powershell
Get-Item 'C:\Program Files\RustDesk\RustDesk.exe',"$env:ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe","$env:ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" -ErrorAction SilentlyContinue|Select-Object FullName,Length,LastWriteTime;Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,StartMode,PathName|Format-List
```

通过这一步的标志：

- `C:\Program Files\RustDesk\RustDesk.exe` 不存在。
- 当前 edition 的隐藏副本这时还不该存在。standard 看 `Client\standard\RustDesk\RustDesk.exe`，SOS 看 `Client\sos\RustDesk\RustDesk.exe`。
- `RustDesk` 服务不存在。
- `RustDeskManagementAgent` 仍是 `Running`，`PathName` 里的 `--edition` 和 `--package-kind exe` 跟第 2 步一致。
- `C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe` 还在。

## 5. 等自动恢复

不要手动执行 `check-now`，不要手建 `RustDesk` 服务，也不要再跑清理脚本。服务第一次自动检查排在约 5 分钟后，循环间隔约 30 秒，所以从卸载完成起至少等 6 分钟，最长等到 8 分钟。

每一分钟执行一次：

```powershell
Get-Item 'C:\Program Files\RustDesk\RustDesk.exe',"$env:ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe","$env:ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe" -ErrorAction SilentlyContinue|Select-Object FullName,Length,LastWriteTime;Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'"|Select-Object Name,State,StartMode,PathName|Format-List;Get-Content "$env:ProgramData\RustDesk\ManagementAgent\state\agent.json"
```

standard 通过的标志是这几件事同时成立：

- `C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe` 存在。
- `C:\Program Files\RustDesk\RustDesk.exe` 没有被装回去。
- `C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe` 不存在。
- `RustDesk` 服务是 `Running`、`Auto`，`PathName` 是 `"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe" --service`。
- `RustDeskManagementAgent` 仍是 `Running`，`PathName` 仍带 `--edition standard --package-kind exe`。
- `agent.json` 里 `edition` 仍是 `standard`，`package_kind` 仍是 `exe`，`target_key` 仍是 `windows-x86_64-exe-standard`，`last_error` 为空，`retry_stage` 为 `0`。
- `last_primary_url` 以 `.exe` 结尾，不能是 `.msi`。

SOS 通过的标志把上面的 `standard` 换成 `sos`，`target_key` 必须是 `windows-x86_64-exe-sos`，服务路径必须是 `C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe`。另一个 edition 的隐藏副本不能出现。

失败时不要手建服务后再把这一轮判成通过。把下面三样留下来：

- 第 5 步那一行的完整输出。
- `C:\ProgramData\RustDesk\Client\<edition>\` 下面是否留下 `RustDesk-download-*` 目录。
- 事件查看器里 `RustDesk`、`RustDeskManagementAgent` 相关的 Application 事件。

`2026100506` 的已知失败形态是：`RustDesk` 服务不存在，`retry_stage` 递增，`last_error` 为 `EXE service installer exited with exit code: 1`，隐藏目录下可能留下 `RustDesk-download-<pid>`。这个目录里已经有小写的 `rustdesk.exe`，说明解包完成了，失败发生在安装脚本拿退出码和注册服务的阶段。

## 6. 测完再清理

一个 edition 通过后，再双击 `scripts/clean-rustdesk-windows.bat`。确认服务和目录都干净，然后才能在同一台机器上装另一个 edition。
