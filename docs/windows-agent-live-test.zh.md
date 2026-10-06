# Windows 管理代理真机测试命令

一台 64 位 Windows，管理员 PowerShell。standard 和 SOS、MSI 和 EXE 共用 `RustDeskManagementAgent`，同一台机器必须测完并清理干净再测下一个。不要手动执行 `check-now`。

## 清理

只用于安装前和整轮测完。测卸载恢复时不要再跑。

```powershell
Start-Process -FilePath "D:\github\rustdesk\scripts\clean-rustdesk-windows.bat" -ArgumentList "_elevated" -Wait
```

核对：

```powershell
foreach ($n in @('RustDesk','RustDesk Service','RustDeskManagementAgent')) { $svc = Get-CimInstance Win32_Service -Filter ("Name='{0}'" -f $n) -ErrorAction SilentlyContinue; if ($svc) { "STILL $n $($svc.State)" } else { "SERVICE CLEAN: $n" } }; "pd=$(Test-Path 'C:\ProgramData\RustDesk') pf=$(Test-Path 'C:\Program Files\RustDesk')"
```

三个服务都是 `SERVICE CLEAN`，两个目录都不存在，才能安装。

## 安装

MSI：

```powershell
$msi = "C:\Users\Administrator\Downloads\rustdesk-1.5.3-2026100510-standard-windows-x86_64.msi"
Start-Process msiexec.exe -ArgumentList @('/i', $msi, '/qn', '/norestart') -Wait
```

EXE：

```powershell
$exe = "C:\Users\Administrator\Downloads\rustdesk-1.5.3-2026100510-standard-windows-x86_64.exe"
Start-Process -FilePath $exe -ArgumentList "--silent-install" -Wait
```

SOS 把文件名里的 `standard` 换成 `sos`。

安装后立刻核对：

```powershell
Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,StartMode,PathName | Format-List; "pf=$(Test-Path 'C:\Program Files\RustDesk\RustDesk.exe') desktop=$(Test-Path "$env:PUBLIC\Desktop\RustDesk.lnk")"; Get-Content "$env:ProgramData\RustDesk\ManagementAgent\state\agent.json"
```

通过要求：

- 两个服务都是 `Running`，`StartMode` 是 `Auto`。
- `PathName` 里的 `--edition` 和 `--package-kind` 与当前包一致。
- `--normal-install-path` 是 `C:\Program Files\RustDesk`。末尾不能有反斜杠，路径里不能有多余引号。
- `C:\Program Files\RustDesk\RustDesk.exe` 存在。
- `agent.json` 的 `edition`、`package_kind`、`target_key`、`recovery_install_path` 互相一致，`last_error` 为空。
- 再等一两分钟，这四项不能被服务循环改掉。

## 卸载并保留管理代理

MSI：

```powershell
$item = Get-ItemProperty HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*, HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\* | Where-Object { $_.DisplayName -like '*RustDesk*' -and $_.WindowsInstaller -eq 1 } | Select-Object -First 1; if (-not $item) { throw '未找到 MSI' }; $p = Start-Process msiexec.exe -ArgumentList @('/x', $item.PSChildName, '/qn', '/norestart') -Wait -PassThru; "exit=$($p.ExitCode)"
```

EXE：

```powershell
$p = Get-ChildItem 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue | ForEach-Object { Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue } | Where-Object { $_.DisplayName -eq 'RustDesk' -and $_.WindowsInstaller -ne 1 -and $_.UninstallString } | Select-Object -First 1; if (!$p) { throw '未找到 RustDesk EXE' }; $cmd = $p.UninstallString.Trim(); if ($cmd.StartsWith('"')) { $exe = $cmd.Split('"')[1]; $arg = $cmd.Substring($cmd.IndexOf('"',1)+1).Trim() } else { $sp = $cmd.IndexOf(' '); $exe = $cmd.Substring(0,$sp); $arg = $cmd.Substring($sp+1).Trim() }; $r = Start-Process -FilePath $exe -ArgumentList $arg -Wait -PassThru; "EXE uninstall exit=$($r.ExitCode)"
```

EXE 卸载退出码可能是 `1`。只要 `C:\Program Files\RustDesk` 已消失，就不要再卸一次。

卸载后核对：

```powershell
"pf=$(Test-Path 'C:\Program Files\RustDesk\RustDesk.exe') standard=$(Test-Path "$env:ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe") sos=$(Test-Path "$env:ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe") agent=$(Test-Path "$env:ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe")"; Get-CimInstance Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Select-Object Name,State,PathName | Format-List
```

通过要求：普通目录不在，当前 edition 的隐藏副本这时还不在，`RustDesk` 服务不在，管理代理仍是 `Running`。

## 等自动恢复

从卸载完成起至少等 6 分钟，最长 8 分钟。每 30 秒看一次：

```powershell
$state = "$env:ProgramData\RustDesk\ManagementAgent\state\agent.json"; $j = Get-Content -LiteralPath $state -Raw | ConvertFrom-Json; "exe_standard=$(Test-Path "$env:ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe") exe_sos=$(Test-Path "$env:ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe") pf=$(Test-Path 'C:\Program Files\RustDesk\RustDesk.exe')"; Get-Service RustDesk,RustDeskManagementAgent -ErrorAction SilentlyContinue | Select-Object Name,Status; "stage=$($j.retry_stage) err='$($j.last_error)' url='$($j.last_primary_url)'"
```

通过要求：

- 当前 edition 的隐藏 `RustDesk.exe` 存在，另一个 edition 的隐藏副本不存在。
- `C:\Program Files\RustDesk\RustDesk.exe` 没有被装回去。
- `RustDesk` 服务是 `Running`，路径指向对应隐藏目录。
- `RustDeskManagementAgent` 仍是 `Running`，`--edition` 和 `--package-kind` 没变。
- `agent.json` 身份没变，`retry_stage` 为 `0`，`last_error` 为空。
- `last_primary_url` 以当前包的 `.msi` 或 `.exe` 结尾。
- `C:\ProgramData\RustDesk\ManagementAgent\cache` 里不再留下 `rustdesk-<edition>-latest.msi` 或 `.exe`。

失败时留下 `agent.json`、`sc qc RustDeskManagementAgent`、`sc qc RustDesk`，以及 `C:\ProgramData\RustDesk\Client` 下是否有 `RustDesk-download-*`。

## 2026100510 结果

四个包都通过。安装路径没有多余引号。卸载后管理代理都自己下载并完成恢复，下载缓存随后删除。

- standard MSI：服务路径无多余引号，安装后已有 `--service`、`--server`、`--tray`，两分半内没有误重装。
- standard EXE：约 330 秒恢复，`last_primary_url` 以 `.exe` 结尾。
- SOS EXE：约 330 秒恢复，服务路径是 `C:\ProgramData\RustDesk\Client\sos\RustDesk\RustDesk.exe`。
- SOS MSI：约 330 秒恢复，`last_primary_url` 以 `.msi` 结尾。
