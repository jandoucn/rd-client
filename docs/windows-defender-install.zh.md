# Windows 安装/升级的 Defender 文件排除项

状态：实现中，mock 逻辑和入口检查通过；完整安装包、Flutter/桌面和真实 Defender 写入尚未验收。用户明确禁止在当前开发机测试真实 Defender；不得在此机器启用 Defender 或读取/修改其实际设置来消除阻塞。

## 用户选择

首次 EXE 安装界面增加“为 RustDesk 配置 Defender 文件排除项”，默认不选。MSI 的安装目录页有同类明确选项。选择写入 64 位 HKLM `SOFTWARE\RustDesk\Defender\Consent`，值 `1` 表示允许、`0` 表示拒绝。

升级使用已保存选择，没有记录就不添加。读取选择失败按 `consent_failed` 处理，不尝试 Defender 写入。撤销勾选仅保存拒绝，不删除既有排除项；后续升级不再补充。

Defender 功能通过 `Get-MpPreference`/`Add-MpPreference` 是否可用及读回是否成功判定，不只看 Windows 版本。本功能目标系统为 Windows 10/11，两系统真实写入均须在隔离可恢复 VM 单独验收。

## 静默安装

EXE：

```text
RustDesk安装包.exe --silent-install defender=1
RustDesk安装包.exe --silent-install defender=0
RustDesk安装包.exe --silent-install
```

`defender=1` 明确同意，`defender=0` 明确拒绝；无参数沿用记录，全新机器无记录不写。冲突/非法值拒绝进入安装。自定义路径由现有安装 UI/安装逻辑确定，本功能不增加隐式路径转换。

MSI：

```text
msiexec /i 客户端.msi /qn /norestart DEFENDER_EXCLUSIONS=1
msiexec /i 客户端.msi /qn /norestart DEFENDER_EXCLUSIONS=0
msiexec /i 客户端.msi /qn /norestart
```

MSI 缺省 `inherit`，保存选择优先于没有明确参数的升级请求。交互安装时用户在目录页面的选择成为本次明确选择。非法值不执行配置并记录错误；不把配置错误当作主产品安装失败。

同机 standard/SOS 当前共用 agent 和同一 Defender 选择，不能并行安装。旧版本没有 Consent 时，新静默升级不会替用户同意。恢复安装使用专用标记跳过配置，普通升级仍遵循 Consent。

## 精确范围与顺序

精确 EXE 排除项仍是两项：安装器确定的最终主程序 exe，以及实际 agent 目录中的 `rustdesk-management-agent.exe`。另加三个目录排除项：最终安装目录、`ManagementAgent`、`Client`。支持空格、Unicode 路径。不排除磁盘根、进程、DLL、安装器或整个 ProgramData。

| 路径 | 执行时点 | 实际保护范围 |
| --- | --- | --- |
| EXE 首装 | 现有提权 runner 中，正常卸载处理后、最终目录复制前 | 最终主/agent EXE；下载/解包的安装器此前已启动，无法追溯保护 |
| EXE 正常升级 | 新目标包 `--update` 提权 runner，服务停止/文件替换前 | 最终主/agent EXE，已下载包及解包目录不在范围 |
| MSI 首装/升级/修复 | deferred、Impersonate=no、InstallFiles 前 | 已计算目标路径；MSI 本身及已解压的 CA DLL 不在范围 |
| agent 保底安装 | `RUSTDESK_AGENT_RECOVERY=1` 跳过 MSI 排除配置；EXE 保底提取不走普通安装器 | 本版不自动添加保底客户端路径，也不改变保存的普通安装选择 |

执行逻辑为 `res/defender-exclusions.ps1`，Rust EXE 安装器和 MSI CA 编译嵌入同一份内容，不从网络获取脚本。EXE 在受保护 BAT 旁分段解码临时文件，避免 CMD 单行长度限制；MSI 通过原生 CreateProcess 启动 Windows PowerShell，不经 CMD 解析用户路径。

Defender 非成功结果不阻止主安装/升级；不改原文件替换、回滚或服务处理策略。不得因配置失败主动卸载旧版本。完整失败事务是否保留旧版本，仍须以实际安装包回归证明。

## 状态、记录和卸载

| 状态 | 含义 |
| --- | --- |
| not_selected | 未授权或明确拒绝，不调用 Defender 读写 |
| configuring | 本轮正在执行；若 helper 超时/退出未结束，不能沿用上次 verified |
| consent_failed | 路径/选择/保存选择错误，无 Defender 写入 |
| unavailable | 官方 cmdlet 不可用 |
| read_failed | 初次配置或安装器拥有项记录读取失败，未写 Defender |
| failed | 添加被拒绝或调用失败 |
| unverified | 写命令返回后未读到配置，或读回不可用 |
| partial | 两项中仅一项已核验 |
| verified | 两个精确 EXE、三个目录，以及已启用时的两个 CFA 允许项都已读回确认，不意味着永不拦截 |

结果含精确路径、逐项状态、Added、错误及时间，保存到同一注册表的 `LastResult`。`OwnedPaths` 仅记录安装器成功添加且读回确认的项；原先已有项不认领。拥有项/结果保存错误独立记录，不伪造 Defender 写入失败或成功。

受控文件夹访问仅在 `EnableControlledFolderAccess` 为 1、2、3、4 时，为上述两个 EXE 添加允许应用。未启用时记录跳过，不启用该功能，也不把目录加入受保护文件夹。

防火墙沿用现有安装规则。EXE 在 agent 文件复制后补 `RustDesk Management Agent` 入站和出站规则；MSI 现有 `AddFirewallRules` 同时核对主程序和 agent。Defender 脚本只读回 `netsh` 规则，不新增第三套防火墙写入。规则缺失记为 `unverified`，不阻断安装。

当前卸载不调用 `Remove-MpPreference`，保留用户原有项及安装器项；即使已记录 OwnedPaths，也不自动删除，因为之后用户/组织可能依赖相同项。若未来增加清理，必须单独明确确认及来源核对。新增注册表设置不属于现有产品卸载键，升级/卸载不应隐式清掉用户选择。

EXE 在提权 runner 输出结果/告警；MSI 日志记录 helper 退出状态与结果位置。当前不增加“360 已加白”“绝不会查杀”等成功提示。

## 与 DTF 故障的关系

排除项不能恢复隔离文件，不能启动服务，也不影响 360。主程序和 agent 都 stopped 时仍需独立 bootstrap；如果启动器也被拦截，不能承诺本机自动恢复。

独立 BAT `scripts/configure-rustdesk-trust.bat` 属于显式运维工具，不作为 agent 自动动作。本版安装集成不调用它，因为它默认路径/退出码和用户选择语义与安装需求不同；采用共享 PS 逻辑满足真实目录与持久化选择。

## 测试与尚未通过门禁

已提供：

- `tests/test_defender_installer.py`：真实执行 PS 函数，所有 Defender/注册表 IO 替换为 mock，含 16 种选择、幂等、拒绝、读写失败/部分成功及路径拒绝。
- `tests/defender_options.rs`：明确参数、缺省继承、冲突/非法参数拒绝。
- `windows_defender.rs` 的单测：脚本内容、Unicode/单引号路径、CMD 每行长度和不修改安全防护。
- `flutter/test/defender_install_option_test.dart`：默认关闭、保存选择、交互、提示与 disabled。

运行 mock：

```text
python -m unittest discover -s tests -p test_defender_installer.py -v
flutter test test/defender_install_option_test.dart
```

不得在本机执行真实写入。Flutter SDK、MSVC/WiX、全仓 Cargo 依赖、实际四包安装/升级和隔离 Windows 10/11 写入/UAC 验收当前未完成；源码字符串入口检查不能替代 MSI 编译或包测试。实现不能在这些门禁完成前宣布可发布。
