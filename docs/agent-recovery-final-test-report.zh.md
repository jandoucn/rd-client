# Management Agent 最终测试报告

测试日期：2026-10-10（后续运行跨日时，以证据内 UTC 时间为准）

报告性质：当前候选的测试结果与交付缺口，未发布。
验收依据：[完整升级方案](agent-recovery-upgrade-plan.zh.md)、[全量测试手册](agent-recovery-test-runbook.zh.md)。

## 1. 结论

**当前候选尚未通过完整保护验收，不能标记为“异地可用”或“全部完成”。**

2026-10-12 后续事实以 [VM SYSTEM、实包回滚和接续检查点](agent-vm-system-driver.zh.md) 为准：两系统本地签名API/双包HTTP/真实standard MSI跨build升级、失败回滚、bootstrap同步和原身份保留通过；进程退出后补报仅一次，普通MSI恢复入口初始化/升级/回滚也已验证。v158各30目标224项通过，11项ignored未计入；v164终态接续修复的helper完整15项另通过。全workspace测试仅编译成功，格式和严格Clippy仍失败；四包迁移、真正断电接续、独立agent自更新、完整离线恢复阶梯与真实远控尚未全部验收。以下旧文中的权限/工具阻塞、仅inspect、未接bootstrap和未具备本地命令安装链等均是历史阶段，不能覆盖本段及VM证据。

v173控制台1440/390确认/取消/能力门禁三条浏览器测试通过；v168/v169两系统额外71项本地workspace测试（1 ignored）和38项更新/安装/IPC相关测试通过。v186–v196两系统SOS MSI安装切换、重复安装、周期恢复和干净全新安装均通过，完整证据和健康快照已保留。开机首次恢复仍有PT1M超时及触发归因缺口，未据不充分对照修改生产超时；SOS旧版跨build回滚和EXE矩阵仍未完成。不要将这些阶段结果与重复测试数量相加成独立功能覆盖率。

Windows 10 LTSC 2019 和 Windows 11 LTSC 2024 测试环境、干净快照、客体控制通道均已建立。实际 standard MSI 安装和 Debug MSI 替换 agent 已验证；低层 agent、SQLite、HTTP 和后台页面回归通过。实际 VM 测试发现冷启动服务超时、停止服务重装删除注册、bootstrap 任务读回不兼容和服务命令等价表示不匹配。停止服务注册保留已修复并在两系统验证；bootstrap手工准备和R1显式测试门禁在Win10通过，详见第10节。

独立任务尚未自动随完整包部署，R1 默认门禁未由安装器启用，R2–R5 执行器、完整安装事务/helper/接续及四包在线迁移仍未完成。不能把手工运行新 MSI 等同于“后台命令→旧程序下载→新包→新 agent”的一次在线升级。

| 发布判定 | 当前结果 | 原因 |
| --- | --- | --- |
| 候选构建通过 | 部分构建 PASS，整体 BLOCKED | agent、Flutter Windows Debug、MSI fixture 能构建；严格全仓门禁未绿，无完整 Release 四包 |
| 实际包验收通过 | FAIL | 仅覆盖 standard MSI 局部；冷启动和完整恢复存在失败，其余包矩阵未完成 |
| 异地完整保护可发布 | FAIL | 双服务正常停止恢复入口未随包就绪；事务/离线资产/完整迁移未验收 |

360：用户授权测试后明确要求“不能后台自动加就没必要测试了”。个人版受支持的后台静默信任接口尚未确认；界面测试已停止，两个文件未加入信任，未实现或接入自动加白。Win10 已恢复到安装360前快照，宿主未安装360。

## 2. 源码与产物身份

| 对象 | 身份 |
| --- | --- |
| 客户端 HEAD | `80df2807659d3a8aa26f9ede976b3f298f5cff68`，`master`，含本地未提交改动 |
| 后台 HEAD | `d065b96498008784a1bc46146b3c4f8196350553`，`main`，含本地未提交改动 |
| 版本元数据 | `version.json`：product=rustdesk-yan、version=1.5.3、build_seq=2026100502 |
| 旧 standard MSI | `rustdesk-1.5.3-2026100506-standard-windows-x86_64.msi`，SHA-256 `e6561c6419701afbb63fddb197b0d92c825ba7835f354bab1bbde48b688e92d4` |
| 初次新 Debug MSI | SHA-256 `a668214798e6eb4799325850798463c6ee2a0f9b6e1da38bc49e5dc70b56f982` |
| 停止服务保留修复 MSI | SHA-256 `165a92846d122868e714ef7949396beabe67cf5d248184d135500bc4f2f65a92` |
| 旧 agent | SHA-256 `02ec919f1a0cea4c282d9a05b5dec22ba2708c79915ac798b287690c57389010` |
| 初次 Debug MSI 内 agent | SHA-256 `38e85e278ffd0c471c96f50ebd97456869b7543db2b84f023901787b3c1552bc` |
| 最新单独Debug agent | SHA-256 `618451fba5b2c5813cd051dd76c0b29b1d21e9c459cb52df39264ceb218e0735`，仅手工bootstrap/R1 fixture部署 |

旧包文件名不作为内部构建身份的充分证据：实测主 EXE 文件版本为 `1.5.3+2026100502`；新 Debug EXE 也沿用这一资源版本，agent 文件版本资源为空。完整可发布候选必须重新绑定源码、构建序号、各组件版本和包摘要。

修复 MSI 与后来单独构建的 bootstrap agent 是不同产物，不能混称同一最终包。原 portable 的 `data.bin` 来自旧已安装程序，只用于此前源码构建门禁，未作为新 EXE 候选验收。

## 3. 环境与隔离

| 项目 | Windows 10 VM | Windows 11 VM |
| --- | --- | --- |
| 名称 | RustDesk-Agent-Win10 | RustDesk-Agent-Win11 |
| 实际版本 | 10.0.17763.316，Enterprise LTSC 2019 | 10.0.26100.1742，Enterprise LTSC 2024 |
| ISO | 用户指定 cn_windows_10_enterprise_ltsc_2019_x64_dvd_9c09ff24.iso | zh-cn_windows_11_enterprise_ltsc_2024_x64_dvd_cff9cd2d.iso |
| VM UUID | a6c9edff-6159-468f-bdba-08720a153424 | d91a13d9-2bb7-40bf-b5e4-9d57870cd242 |
| 资源 | 4核、4GB、80GB动态VDI、BIOS | 4核、6GB、80GB动态VDI、EFI/TPM2 |
| 干净快照 | CleanOS-LTSC2019-Offline，82889123-f059-43ca-ac6a-3b28c85b4361 | CleanOS-DiskBoot-Offline，bccf5f7c-3623-436b-9865-b6ad525f8ddb |
| 升级后、360前快照 | 37e29878-7539-427f-9533-4bd6c8ef87fc | f4711c1e-856d-4745-98c6-1cd2964aaae7 |

VirtualBox 7.2.20r175154，Guest Additions 同版本；NAT，无共享剪贴板/拖放，安装完成后卸下DVD并验证硬盘冷启动。客体hosts阻断生产API、下载和ID域名；客户端测试主要离线进行。SYSTEM驱动任务只在RDLAB-*客体运行，保留UAC。

宿主 Windows 10 LTSC 17763 的 RustDesk 与 RustDeskManagementAgent 全程作为现有控制链保留；未安装测试MSI、未停删宿主服务、未真实访问或更改宿主Defender配置。Defender脚本测试使用mock，安装fixture显式选择不添加排除项。MySQL冻结，未运行MySQL门禁。

## 4. 源码、集成及界面回归

| 测试组 | 实际结果 | 证据及边界 |
| --- | --- | --- |
| agent真实Rust回归 | PASS，31+9+2+5+10=57项 | agent-bootstrap-binpath-green，exit0；覆盖最新源码，非stub |
| bootstrap引号/默认值/路径新增回归 | PASS，计入上述57项 | 红阶段实际失败；含唯一禁用任务注册/读回/删除，未运行宿主生产恢复任务 |
| 独立底座 | 历史PASS：recovery11/bootstrap4/consent2/guard2 | 低层逻辑，不代替包/服务E2E |
| PHP commands/授权/事件/包解析 | PASS，65个SQLite断言 | agent-report-api-sqlite，直接核对持久化与失败旧数据保留 |
| PHP状态投影 | PASS，30个SQLite断言 | agent-report-status-sqlite |
| HTTP协议 | PASS，12项 | agent-report-api-http；签名、拒绝、旧实例、状态幂等、数据库重启 |
| 后台浏览器E2E | PASS | agent-report-console-e2e，25个执行项含继承的重复HTTP用例；独立浏览器流程1项×1440/390，不能报25个独立UI能力 |
| Rust→PHP→SQLite | PASS，计入agent单测 | 真实签名/验签、分页、claim、事件、补报、本地journal及SQL；仅inspect执行器 |
| Defender脚本 | PASS，6项、历史19个子场景 | 实际PowerShell逻辑mock；不代表真实Defender加白验收 |
| 原MSI契约 | PASS，12项 | 源码契约，非实际包验收 |
| 新停止服务保留契约 | PASS，1项含2个子检查 | test_msi_stopped_service_preservation；先红后绿，两系统实际回归见下节 |
| Flutter widget/全量 | 历史PASS，172项 | agent-flutter-full-tests-ready；未替代真实桌面用户流程 |
| Windows Debug桌面启动 | PASS，Win10 | 实际安装主程序启动首页，OCR识别首页/连接入口；未验证远控连接与所有导航 |

## 5. 实际包与故障验收

| 用例 | Win10 | Win11 | 说明 |
| --- | --- | --- | --- |
| 空系统安装旧standard MSI | PASS | 首装exit0，重装采集PASS | Win11初次采集隐藏agent时失败，修正采集器后重装，不伪作首装前快照 |
| 旧MSI→初次新Debug MSI | PASS（安装/替换子项） | PASS（安装/替换子项） | exit0、双服务当时Running/LocalSystem、agent预期hash；手工安装，非在线迁移 |
| 新包冷启动主服务 | FAIL | FAIL | 主服务Stopped，agentRunning；Win10 SCM7000/7009，30秒连接超时；根因未最终确认 |
| 同包重装身份字段保留 | 身份子项PASS | 身份子项PASS | enc_id/key_pair字段摘要和installation_id一致；原综合重装仍失败，不能因身份相同标整项PASS |
| 原包停止主服务后重装 | FAIL | FAIL | 删除服务后CreateStartService被STOP_SERVICE='Y'抑制，MSI仍返回0 |
| 修复包停止主服务后重装 | PASS | PASS | 服务仍Stopped、注册/路径/LocalSystem保留；agentRunning；人工启动恢复成功 |
| 损坏MSI拒绝 | 有效前置复测PASS | 原综合FAIL，未补有效前置复测 | Win10复测退出1620、文件摘要保留、双服务仍Running；原前序重装污染证据保留；不是事务中途回滚测试 |
| 双服务正常停止DTF | FAIL | 未执行，BLOCKED | Win10前置双Running，停止后45秒仍双Stopped，无产品bootstrap任务；人工恢复成功 |
| 手工部署bootstrap/幂等/启动agent | 修复后PASS | BLOCKED | 手工准备不代替安装器自动部署或完整DTF；详见第10节 |
| 显式启用R1恢复门禁 | PASS | BLOCKED | Win10停止主服务后约16秒恢复Running；门禁撤回，不代表随包默认启用 |
| 安装中断/断电/helper接续 | BLOCKED | BLOCKED | 当前transaction未接生产执行器，不能用bool证据或VM快照恢复冒充产品回滚 |
| 独立控制端实际远控 | BLOCKED | BLOCKED | 当前未完成隔离控制端连接、画面、输入/UAC/文件传输验收 |

四包×两路径×两OS共16格矩阵：当前仅standard MSI两系统的旧包安装和手工目标包替换有局部证据；standard EXE、SOS EXE、SOS MSI、真实旧程序在线下载迁移均BLOCKED。不能生成“16格全PASS”计数。

## 6. 严格门禁与覆盖率

| 门禁 | 结果 | 具体原因 |
| --- | --- | --- |
| cargo fmt --all -- --check | FAIL | 既有多文件格式差异；本轮bootstrap/transaction目标文件格式检查通过 |
| cargo test --locked --workspace --no-fail-fast | FAIL，exit101 | 最新运行还出现hbb_common的WebRTC连接超时和取消缓存残留；另有terminal_helper.rs架构示意doctest失败，ignored项不计已覆盖 |
| cargo clippy --locked --workspace --all-targets --all-features -- -D warnings | FAIL，exit101 | hbb_common及既有build scripts等lint；未删测试/扩大allow伪造绿色 |
| flutter analyze | FAIL（严格） | 247项既有warning/info；非fatal参数运行无error、exit0不能代替严格通过 |
| flutter test | PASS，172项 | 对应历史生成bridge后的源码 |
| PHP/Python/diff检查 | 已执行适用文件，局部PASS | 不替代容器、broker或完整后台门禁 |
| Docker/Compose/Go broker | BLOCKED | 本机未发现docker/go可执行工具；尚未完成容器端口/路由/卷重启持久化与broker并发 |
| 业务分支覆盖率≥80% | BLOCKED | 尚未生成可信分支覆盖率报告；测试数量不等于覆盖率 |

严格门禁中的既有失败保持记录，没有为本任务重排全仓或修改冻结子模块。后续最终发布仍需将适用门禁实际完成。

## 7. 本轮修复、审查与回归面

1. `res/msi/Package/Components/RustDesk.wxs`：仅两条删除服务动作新增条件。修复 stopped维护路径误删服务，明确卸载和允许重建路径仍按原动作执行。测试先红后绿，实际修复包两OS通过。
2. `src/management_agent/bootstrap.rs`：任务读回比较接受引号实体与字面引号等价，以及任务计划程序省略默认Enabled=true；显式false、重复标签、不同命令/参数、账户/权限/触发器仍拒绝。新增真实任务读回测试带正常安装路径。
3. 同模块固定agent服务契约核对：只接受无空格可执行路径的首对引号等价，参数和路径仍完整比较；有空格的未引号路径拒绝。具体VM验收见第10节。
4. `tests/test_msi_stopped_service_preservation.py`与bootstrap模块内测试锁定上述行为。`transaction.rs`只完成既有目标格式整理及先前停服落盘门禁，本轮未接生产执行器。
5. 本地`D:\RustDeskAgentLab\guest-*.ps1`只用于明确RDLAB客体，不进入客户端运行逻辑。文档和证据索引记录实际结果。

审查确认上述已有路径变更均对应本轮实际失败。没有修改固定密码、服务器、公钥、远控协议、默认权限、macOS签名、Android证书或自动发布触发；其他窗口的信任BAT、DTF记录及原有未跟踪资产保留。全工作区仍有前序实现和用户工作，不能把所有diff都归为本轮新增。

## 8. 剩余工作包

| 工作包 | 状态 | 剩余验收/实现 |
| --- | --- | --- |
| 1. 需求核对 | 完成 | 全范围保留 |
| 2. 后台授权与持久化 | 未完整完成 | 高权限/并发/旧实例对账/容器门禁 |
| 3. 本地健康与恢复 | 未完成 | 真实健康IPC、意图、R1安装门禁、R2–R5执行器及自愈outbox |
| 4. 事务/helper/bootstrap | 未完成 | 安装自动部署、完整元数据、所有权/握手、接续、恢复备份与回滚 |
| 5. 四包迁移 | 未完成 | EXE替换ProgramData agent、SOS两包、在线旧调用链、统一产物版本 |
| 6. 管理页面与能力 | 未完成 | 高权限/批量页面、构建能力manifest、客户端桥接E2E |
| 7. 全门禁与组合故障 | 未完成 | 严格源码门禁、80%覆盖率、长测/负载、断电回滚、真实远控 |
| 8. 可恢复Windows环境 | 完成 | 两OS干净快照与控制通道；保留用于剩余任务 |

360额外任务已按用户指示取消，不继续界面加白；也不以“没有找到接口”推断绝对不可能。

## 9. 证据、复现与资源保留

固定证据根：`D:\RustDeskAgentLab\report-evidence\`，`index.json`逐文件记录来源、大小、SHA-256；原始VM结果、安装日志、源测试日志在同目录归档。候选及源码变化后必须重新生成索引，不能将旧摘要作为新构建证明。测试凭据、完整配置密钥及provider凭据不进入报告或源码。

主要源码命令：

```text
cargo +1.90.0-x86_64-pc-windows-msvc test --locked --bin rustdesk-management-agent --test management_agent_journal --test management_agent_operations --test management_agent_network --test management_agent_transaction
python -m unittest discover -s tests -p test_management_agent_msi.py -v
python -m unittest discover -s tests -p test_msi_stopped_service_preservation.py -v
python -m unittest discover -s tests -p test_defender_installer.py -v
php -c <fixture-ini> tests/agent_operations_test.php
php -c <fixture-ini> tests/agent_status_test.php
python -m unittest discover -s tests -p agent_status_http_test.py -v
python -m unittest discover -s tests -p agent_console_e2e_test.py -v
```

Cargo需使用本机已准备的Rust/vcpkg/libclang环境；PHP/HTTP fixture需要sodium/pdo_sqlite和`AGENT_TEST_PHP`/`AGENT_TEST_PHP_INI`。浏览器使用隔离本地PHP/SQLite和Edge，不访问生产。

保留两VM、恢复快照、实际MSI、构建工具缓存、回归测试和诊断证据用于后续复测；不用后应按精确目录清理，不能删除用户ISO、宿主已安装客户端或原有数据。本轮API临时PHP服务/数据库和禁用计划任务fixture正常退出后自动清理。360检查现场快照保留，运行中交互驱动通过关机及恢复pre360快照清除。

当前报告未表示提交、push、tag、Release或生产部署，也不宣称原完整计划结束。

## 11. 物理机补测门禁

虚拟机已完成 Defender mock、旧 MSI 安装、Debug MSI 升级和部分状态/服务验收。最新 fallback 与恢复门禁代码已经通过 agent 编译、MSI 编译、7 项 Defender 测试和 13 项 recovery 策略测试。

以下项目没有在本轮 VM 中形成可接受的最新代码实证：

- 最新 agent 二进制替换后的主服务正常停止恢复；
- 强杀主程序进程后的恢复；
- 错误 RustDesk 服务 binpath 的固定路径修复；
- 主程序文件缺失时切换 fallback 客户端；
- 主程序和 fallback 同时缺失时的签名包恢复/有界失败；
- 最新 agent 升级后 `service-recovery-enabled` 门禁的实际落盘与重启保持；
- Win10/Win11 最新包的完整故障矩阵。

原因是 VirtualBox Guest Control 使用的 `rdlab` 受 UAC 限制，不能稳定创建或启动 SYSTEM 测试任务；同版本 MSI 重复安装还可能返回 0 但不替换已有 agent。上述项目必须在物理机管理员环境按 [windows-agent-physical-test.zh.md](windows-agent-physical-test.zh.md) 执行，并在执行前核对最新 agent SHA-256。

## 12. 远程维护能力实现边界

本轮已完成并通过代码/契约测试的能力：

- agent 状态上报和能力声明；
- 默认全 agent 运维权限绑定；
- `service.inspect`、`service.start`、`service.restart`、`service.repair`；
- `recovery.reconcile`；
- `package.verify`；
- 固定范围 `diagnostics.collect`；
- 固定 API/下载/ID 目标 `network.probe`；
- 主程序服务路径校验、正常路径修复和 fallback 路径选择；
- 批准包 resolve 的固定域、target、manifest、大小和 SHA-256 校验入口。

以下能力仍未宣称完成：

- `package.reinstall`、`package.update` 的实际安装事务执行；
- `agent.update` 的 helper 接管、旧 agent 备份、重启接续、身份保留和回滚；
- 最新构建在 Win10/Win11 VM 或物理机中的停止、强杀、错误 binpath、主文件缺失和 fallback 实际故障矩阵；
- 在线 API/OSS manifest 下载后的完整安装/回滚闭环。

这些路径继续使用批准包、签名、哈希、固定目标、事务互斥、维护状态、本地禁用和回滚门禁，不能用“命令已创建”或“服务 Running”替代安装成功证据。

默认权限策略：本私人项目的 agent 绑定默认授予全部已实现运维权限，后台页面也默认提交完整权限集合。未实现或未通过事务验收的高风险包动作不会因为权限已授予就被客户端执行；agent 必须返回明确的 gated/unsupported/reconciliation_required 结果，并保留审计事件。

## 10. 最终补充证据

### 10.1 bootstrap与R1

- bootstrap：`win10-bootstrap-final-result.json`。单独新agent手工替换测试资产后，首次安装/幂等重装均exit0；正常停止agent，独立bootstrap执行exit0且SCM Running。先前任务读回及无引号契约失败已保留，修复后通过。生产安装器仍未自动部署该入口。
- R1：`win10-r1-result.json`。仅在VM显式创建`service-recovery-enabled`测试门禁，停止主服务后16.05秒观察Running。门禁随后删除。`win10-final-inspect.json`确认恢复时间1791647149已持久化、无恢复错误、installation_id不变、双服务Running、marker不存在。
- 这些测试没有证明R2–R5、完整双服务周期恢复、bootstrap升级事务、缺文件恢复或真实远控就绪。
- 损坏包：`win10-invalid-validbaseline-final.json`，修正测试驱动对GUI进程的退出码读取后，实际msiexec退出1620，前后主文件和agent SHA-256相同，两项服务仍Running。只证明安装前拒绝，不证明安装中途回滚。

### 10.2 最新全仓与单独复测

最新全仓：`agent-report-final-workspace`，exit101。hbb_common 130通过/2失败；主RustDesk 376通过/3 ignored；agent31通过；其余批次见日志。失败：

1. `test_webrtc_loopback_gathered_endpoints`：`WebRTC wait_connected timeout`。
2. `test_cancelled_new_does_not_leak_the_pc`：取消后SESSIONS残留。
3. `terminal_helper.rs` line8 doctest：架构示意fenced block被解释为Rust。

VM关机后启用`--features webrtc`并用完整测试名独立重跑：连接测试仍FAIL（实际1项、30秒超时），取消清理测试PASS（实际1项）。因此连接问题仍阻断全仓；取消用例是本次全量出现且单独未重现的失败，不据此宣布修复。最初未启用feature的精确过滤命令匹配0项，明确不计PASS。

最终fmt exit1，最终Clippy exit101；两端`git diff --check`无空白错误。最终改动目标Rust文件的rustfmt检查通过。完整Flutter门禁复用此前同UI源码证据，未重复宣称本轮运行。

### 10.3 结果统计口径

以下按本报告选定的独立测试组计数，计数不表示全部能力覆盖率；历史红阶段保留但不重复计入当前状态。VM手工目标包替换仅作为子项，不算完整在线升级。不同OS分开计组。

| 状态 | 组数 | 范围 |
| --- | --- | --- |
| PASS | 16 | 两OS环境2、两OSMSI替换2、两OS停止注册保留2、Rust回归1、SQLite两组2、HTTP1、后台浏览器1、Defender mock1、Win10桌面1、手工bootstrap1、显式R1 1、有效前置损坏MSI拒绝1 |
| FAIL | 7 | 两OS冷启动2、DTF1、全仓test1、fmt1、Clippy1、严格Flutter analyze1 |
| BLOCKED | 7 | 四包完整在线迁移、安装中断/回滚、真实远控、容器/broker、覆盖率、长期/负载、构建能力清单各1 |
| NOT_APPLICABLE | 0 | 未将尚未实现或环境不足的必测能力记为不适用 |
| CANCELLED（范围变更） | 1 | 360界面信任验收，用户已取消该路径；后台静默信任仍未实现，不计功能成功 |

总31组，其中30组属于保留验收范围，1组由用户取消。身份保留是综合重装内的子证据，不另外增加PASS数量；损坏MSI的原前置污染保留在第5节，PASS只来自新的有效前置复测。统计组没有覆盖全部原计划编号，剩余必测范围见第8节及正式runbook。

### 10.4 最终资源状态

Win10回归快照`AgentRegression-20261010`：10a13936-6e3b-448a-a5b8-6d505265864b；Win11`MSIRegression-20261010`：3955224d-0834-4caf-905e-b57dc132d3d6。它们保留人工修复／准备fixture，不替代干净基线。两台最后按正常关机保存，继续任务前读取resume并核对实际状态。
