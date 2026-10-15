# Agent 升级实施进度与测试证据

## 2026-10-14 增量检查点

本节优先于下方历史状态，完整证据和验收边界见 [VM 实测记录](agent-vm-system-driver.zh.md)。本阶段新增修改尚未提交或推送。

- standard/SOS EXE 实际升级、失败回滚及 bootstrap 同步已有分场景证据；schema2 本地 MSI 修复成功及验证失败回滚已有实测。
- normal/fallback 均缺失的同 build 离线 MSI 恢复已完成真实 worker 成功、失败后的 agent 保全及接续对账测试。没有客户端回滚锚点时保持原失败事实，不能将 agent 保全称为客户端 RolledBack。固定目录 ACL 加固只作用于授权事务中的固定对象，保留原安全描述符，并在 ACL 写入与停服前复核本地意图。
- 本地事件记录、独立审计收据降级、坏收据隔离和补报已完成定向测试与复审。真实 helper 在事件写锁竞争时继续完成批准安装；实际事务 pending 的签名上传、丢 ACK 去重和 ACK 后清零另行验证，属于分段证据，不是生产服务完整部署验收。
- Flutter 当前源码全量 172 项单元/widget 测试通过；统一冻结源码的 56 个 Rust 测试目标 no-run 编译通过，但不能据此称全 workspace 测试已运行。新增全失和事件模块的限定格式检查已整理，旧全仓格式、严格 Clippy、覆盖率等门禁仍未全部通过。
- 尚未关闭：默认关闭的 MSI admission 可部署兼容与完整互斥、完整桌面/远控、最终断电接续矩阵、其他适用平台与正式包验收。部分相关任务受到工具审核阻塞，未将未执行任务算作通过。
- 桌面有限验收中最小化/还原通过；服务账户配置仅 salt 字段变化，ID/密钥/密码存储值保持。已找到本轮未改的既有按需生成路径，但精确写入因果与认证影响没有实测闭环。

历史段落中的“未实现”“当前版本”“当前现场”和提交状态均只代表当时检查点；不能覆盖上述较新证据，也不能把新增分段通过扩大为完整发布就绪。

日期：2026-10-10

2026-10-12 最新执行证据见 [VM 实测与快照](agent-vm-system-driver.zh.md)，其中按阶段注明安装包和测试二进制差异。两系统本地签名API→双包HTTP→独立helper→真实standard MSI→终态补报已通过，包含2026100502→2026100503升级、失败回滚、bootstrap同步与原身份保留；普通MSI的bootstrap初始化/升级/回滚也已实测。v158各224项专项测试通过，v164另验证终态接续错误保留修复的完整helper15项。完整八工作包仍保留：四包迁移、真正断电接续、完整离线恢复阶梯、独立agent自更新、真实远控及全仓执行门禁尚未完成，生产status尚未声明包安装能力。后续历史段落不作为最新完成声明。

v173控制台三条真实浏览器/SQLite测试通过，覆盖1440/390重启取消/确认与能力门禁。v186–v196进一步完成两系统SOS专用MSI的standard→SOS切换、同包重复安装、本地禁用与周期双服务恢复，以及干净断网系统全新安装；SOS服务、清单、状态和恢复任务一致，应用ready。首次开机恢复在并行冷启动时有任务超时证据，单台对照受时钟回退和触发归因影响，尚不能宣布开机验收完成；生产时限未改。当前两VM均为SOS全新安装，旧standard/SOS健康快照和注册安装源保留。

本文件记录当前工作区实现状态，不表示已经发布。功能代码均未提交/推送；远程 `80df28076` 仍为此前文档交接提交。原始 DTF 段落及独立信任 BAT/测试属于另一个窗口新增的工作，保留，不由本轮自动操作现场机器。

## 已实现的底座

| 范围 | 当前实现 | 尚未保证 |
| --- | --- | --- |
| 纯恢复决策 | 文件存在仍检查 stopped；意图、维护、启动宽限、退避门控；正常/保底/包恢复决策 | R2/R3/R4/R5 执行器尚未接入，不是完整救援链 |
| Windows 观测 | SCM 状态/配置、精确 service 参数、LocalSystem、正常/保底文件及重解析点检查 | 不是密码学安装完整性证明；无应用 ready IPC |
| R1 启动 | mutex 下重查标记、意图和服务状态，记录退避后启动并核对 SCM Running | 需要 `service-recovery-enabled` 门控；安装维护互斥未完整接入，不应手工对生产启用 |
| 身份读取 | 原有 TOML/密钥校验；旧版零 nonce/随机 nonce 的 enc_id 解密、机器 UUID/公钥 fallback | 不生成替代 ID/密钥；凭据备份/登记迁移尚未完成 |
| 状态上报 | 既有设备签名；后台只接受已登记密钥；独立状态和命令 worker、有界队列、应答限制 | 当前仅声明 `agent.status@1`、`service.inspect@1`；其他命令未开放 |
| SQLite 投影 | 精确身份、安装实例、持久 session_generation/sequence、重复幂等、旧会话拒绝、未知字段保留；显式重新绑定允许新实例替换 | 完整凭据迁移和旧实例事务对账未完成 |
| 后台接入 | enroll/commands/claim/events/package resolve、管理员授权/创建/取消/overview、运维审计；设备详情展示独立 agent 状态及只读检查 | 仅观测页面完成；安装、agent.update、批量运维未完成；MySQL 没扩展 |
| DTF bootstrap | 独立副本、SYSTEM 开机/5 分钟任务、hash/服务契约/禁用检查、有界启动及预算 | 手工管理入口 `install-bootstrap`/`bootstrap` 已有；不自动接安装包，不支持摘要更新、服务重建/备份文件恢复、长期健康预算复位 |
| Defender | 首次 UI 选项、64 位 HKLM 保存选择、静默明确参数、EXE/MSI前置配置、增量添加/读回、拥有项记录；控件测试和原生CA编译通过 | 新源码完整四包和真实 Defender 未验收；不会配置 360 |

主服务 Running 仅代表 SCM 成功，不把它宣称为远控成功；status 始终把 application 标为 unknown、probe_supported=false。

## DTF 必测路径

DTF 的两个请求链同时停止不是充分的服务停止证据，需现场核对。实现对应回归：

1. 主服务和 agent 均正常 stopped，不是只 taskkill agent。
2. 已批准 exe 恢复后由定时/开机 bootstrap 拉起 agent。
3. agent 按可靠运行意图启动主服务；本地主动停用不得被覆盖。
4. 文件仍隔离、服务 Disabled、任务缺失、再次启动被拒绝时明确失败/退避。

本机已在运行 RustDesk 和 agent，未对它们停服/破坏。真实双服务恢复 E2E 未执行；只有决策和独立禁用测试任务的注册/查询/删除测试。

## 已执行的测试

| 检查 | 实际结果 | 边界 |
| --- | --- | --- |
| recovery standalone Rust | 11 项通过 | 纯决策/路径/退避/MSI码分类，MSI码分类尚未接旧安装执行器 |
| DTF bootstrap standalone Rust | 4 项通过 | 纯决策；非现场双服务停启 |
| Defender 参数 standalone Rust | 2 项通过 | 用户参数语义 |
| Win32 mutex | 2 项通过 | 实际跨线程、跨进程；未接普通安装协调 |
| 新模块隔离 Cargo harness | 初始12项及扩大主模块后的36项通过 | 扩大harness含重复模块测试，不是36个独立功能；hbb_common/base和路径类型有stub，非全仓测试/真实包 |
| Defender PowerShell mocks/契约 | 6 tests 通过，选择/错误含 16 个 subcase | Defender/注册表 IO 全部替换 mock；另含路径范围拒绝 |
| 真实 CMD staging fixture | 通过，计入隔离 harness | 仅 Unicode/base64/脚本失败后继续；没有 Defender IO |
| Task Scheduler roundtrip fixture | 通过，计入隔离 harness | 唯一 disabled task，用后删除；修复 UTF-16 导入和 SYSTEM XML LogonType |
| 既有 management-agent MSI Python契约 | 12 项通过 | 源码/打包契约，不能代替实际 MSI |
| agent status PHP/SQLite | 30 断言通过 | 真实 SQLite 数据读写/冲突/旧数据保留 |
| agent status HTTP/SQLite | 7 项通过 | 生产路由/验签/nonce，测试只替换 Windows不支持的数据库连接边界 |
| PHP语法、Python compile、diff check | 已执行适用文件 | 不代表全部仓库门禁完成 |

Defender接入增加：EXE安装页明确选项、静默`defender=0/1`、普通升级继承；MSI secure property、独立保存值/复选框、64位注册表读取、InstallFiles前CA。保底MSI用专用属性跳过配置，避免扩大恢复目录排除。CMD实际传输fixture和CA脚本嵌入/长度/Unicode/XML检查通过；C++/WiX和Flutter仍未编译验收。MSI日志和HKLM LastResult提供非成功证据，不能承诺真实组织策略环境成功。

本机未执行真实Get/Add/Set/Remove-MpPreference、未启用Defender、未改防篡改。mock测试入口预先定义禁止真实cmdlet访问的桩，只有测试明确替换的模拟函数可以执行。用户此边界是后续测试的持续约束。

扩大主模块检查曾先失败：已有路径参数测试暴露尾斜杠后的残留引号，已最小修复；重复加载测试模块曾共用临时资源，已采用随机唯一文件/任务名，重跑通过。Task Scheduler只创建disabled唯一fixture，查回并删除，不停现有服务。

可复跑底座：

```text
python scripts/test-agent-foundation.py --rustc <绝对rustc路径> --linker <需要时的linker路径>
python -m unittest discover -s tests -p test_defender_installer.py -v
python -m unittest discover -s tests -p test_management_agent_msi.py -v
```

标准 Cargo 入口仍应执行 `cargo test --locked --bin rustdesk-management-agent`；不能永远用隔离 stub harness 替代。当前工具临时装在 `C:\Users\Administrator\AppData\Local\Temp\opencode`，无需改系统 PATH/Git 配置。PHP HTTP 测试使用 `AGENT_TEST_PHP` 和可选 `AGENT_TEST_PHP_INI`，临时服务、数据库自动清理。

## 阻塞与未执行

- 完整 Cargo test/clippy：Git `wezterm` 的 freetype 子模块下载报 `invalid index-pack output`，未进入实际全仓编译；离线模式缺该依赖。
- 隔离harness Clippy严格模式：重复加载模块、stub类型及既有代码告警使其失败；新增Windows模块测试位置告警已修复，不把隔离harness通过测试宣称全仓Clippy通过。
- 全仓 fmt check：既有多文件格式差异及 `src/ui/inline.rs` 等构建生成文件缺失；只整理本轮新模块，未重排无关源码。
- Flutter analyze/widget/desktop integration：未安装 Flutter SDK。
- MSI C++/WiX实际包：缺 MSVC、Windows SDK、NuGet/.NET SDK/WiX。XML parse/入口检查不等于 CA 可编译。
- Docker/broker/Playwright：工具未完整准备，未完成容器/浏览器门禁。
- EXE/MSI 四包全新安装/旧版升级、agent 替换、断电接续、真实远控/锁屏/UAC：未执行，须可恢复 Windows VM。
- Defender 实际读取/写入/启用、防篡改/组织策略：用户明确禁止本机真实测试，不能在本机执行；隔离 Windows 10/11 验收仍 BLOCKED。
- 覆盖率报告尚未生成，不宣称业务分支已达到 80%。

## 临时资源清理

本轮HTTP测试服务器与新数据库由测试自动清理；之前清理失败留下的 `agent-status-test-9bhfkq1s`、`agent-status-test-vcavzok1`、`agent-status-test-_6ic0ug6` 已精确删除。用于脚本嵌入验证的临时 `DefenderScript.inc` 已删除。测试任务 `RustDeskAgentTest-*` 经查询无残留，原有RustDesk/agent服务仍Running。

临时工具链、隔离harness及Cargo失败日志暂保留在 `C:\Users\Administrator\AppData\Local\Temp\opencode`，用于后续依赖/类型排查；不进入提交，任务验收或换机结束后可按本轮精确路径清理。没有本轮Docker资源，也未运行现有信任BAT的真实读写模式。

## 下一步实施顺序

1. 准备 MSVC/Flutter/隔离 Windows 包链，先做 POC-EXE/MSI/RESUME；Defender只mock，真实写入另机。
2. 将 bootstrap 的安装、重复执行、版本摘要更新、失败清理及彻底卸载接入完整迁移事务；当前同hash校验会在agent更新后拒绝启动，需要事务性更新批准记录。
3. 安装器提权后统一维护互斥/持久事务，再考虑启用 service-recovery；默认门控必须保持，不能只创建标记绕过验收。
4. 实现本地健康 IPC、可靠意图记录、R2服务修复、R3可信保底、R4离线包、R5精确目标和完整回滚。
5. 后台运维授权、commands/claim/events、可靠动作outbox、页面和能力测试清单；仅已测支持能力才声明。
6. 完成全仓/四包/两系统真实 E2E 和资源清理后，再请求获授权的提交/推送/发布。

## 回归面审查

- `src/management_agent.rs`：新增观测/身份/异步策略状态、state字段、门控R1，保底文件存在不再停掉策略上报；已接入固定服务启动/重启/修复、恢复协调、固定对象校验、脱敏诊断和固定网络探测命令执行。批准包 resolve 已接入固定域下载与摘要校验入口，但仍必须经过现有安装事务/helper/回滚链后才能执行包更新。
- `src/management_agent_bin.rs`：日志和bootstrap命令入口，原服务/恢复helper仍保留。
- `src/platform/windows.rs`、`src/core_main.rs`：Defender配置的薄安装/升级钩子和显式静默参数，保留文件替换与服务代码。
- Flutter安装页与新控件：明确用户同意/拒绝并传参，不更改其他导航。
- MSI CA/fragment/dialog/project：同一PS逻辑在InstallFiles前运行，配置失败不阻断；保底恢复专用标记避免范围扩大。
- 后台index/lib：SQLite新增投影与短签名路由、详情字段和精确删除清理，不改主心跳和旧更新事件语义。

除这些必要接入，未修改固定服务器、密码、公钥、默认权限、macOS签名、Android证书或发布触发方式。无新生产依赖，也未提交、push、发布或访问生产。

## 2026-10-10 重启前 TDD 续接检查点

用户将重启客户端/OpenCode，本节是最新状态；上面的历史工具缺失和测试数量以本节为准。正式目标仍是完整执行 `agent-recovery-upgrade-plan.zh.md`，没有完成全部计划。

### 完整任务账本（持续保留八个工作包）

任务栏不得用近期步骤替换或缩减本账本。任务数量不是测试数量；局部测试通过不代表工作包完成。2026-10-10恢复会话时曾错误压缩为五个近期步骤，现已恢复，未删减正式范围。

| 工作包 | 当前状态 | 未完成范围 |
| --- | --- | --- |
| 1. 完整需求与环境核对 | 完成核对 | 随环境变化持续更新事实 |
| 2. 后台授权、命令、事件与包接口 | 部分实现，未验收完成 | 授权并发、旧实例对账、完整协议/SQLite/容器门禁 |
| 3. 本地健康、意图、R0–R6与outbox | 进行中；队列与日志子项已补 | 健康IPC、可靠意图、完整恢复执行链、安装动作outbox、旧实例对账 |
| 4. 安装事务、helper与bootstrap | 部分底座，未完成 | 接管、重启接续、回滚、摘要升级、服务重建、批准备份恢复 |
| 5. EXE/MSI与旧版一次迁移 | 未完成 | 四种实际包迁移、ProgramData agent替换、版本资源、失败保留旧版本 |
| 6. 管理页面、能力清单与跨端测试 | 部分只读完成 | 高权限/批量流程、构建能力清单、桌面驱动验收 |
| 7. 全量门禁、真实包验收与清理 | 未完成 | workspace/Flutter全量、覆盖率、四包/两系统/真实远控/断电/负载 |
| 8. POC环境与构建阻塞 | agent与Windows Debug已可构建 | 正式四包、可恢复VM与独立控制端；严格fmt/Clippy既有失败 |

本次重启续接的未知类型/版本、执行参数、跨页投递、ack丢失、取消、过期、日志中断/并发、保留期/容量和网络退避用例已按红→绿执行；仍是工作包3的局部子项。最新结果见末尾“重启后本轮续接结果”，不按任务启动或局部通过宣称整个工作包完成。

### 本轮完成并实测

- 后台 `sqlite/agent_operations.php`：授权绑定、默认仅 observe、显式 service/install/agent_update/enable 等权限、撤销 tombstone/代次、公钥变化拒绝；独立命令创建/幂等/分页/领取/取消/事件状态转换/事件去重/精确恢复包解析。
- 新包解析匹配当前产品 `rustdesk-yan` 和实际 OSS `/rustdesk/stable/` 路径；保留既有跨域镜像元数据但不授予跨域执行资格。大小/摘要/目标变化与撤销拒绝；客户端仍必须做密码学验签，后台检查字段形状不等于验签。
- 管理页 `sqlite/devices.html`：独立 agent 在线/离线、服务和应用状态、只读授权、固定服务检查、撤销确认、命令历史和自动刷新；不把 pending 当作成功。
- 后台 `sync_reported_update_policy`：`management_agent_enabled` 单向服从已存管理员期望，旧 sysinfo 不得反向开启；其他旧更新选项继续同步。
- 客户端 `journal.rs`：领取意图/可靠接收/开始/结果分别原子落盘、稳定 event_id、确认后删 outbox、摘要冲突/损坏/容量/过期/并发旧快照拒绝；未接安装事务，不宣称完整断电恢复。
- 客户端 `operations.rs` / `operations_runtime.rs`：严格信封/身份/代次/正文摘要与命令号签名；独立有界 worker；实际仅执行 `service.inspect`，其他动作未开放。
- bootstrap 相同摘要/配置重复安装核验文件和已有任务；严格核对动作/参数/SYSTEM/权限/单实例/周期/时限/启用状态；摘要升级仍必须走未来事务。
- Defender 新增结果持久化失败/无效保存选择/缺失provider字段 mock；真实 CMD 模拟子进程超时后继续安装，无真实 Defender/注册表 IO。
- MSVC 实际编译暴露 `Defender.cpp` 缺少 `strutil.h`，已最小添加头文件，重新编译通过。

| 最新测试 | 结果 | 适用边界 |
| --- | --- | --- |
| `tests/agent_operations_test.php` | 65 个 SQLite 断言通过 | 真实数据，未生产部署 |
| `tests/agent_status_test.php` | 30 断言通过 | 真实 SQLite |
| `tests/agent_status_http_test.py` | 12 测试通过 | 生产前端路由/验签；仅替换 Windows PDO路径边界 |
| `tests/agent_console_e2e_test.py` 指定浏览器用例 | 通过 | Edge/Playwright 1440与390宽度、API和SQL核对、刷新/503/403/取消确认/无溢出 |
| 隔离 Cargo harness | 43 lib + 4 journal + 2 operations 通过 | lib含重复模块，hbb_common/base仍stub；非全仓 |
| Rust→PHP→SQLite 往返 | 通过，计入lib | 真实Rust签名/后台验签/claim/3事件/本地ack/SQL终态；仅只读SCM |
| 底座 standalone | recovery 11、bootstrap 4、consent 2、guard 2通过 | 无实际生产服务停启 |
| Defender Python | 6 tests，19个选择/失败子场景通过 | 全mock；另有范围和嵌入检查 |
| 既有 MSI Python契约 | 12通过 | 非安装E2E |
| Flutter Defender widget | 2通过 | `flutter test --no-pub test/defender_install_option_test.dart` |
| Defender新控件/测试 Dart分析 | 无问题 | 不等于Flutter全仓通过 |
| MSI CustomActions MSVC x64 | 编译通过 | 已生成DLL，未调用Defender CA |
| MSI隔离打包fixture | 生成Package.msi | 使用当前已安装旧客户端/旧agent副本＋本轮CA，验证WiX链；不是新源码候选包，未安装 |

### 工具环境与运行任务

用户已明确授权“本机准备构建工具”。已安装 `C:\RustDeskBuildTools`（VS Build Tools 2022、v143、Windows SDK）、临时 .NET SDK 8.0.425、NuGet/WiX 4.0.5、Flutter 3.24.5/Dart 3.5.4、Rust MSVC/GNU 1.90.0、Python Playwright 1.63（使用系统Edge）。不因此授权提交、push、发布或生产部署。

- 临时根：`C:\Users\Administrator\AppData\Local\Temp\opencode`。
- Flutter：`flutter-3.24.5\bin\flutter.bat`。
- Rustup/Cargo：`cargo\bin\cargo.exe`、`rustup`；隔离harness用 `CARGO_HOME=...\cargo-check`，全仓用 `...\cargo`。
- `flutter_rust_bridge_codegen` 1.80.1 已安装到 `cargo-check\bin`，还未生成bridge；后续还需 cargo-expand 1.0.95 及官方生成步骤。
- PHP/INI：`php\php.exe`、`php-agent-test.ini`。
- Python Playwright通过 `PYTHONPATH=...\playwright-python` 使用，无系统pip/PATH修改。
- 为缓解Cargo长路径，创建了 `subst R: ...\cargo` 映射；后续可尝试 `CARGO_HOME=R:\`，该尝试尚未运行，重启机器后可能需重建映射；使用前先确认R盘没有其他用途。
- `agent-full-cargo2` 已结束，exit 101：harfbuzz深路径超过Windows限制，完整编译未开始；stderr日志已保存。
- `agent-bridge-codegen` 已结束，exit 0，工具安装成功；没有尚在运行的本轮构建任务（续接时再次确认）。
- 首次后台日志脚本未正确继承detached输出，旧 `agent-full-cargo` 的空日志不能作成功证据；已修临时runner显式重定向，以 `agent-full-cargo2.stderr.log` 为准。

### 尚未完成与明确阻塞

1. POC-EXE、POC-MSI、POC-RESUME：没有隔离VM/快照/独立远控控制端，禁止破坏当前唯一服务链；实际新源码客户端/agent完整包未生成。
2. 全仓Cargo test/clippy：依赖harfbuzz长路径；`cargo fmt --all -- --check`仍有既有格式差异和缺少生成 `src/ui/inline.rs`。
3. Flutter全仓 analyze/test已尝试，失败：缺 `flutter/lib/generated_bridge.dart`，并产生连带类型错误和既有告警；不可删除相关测试或把新增控件通过称为全仓通过。pub get临时改变了pubspec.lock，已恢复原lock，后续准备依赖后也要检查避免无关版本漂移。
4. R0健康IPC/重连、可靠意图、R2服务修复、R3完整可信保底、R4离线包、R5批准在线fallback、预算/熔断的完整执行链均未完成。
5. 统一安装互斥/事务/helper握手/接续任务/回滚、EXE正常更新替换ProgramData agent、MSI版本资源/四包首次迁移未完成。
6. bootstrap摘要升级、失败清理、服务重建、备份文件恢复、核心挂起检测和安装包自动生命周期未完成。
7. 远程服务/安装/agent更新实际执行、旧实例事件对账、网络专属退避/Retry-After、命令游标的客户端推进、完整schema错误分类与未知命令不中断队列仍未完成。
8. 可靠日志仍是原子快照：`pending`残留会干净拒绝后续写入，尚未实现断电接续清理；30天去重清理/100MiB日志/动作满队列测试未完整实现。
9. 高权限/批量页面、能力构建清单、1000台负载、容器/broker全门禁、Windows10/11真实远控/锁屏/UAC、覆盖率80%报告未完成。
10. 本机真实 `Get/Add/Set/Remove-MpPreference`、防篡改、真实排除项测试持续禁止；只mock；真实验收需另行授权可恢复环境。

### 重启后第一步

继续读取本节、正式计划和测试手册；核对两个仓库status和用户其他窗口改动。不得把已通过的局部测试重复解释成整套完成。

1. 确认全仓依赖短路径方案；使用同一Cargo.lock，解决harfbuzz，再执行真实agent编译和workspace门禁。
2. 按 `flutter-tag.yml` 的bridge生成链准备生成资产，再运行Flutter全仓及Windows实际构建；不修改macOS签名/Android配置。
3. 按正式计划先完成三项POC，再开放高风险安装/agent.update；准备隔离VM后执行四包旧版单次迁移与断电矩阵。
4. 其余TDD工作按未完成清单逐项红→绿→集成→实际包验证。继续保持功能源码未提交。

### 本轮资源与回归面

- HTTP服务器、SQLite临时库、Edge浏览器/context、disabled唯一任务、正常通过Rust/CMDfixture均自动清理。
- 保留隔离打包目录 `...\agent-msi-fixture-64k_412p`（成功MSI与旧payload）、`...\agent-msi-fixture-lvek66q9`（首轮缺lib失败fixture），均只作打包调查，不安装；后续验证后精确清理。
- 两次直接MSBuild生成了 `res/msi/CustomActions/x64/Release`、`res/msi/x64/Release`、`Package/bin`/`obj`和生成的 `DefenderScript.inc`；属于本轮构建资产，后续稳定后清理，不提交。
- 临时SDK/工具/依赖缓存和日志为续接保留；全局VS工具为用户授权的构建环境，不作测试清理卸载。
- 当前机器的 `RustDesk` 与 `RustDeskManagementAgent` 均Running；没有本轮停/删服务、安装fixture或Defender真实访问。
- 本轮新增行为回归面：`management_agent.rs`（独立命令worker），`status.rs`（仅新增已接入inspect能力），`bootstrap.rs`（重复安装与严格task核验），`windows_defender.rs`（生产60秒值不变，增加mock超时参数测试），`Defender.cpp`（编译所需头文件）；后台index/lib薄路由/schema、agent_status显式重新绑定、devices页新观测操作及agent字段单向策略。各项均对应正式计划，不重写远控协议与跨平台旧路径。
- AOCI工具/索引未在当前环境发现，未生成或伪写认知维护记录。

### OpenCode 自动接受权限排查记录

本机桌面端 `@opencode-ai/desktop` 为1.18.35，npm CLI为1.18.31。只读检查 `opencode debug config` 的permission/default_agent/agent/tools投影，未发现显式deny或agent覆盖；没有修改全局权限、provider、凭据或桌面程序。

读取实际安装的app.asar前端代码确认：旧设置界面的Switch为 `disabled: !dir()`；新布局 `createPermissionScopeController` 从选中session的lineage取得 `session.directory`，其 `enabled` 为 `!!directory()`。无项目/无选中已存在会话时灰色，属于作用域条件，不是管理员权限问题；不能凭桌面/CLI版本差异认定根因。

用户操作：关闭设置→打开RustDesk项目→选中已有的本次会话（非首页/空白draft）→再打开设置→启用“自动接受权限”。若新布局仍无法取得会话目录，可切换旧布局后在项目内设置；命令行已核实支持 `opencode "D:\github\rustdesk" --continue --auto`，只自动批准原本ask且未显式deny的请求。重启后继续业务任务须重新读取本节。

## 2026-10-10 重启后本轮续接结果（最新）

完整任务清单恢复为八个工作包，只有工作包1为完成核对，其余保持部分实现/待验收，工作包3进行中。用户指出任务栏从八项压成五项后，已修正并固定上面的完整账本；没有删减原计划。

### TDD红→绿与实际接入

1. `operations.rs` 增加投递身份/摘要校验和执行拒绝分类：未知schema/type/version报告unsupported，未知执行参数报告rejected；不放宽设备、实例、代次和正文摘要。
2. `operations_runtime.rs` 有界读取最多5页×20项，跳过身份不匹配/摘要损坏的投递并限流记录；游标按数字时间＋命令ID验证前进；单轮最多2个实际命令，避免单条坏投递挡住后页检查。
3. Rust→PHP→SQLite测试注入首屏20个foreign投递、后页unknown schema和合法inspect；重复事件ack丢失后补报、GET后claim前取消、后台旧accepted但本地未收且已过期。核对本地日志、SQL状态、事件行数以及原主心跳不变。
4. `journal.rs` Windows独占文件写锁、preimage摘要核对后清理崩溃残留pending，未提交快照不被提升为开始/成功；活跃写者占锁时保留其staging；老快照拒绝覆盖。
5. 日志增加cancelled终态、开始前取消/过期的本地落盘，已开始动作不得被过期或取消打断；30天仅清理无未确认事件的终态，running/reconciliation与未ack记录保留。
6. outbox最多10,000项/64MiB，开始前保留终态结果空间，满队列拒绝新动作，不删除未ack事件；8项边界测试之外新增第9项容量用例。
7. `network_policy.rs` 独立网络退避，单调时间；401/403至少300秒，网络失败退避到3600秒；429/其他HTTP的Retry-After支持数字和HTTP日期并限制1–3600秒。`PollError`保持结构化HTTP/Transport/Local分类，接独立commands worker，不改本地恢复预算。
8. `status.rs`仅修测试fixture：Windows接受的stream显式切回阻塞并保持5秒timeout，消除非阻塞继承导致的偶发10035；生产状态网络函数不因此改变。

### 最新执行证据

| 项目 | 实际结果 | 限制 |
| --- | --- | --- |
| 真实仓库 `cargo test --locked --bin rustdesk-management-agent` | 29通过 | 使用真实hbb_common/base、MSVC及原生依赖，非stub |
| 真实仓库journal/operations/network集成test入口 | 9＋5＋2通过 | 生产服务无停启；本地fixture |
| `agent-real-final-tests2.result.json` | exit 0 | 最新联合回归：29/9/2/5，各过滤器实际执行 |
| 真实agent构建 | 成功，`target/debug/rustdesk-management-agent.exe` | `status --edition standard --package-kind msi`启动冒烟通过；未装入现场服务 |
| Flutter Rust库 | `cargo build --locked --lib --features flutter`通过 | Debug动态库，非发布包 |
| Flutter全量tests | 172通过，exit 0 | `agent-flutter-full-tests-ready`日志；对应生成bridge的源码 |
| Flutter全量analyze | 无error，247既有warnings/infos | `--no-fatal-infos --no-fatal-warnings` exit0；严格默认命令仍失败，不能说严格全绿 |
| Windows Flutter bundle | Debug构建成功 | `agent-flutter-windows-build2.result.json` exit0，`rustdesk.exe --version`输出1.5.3；未真实远控E2E |
| workspace `--no-fail-fast` | 单元/集成批次通过，最终exit101 | 既有`src/server/terminal_helper.rs:8`架构文档代码块被当Rust doctest导致最后失败；不能标总门禁通过 |
| strict workspace all-features Clippy | 已实际执行，exit101 | 既有hbb_common、portable/dylib构建脚本及其他依赖告警/错误；未修改无关源码或降级门禁 |
| `cargo fmt --all -- --check` | 已执行，失败 | 既有多文件格式差异；构建前缺version生成资产，后续已由真实build生成 |
| API HTTP/SQLite | 12测试/65断言通过 | 最新fixture只在显式测试环境取消固定test-device命令，生产路由验签不改 |
| 管理页E2E | 桌面1440与390通过 | Edge，API/SQL核对、加载/错误/刷新/确认/无溢出 |
| Defender mock回归 | 6 tests、19选择/失败subcases通过 | 没真实MpPreference/注册表IO |

日志根仍为 `C:\Users\Administrator\AppData\Local\Temp\opencode`；详细真实构建/回归入口见 `agent-real-final-tests2`、`agent-full-cargo-native3`、`agent-real-journal-contract-tests`、`agent-real-build`、`agent-flutter-rust-lib`、`agent-flutter-full-tests-ready`、`agent-full-workspace-tests2`、`agent-full-clippy-native2`。

### 构建环境已解除的阻塞

- `CARGO_HOME=R:\`（subst指向既有临时cargo缓存）已解决harfbuzz长路径，锁定Git依赖全部取回；不改Cargo.lock或Git全局配置。
- vcpkg固定commit `9e593bb18ea69cc5095e012465dcd675a822ed0d`、manifest实际16项全部构建完成，约28分钟；路径 `...\vcpkg-agent`。浅克隆缺port历史已通过完整fetch解除。
- MSVC/SDK仍用`C:\RustDeskBuildTools`；进程设置`VCPKG_ROOT`、`VCPKG_INSTALLED_ROOT`，`LIBCLANG_PATH=...\llvm-agent\bin`。
- bindgen需要`BINDGEN_EXTRA_CLANG_ARGS=-IC:/RustDeskBuildTools/VC/Tools/MSVC/14.44.35207/include -IS:/`；S盘subst指向Windows SDK UCRT。只给当前命令环境加路径，不改系统PATH。
- cargo-expand1.0.95和flutter_rust_bridge_codegen1.80.1已安装。完整LLVM DLL临时来自libclang18.1.1；生成bridge使用临时YAML，必须把Rust生成文件置于已有crate声明`src/bridge_generated.rs`，否则cbindgen漏掉wire接口，即使生成工具返回0也不算成功。
- 成功生成 `src/bridge_generated.rs/.io.rs`、`flutter/lib/generated_bridge.dart`及freezed等现有构建资产；按原流程生成`src/ui/inline.rs`，真实build生成`src/version.rs`。不把这些生成内容当业务补丁。
- bridge/pub get临时改变了Flutter锁文件，已在最终稳定状态恢复原`flutter/pubspec.lock`；新一轮pub get后仍须检查漂移。
- portable workspace要求载荷，已用既有generate函数从当前已安装旧客户端生成`libs/portable/data.bin`和`app_metadata.toml`，仅用于源码门禁；禁止把它标成新候选包或上传。

### 未完成范围（原计划持续有效）

R0健康IPC/重连、可靠意图、R2/R3/R4/R5完整执行、自愈事件outbox/恢复资产准备、统一安装事务/helper/接续/回滚、bootstrap摘要更新和自身备份/服务重建/挂起、EXE在线agent替换、MSI单调文件版本/四包迁移、高权限/批量管理、能力manifest、跨实例历史对账、1000台负载、Docker/broker门禁、80%覆盖率以及所有实际Windows10/11四包和远控/断电验收仍未完成。网络退避目前只接新命令worker，旧策略刷新及状态worker的网络专属退避仍待补。

本机没有可恢复VM/快照或独立远控控制端。用户授权本机准备构建工具不等于授权破坏当前服务或访问生产；三项POC及破坏性矩阵继续BLOCKED。service-recovery-enabled保持门控，远程仅声明/执行service.inspect，未开放安装/agent.update。

### 本轮审查与资源

必要回归面：`management_agent.rs`只增加网络policy模块及commands worker退避；operations/runtime改投递/领取/分页/拒绝/补报，journal改写入接续、取消/过期、保留期和容量；status仅测试socket设置；API仅测试router显式故障注入。没有更改固定密码/服务器/公钥/权限、远控协议、其他平台更新或发布触发。

审查发现并修复：游标数字进位、未知schema阻塞、old accepted过期阻塞、写者旧快照覆盖、崩溃pending永久阻塞、开始时outbox不留终态空间、HTTP Retry-After丢失分类；覆盖率报告仍未生成。只读inspect可在已有running日志下重新采样，非幂等动作仍须reconciliation，不承诺整机断电安装事务已完成。

本轮失败测试残留精确清理：`rustdesk-journal-locked-39360`、`rustdesk-journal-interrupt-39360`、`rustdesk-journal-cas-34552`、`rustdesk-journal-capacity-43720`、`agent-status-test-rust-33044-dc64a612b056266c`；正常HTTP/SQLite/CMD/禁用任务/浏览器fixture用后自动清理。SDK、vcpkg缓存、生成源码、Debug bundle与真实agent保留供下一阶段打包/桌面验收，不提交。旧打包fixture继续保留于上节精确目录。

隔离副本 `...\agent-msi-fixture-b7eitxbz` 已成功打包新源码Debug bundle和新agent；`agent-new-msi-fixture.result.json` exit0，产物`res/msi/Package/bin/x64/Release/en-us/Package.msi`。这个fixture使用准备时的Debug产物，后续源码变更不会自动进入包，不能作最终版本一致性证据。该包不包含尚未完成的迁移事务，未安装，禁止称为完整保护或发布。本轮未commit/push/tag/Release/生产访问，本机RustDesk和agent保持Running，真实Defender禁止仍有效。

### 后续源码级事务TDD（工作包4进行中）

用户确认目前“暂时没有环境”（隔离VM/快照/独立远控端），继续独立源码级TDD，实际安装POC继续BLOCKED。

- 新增 `src/management_agent/transaction.rs` 和 `tests/management_agent_transaction.rs`，先执行缺实现红阶段，再执行旧快照覆盖红阶段、伪造阶段写入红阶段，最后9项测试通过；真实仓库命令 `cargo test --locked --test management_agent_transaction`，`agent-transaction-final.result.json` exit0。
- 阶段 planned/package_verified/prepared/services_stopped/applying/validating/committed 与 rolling_back/rolled_back/rollback_failed，逐步核对准备证据；原身份和目标包摘要固定。
- 准备阶段要求包/备份/helper/接续任务/握手/维护证据；停服与执行仍要求对应证据，本地禁止阻止新变更；已进入替换后仍允许安全回滚，不报假成功。
- 已开始阶段中断返回reconcile，活跃owner返回wait，rollback返回rollback；24小时/3次边界和时间回拨进入人工对账；committed/rolled_back仅补报，不能重装。
- Windows独占writer文件锁、flush＋MoveFileExW原子发布；revision逐次推进，旧快照/跳过落盘/伪造相邻阶段/更换包目标拒绝；损坏旧记录、坏staging或锁冲突保留旧字节。
- **尚未生产接入**：module仅由测试入口编译，未放进management_agent dispatch/CLI；没有真正停止服务、替换文件或注册任务。完整事务仍缺触发来源、命令关联、旧版本/服务配置、可信路径引用、备份位置、helper版本/创建时间/接续任务标识、结果、staging崩溃恢复、实际所有者身份验证及执行器。当前Evidence字段由测试fixture提供，不能把测试bool视为真实系统验证。
- 保存层pending残留仍干净拒绝，不自动提升未提交事务；下一步需绑定可信事务记录与实际helper生命周期，再做接续，不能仅据时间自动抢占。
- 事务测试导致的失败残留 `rustdesk-transaction-invalid-26536`、`rustdesk-transaction-cas-22084` 已精确核对并清理；正常通过fixture自动清理。9项测试不代表完整安装流程、断电安装E2E或分支覆盖率80%。

最新活跃工作包为4；工作包3仍有本地健康/意图/R0–R6和自愈事件缺口。完整八项数量保持，未把源码底座通过换算成工作包完成。当前本轮后台构建任务均已结束，后续会话先查result/status确认。

### 用户新增授权：本机虚拟机环境（以本节覆盖“没有VM”状态）

用户已明确授权自行安装虚拟机软件、使用 `D:\ISO` 中镜像安装测试系统、管理测试快照并执行测试；用户随后已自行安装Oracle VirtualBox，剩余环境交由本轮处理。当前活跃工作包改为8，完整八个工作包仍保留。

- VirtualBox实际版本7.2.20r175154，VBoxSup驱动Running，宿主i9-14900启用VT-x/SLAT，宿主Hyper-V/VMP Disabled；无需宿主重启即能启动VM。
- 本轮官方EXE完成下载，SHA-256 `a81777d2b36380ce042a29e9c554cf032eb46a793f62e3cc82e7411e535c2c26` 与Oracle一致，Oracle签名Valid。用户并行已安装，本轮重复MSI最终1603（不是VirtualBox不可用）；以实际VBoxManage/驱动/VM运行状态为准，不再次启动安装器。
- VM根目录`D:\RustDeskAgentLab\VMs`，权限限制SYSTEM/Administrators；测试凭据文件`D:\RustDeskAgentLab\vm-test-password.txt`已限制权限，不提交、不放进源码或诊断日志。后续调用用passwordfile，并对unattended输出脱敏。
- Win10 VM `RustDesk-Agent-Win10`，UUID `a6c9edff-6159-468f-bdba-08720a153424`：用户指定 `D:\ISO\cn_windows_10_enterprise_ltsc_2019_x64_dvd_9c09ff24.iso`，检测10.0.17763.316，4核/4GB/80GB动态VDI/NAT。新磁盘`system-ltsc2019.vdi`；文件安装完成后首次启动期间savestate，等待Win11完成后恢复，不能标干净系统完成。
- 之前Win10 LTSC2021 ISO首次无人值守安装0x80070570，保存失败快照 `InstallFailure-0x80070570` UUID `45663ecd-c7d6-4ac3-ae23-1b437d3c0a85`，对应旧`system.vdi`。保留证据，不是可用基线。LTSC2021诊断DISM导出已停止、宿主E盘ISO已卸载；`agent-win10-materialize-iso`已生成实体诊断ISO，未使用。不得删用户原ISO。
- Win11 VM `RustDesk-Agent-Win11`，UUID `d91a13d9-2bb7-40bf-b5e4-9d57870cd242`：使用`D:\ISO\zh-cn_windows_11_enterprise_ltsc_2024_x64_dvd_cff9cd2d.iso`，检测10.0.26100.1742，4核/6GB/80GB动态VDI/EFI/TPM2/NAT。最新首次配置94%，自动重启中，Guest Additions尚未可用；没有干净快照。
- 截图当前模型不能直接读图，使用宿主Windows OCR脚本提取安装文字；这些进度不算验收PASS。宿主RustDesk/agent持续Running，未在宿主停删服务/防护配置。
- 客体计划先通过Guest Additions control通道执行`guest-baseline.ps1`（仅RDLAB-*、必须管理员），阻断生产rdapi/download/ID域名，再建立干净快照。当前尚未执行客体脚本、未安装客户端测试包、未测试真实远控。
- 测试包/Debug bundle在`D:\RustDeskAgentLab\artifacts`，`manifest.json`记录旧MSI与新DebugMSI/agent摘要；MSI与单独新agent可能冻结不同源码时点，分别核验，禁止混作最终发布版本一致性证据。客体安装显式`DEFENDER_EXCLUSIONS=0`，宿主真实Defender禁测持续有效。
- `D:\RustDeskAgentLab\README.zh.md`、`resume.json`、`guest-*.ps1`保存环境与固定客体驱动；语法检查通过但客体尚未运行。临时工具脚本`...\opencode\prepare-agent-vm.py`、`agent-lab-control.py`、`ocr-agent-vm.ps1`、`read-agent-vm-ocr.py`可续用。resume后必须先查实际VM状态，不盲目重装/恢复旧快照。

源码事务最新10项通过：补`stopping_services`持久停服意图，`may_stop_services(path,evidence)`仅磁盘快照与当前事务完全一致才授权停服；内存已推进但未保存不得停服。红/绿日志`agent-transaction-persisted-stop-red/green`，green exit0。该module仍只有测试入口、未接安装helper，不能把此bool当完整实际生命周期安全。

### 虚拟机验收完成情况与360取消指示（最新）

本节覆盖上节安装中状态：Win10实际10.0.17763.316、Win11实际10.0.26100.1742均完成安装、Guest Additions 7.2.20控制通道、管理员基线和硬盘冷启动验证。安装DVD已卸载，生产API/下载/ID域名在客体hosts隔离，干净快照均建立。Win10首次重启准备Windows长时间未完成，保留故障快照后离线硬盘启动成功；Win11恢复原干净快照时安装DVD导致重新进安装程序，恢复快照并卸载DVD后冷启动通过。

两系统实际执行旧standard MSI安装和新Debug MSI升级，退出码均0，双服务Running/LocalSystem，agent摘要从`02ec919f1a0cea4c282d9a05b5dec22ba2708c79915ac798b287690c57389010`替换为预期`38e85e278ffd0c471c96f50ebd97456869b7543db2b84f023901787b3c1552bc`。证据`D:\RustDeskAgentLab\win10-legacy-result.json`、`win10-new-result.json`、`win11-legacy-result.json`、`win11-new-result.json`。Win11首装后采集因agent隐藏/系统属性失败，适配Get-Item -Force后同包重装结果通过，不能把该before快照当首装前状态。身份采集需SYSTEM权限，未完成身份保留验收；真实远控、断电回滚、SOS/EXE和完整在线迁移仍未验收。没有真实Defender配置读写。

干净快照：Win10 `CleanOS-LTSC2019-Offline`（82889123-f059-43ca-ac6a-3b28c85b4361），Win11 `CleanOS-DiskBoot-Offline`（bccf5f7c-3623-436b-9865-b6ad525f8ddb）。升级后360前快照两台分别37e29878-7539-427f-9533-4bd6c8ef87fc、f4711c1e-856d-4745-98c6-1cd2964aaae7。完整八项账本保留；工作包8环境就绪，其他待验收项目不因MSI局部通过标完成。

用户随后授权VM内安装360并尝试自动信任，最新又明确“不能后台自动加就没必要测试了”。本轮只在Win10测试VM安装官方极速版15.0.3.1058，安装包来源https://weishi.360.cn/jisu/、奇虎签名Valid、SHA-256 `024e629ae67b58cf2c0a2fc4b1f35f3b2e9ee2d9837d31399038485f78938de9`。未确认个人版受支持的后台静默加入信任接口；官方界面需要自绘控件交互，信任入口需要在线组件。按用户最新指示立即停止界面加白和保留测试，两个文件未加入信任，没有自动配置实现；不得把未找到接口夸大为已证明绝对不可能。

取消现场保存快照 `Cancelled360-Inspection`（a5f1c068-7b86-4ab4-abb6-fe070985bb46），Win10恢复升级后360前快照，清除本轮安装和交互driver运行状态；Win11从未安装360。两台当前正常关机、离线。宿主RustDesk/agent保持Running，未停服、未安装360或测试MSI。仅新增本地lab测试资产与状态文档，本轮未接入业务运行路径、未提交或发布。

### 最终报告与本轮实际故障回归（更新）

用户要求剩余验收继续并形成文档，报告已写入`docs/agent-recovery-final-test-report.zh.md`。报告结论是当前候选未通过完整保护验收，不等于完整八项开发完成。

- 实际冷启动：两OS主服务Stopped，agentRunning；Win10 SCM7000/7009连接超时，未确定最终根因。实际Debug首页启动正常，不能推导远控成功。
- 原MSI同包重装将Stopped推导为STOP_SERVICE='Y'，仍无条件删除服务，后续跳过重建，exit0但主服务消失。TDD新增`tests/test_msi_stopped_service_preservation.py`先失败，只给`RustDesk.wxs`两条删除动作加条件；修复MSI SHA-256 `165a92846d122868e714ef7949396beabe67cf5d248184d135500bc4f2f65a92`，两OS实际Stopped重装注册/账户/路径保留通过。
- 实际双服务正常停止：Win10 45秒均Stopped，包未自动部署bootstrap任务，FAIL；人工恢复成功不是自愈PASS。
- 手工bootstrap：实际任务XML引号实体读回字面引号且省略默认Enabled=true，旧校验拒绝；增加红测试并修复等价比较，显式禁用/参数变化仍拒绝。MSI agent binpath没有引号但路径无空格，旧严格字符串误拒绝；新增红测试，仅接受无空格路径首对引号等价。最终实际Win10安装/幂等重装exit0，正常停止agent后独立bootstrap启动成功。
- 显式R1 fixture：仅VM启用service-recovery-enabled后主服务停止，16.05秒观察Running，恢复时间持久化，marker删除，installation_id不变。生产安装默认仍未开启该门禁。
- 最新真实Rust回归31+9+2+5+10=57通过；PHP65/30断言、HTTP12、实际浏览器1440/390回归通过。最新全仓exit101：WebRTC连接超时、取消缓存残留、terminal_helper架构doctest失败；启用webrtc单独复测连接仍失败、取消通过，零匹配命令明确不计PASS。最终fmt/Clippy仍失败。
- 有效前置损坏MSI复测：Win10真实退出1620，旧文件摘要不变、双服务继续Running，PASS；不代替中途回滚。原污染前置失败保留。
- 最新agent单独二进制SHA-256 `618451fba5b2c5813cd051dd76c0b29b1d21e9c459cb52df39264ceb218e0735`，与修复MSI冻结的agent不同，手工替换仅fixture。
- 证据根`D:\RustDeskAgentLab\report-evidence`与SHA-256索引。Win10回归快照10a13936-6e3b-448a-a5b8-6d505265864b、Win11回归快照3955224d-0834-4caf-905e-b57dc132d3d6；继续任务必须先读resume并检查实际VM状态。原完整八项范围保留，环境包8已完成，其余完整事务/多层fallback/四包在线迁移/真实远控/覆盖率未完成。

### 2026-10-11 严格TDD续做检查点（任务仍进行中）

用户明确要求现有任务栏十项完成才停止，已通过问题工具确认范围是“现有任务栏十项”。原八工作包持续保留，第9项360界面按既有取消决定处理，第10项报告需随最终实现更新，不能把下面模块测试绿阶段换算成十项完成。

- 启动复核：Win10和Win11各4轮（两次正常关机冷启动、两次普通重启）通过，双服务自动Running、无本启动周期SCM7000/7009/7031/7034。采样任务只读，不修复服务。证据`D:\RustDeskAgentLab\RustDesk-Agent-Win10-boot-matrix.json`、Win11同名矩阵，均completed/PASS。历史首次冷启动失败保留，根因未定，不能宣布全场景修复。Win10矩阵使用手工bootstrap，Win11没有该恢复入口，证据须区别。
- 状态写入：新增Windows专属`state_store.rs`，flush后MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)，旧正式状态不先移走，隐藏/系统属性在发布前设置；占用拒绝保留旧状态、未提交staging不覆盖、不提升。非Windows原路径保留。`write_state`薄接入，正常实例/代次/预算字段语义不变。
- 状态测试：6个有实质断言的用例含并发读和8次终止写进程；进程helper并入同一测试，不计空helper为能力。真实AgentState集成验证installation/session_generation；Win10实际服务`win10-state-store-result.json`3项PASS：已有实例保留、代次13→14→15、文件锁持有35秒摘要不变、释放后重启保存成功、staging无残留。实际部署agent SHA-256 `05563fa239974c2bf11f5253d3625e0ba3cee7d8f33cbbf311704178e211796d`。
- 组合决策：recovery5376个组合、bootstrap224个组合通过；目标Rust批次`agent-state-store-continuation-final`32+5+9+2+5+12+6+10=81项PASS。组合数量不当测试数量或覆盖率。
- 实际拒绝：Win10 `win10-bootstrap-boundary-result.json`5项PASS（批准摘要漂移、disable-agent、maintenance、错误edition、预算损坏），测试后原agent摘要和服务恢复；`win10-r1-boundary-result.json`5项PASS（门禁关闭、维护、用户明确停止、服务Disabled、允许恢复），配置逐字节恢复，测试标记删除。
- 覆盖：`D:\RustDeskAgentLab\state-coverage`LLVM实际生产state_store行83.87%、region87.30%、函数100%，branch未提供，不能宣称全agent业务分支80%。早期计7项含空fixture的证据为历史，最终6实质测试需重跑覆盖。
- 新事务模块目前**仅集成测试入口，未接生产dispatcher/CLI**：`install_plan.rs`（准备记录/目标/资产/不可变保存/停服绑定9项），`handoff.rs`（nonce/PID+原生FILETIME/锁释放回执6项），`workspace.rs`（SYSTEM/BA ACL创建及读回3项）、`resume_task.rs`（固定事务动作/当前UTC重试起点/真实任务读回/默认自闭合BootTrigger4项）、`package_gate.rs`（精确四target/固定下载来源/发布签名/包只读锁4项）、`helper_runtime.rs`（父helper双进程/互斥/签名复验/真实接续任务登记，3项）、`directory_backup.rs`（完整目录/漂移/缺件/逃逸5项）、`install_execution.rs`（副作用前落盘/成功/回滚/3010待重启/Pending不竞争/接续12项）、`installer_process.rs`（真实进程退出码与Pending创建时间，短进程用例正在重跑），`service_control.rs`（真实唯一临时SCM服务与进程退出2项）。各模块红阶段日志在opencode临时根，最新green名字可通过result检查。后续必须实际原生安装/产品登记恢复/健康探测接入，不能用mock Backend或布尔证据宣称真实回滚。
- 安全门禁仍缺：准备记录对主服务真实用户意图/全局维护标记及活动会话的运行核对、目标解包文件manifest和agent版本资源、旧MSI可用恢复包绑定、目录交换中断接续、执行结果/进程身份持久化、完整Windows backend、生产固定事务ID CLI、自更新批准bootstrap摘要与生命周期。R1默认未启用，不允许先创建标记绕过安装协调。
- 资源：宿主两个生产服务始终Running。当前Win10运行新state_store fixture agent，手工bootstrap摘要旧批准故意不一致，会拒绝恢复，已经作为拒绝测试验证；需走未来事务更新批准或精确恢复测试基线。Win11关机。只读开机采样任务`RustDeskAgentLabBootSampler`仍在两VM，继续矩阵用，收尾需要删除；不得误删产品Recovery任务。事务测试临时副本服务/唯一任务成功用例自动清理，失败目录须精确复核再清理。
