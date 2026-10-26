# Management Agent 全量测试与新机器执行手册

日期：2026-10-10

适用功能：Management Agent 救活、fallback、离线恢复、agent 自更新、旧版本在线迁移。

适用仓库：`D:\github\rustdesk`、`D:\github\rustdesk-api`。

执行环境：新安装的 OpenCode、Windows 真机测试机 win10-gs（原隔离 VM 已停用，见仓库 `AGENTS.md`「Windows 真机测试统一在 win10-gs」）、可用的本地 API 测试环境。

本文是可由新机器上的 OpenCode 按阶段读取和执行的测试规范。它不是“跑几个命令看是否编译通过”的清单，而是从测试先行、源码级测试、API/数据库集成、真实安装包、Windows 服务、故障注入、旧版本升级到提交前全量门禁的完整验收流程。

## 0. 新机器环境预检

预检必须先于任何代码修改、构建和破坏性测试。每项记录版本和结果；缺少工具只阻断依赖该工具的阶段，不得把整轮测试伪装成通过。

```powershell
git --version
rustc --version
cargo --version
python --version
php -v
node --version
npm --version
npx --version
go version
docker version
docker compose version
flutter --version
msbuild -version
nuget help
```

最低依赖分组：

| 阶段 | 必须具备 |
| --- | --- |
| L1 Rust | Rust toolchain、Cargo、网络/锁文件依赖缓存 |
| L2/L3 后台 | PHP CLI、Python、SQLite、FrankenPHP 或仓库规定运行时 |
| L4 broker | Go toolchain |
| L5 Playwright | Node、npx、`@playwright/test`、Chromium |
| L6/L7 Windows 包 | Windows x64、管理员权限、MSBuild、NuGet、WiX/构建依赖、win10-gs 真机 |
| L8 容器 | Docker Engine/Desktop、Compose |

检查仓库和工具路径：

```powershell
Test-Path "D:\github\rustdesk\AGENTS.md"
Test-Path "D:\github\rustdesk-api\AGENTS.md"
Test-Path "D:\github\rustdesk\Cargo.lock"
Test-Path "D:\github\rustdesk-api\playwright.config.js"
Test-Path "D:\github\rustdesk-api\stream-broker\go.mod"
```

如果 `node_modules`、Playwright 浏览器、Rust target、Flutter 依赖或 Docker 镜像尚未准备，先记录为 `ENV_PREPARE`，准备结束后重新执行受影响阶段；不能在半准备状态下生成最终 PASS。

## 1. 测试结论规则

### 1.1 最终结论只有四种

| 结论 | 含义 |
| --- | --- |
| `PASS` | 测试实际执行，预期结果全部满足，证据完整 |
| `FAIL` | 测试实际执行且有一项不满足；必须修复或明确阻断发布 |
| `BLOCKED` | 因环境、权限、工具、构建机或外部依赖无法执行；不能写成通过 |
| `NOT_APPLICABLE` | 经评审确认不适用，并写明原因；不能用来规避应测项 |

禁止使用“基本通过”“大部分通过”“编译成功所以通过”。失败路径必须断言错误类型、状态、旧数据保留结果和后续恢复能力。

### 1.2 任何测试都必须留下证据

每个测试记录至少包含：

- 测试编号、日期、执行人和 OpenCode 会话标识。
- 客户端仓库 commit、后台仓库 commit、构建版本和 `build_seq`。
- Windows 版本、架构、win10-gs 备份标签（替代原 VM 快照名）、standard/SOS、EXE/MSI。
- 测试前置状态、实际命令、输入参数和测试数据身份。
- 预期结果、实际结果、退出码、关键日志和截图/事件日志路径。
- API 响应、SQLite 直接查询结果、服务状态和文件摘要。
- 失败时的事务阶段、fallback 层级、旧版本/旧 agent 是否仍可用。
- 临时资源清理结果；未清理资源必须写明名称、路径和保留理由。

### 1.3 新机器 OpenCode 的第一步

新机器开始测试时，OpenCode 必须先读取：

1. `AGENTS.md`。
2. `docs/agent-recovery-upgrade-plan.zh.md`。
3. `docs/agent-recovery-test-runbook.zh.md`。
4. `docs/windows-agent-live-test.zh.md`。
5. `docs/windows-exe-test.zh.md`。
6. `docs/windows-msi-standard-test.zh.md`。
7. `rustdesk-api/AGENTS.md` 和 `rustdesk-api/docs/client-management-contract.md`。

然后确认当前任务属于以下哪条测试路径：

- `FRESH_INSTALL`：新机器全新安装。
- `LEGACY_UPGRADE`：已有旧版本在线升级，验证新 agent 能力迁移。
- `RECOVERY_FAILURE_INJECTION`：在已通过安装/升级的机器上做故障注入。
- `PRE_COMMIT_FULL_GATE`：代码完成后、提交前和发布前执行的全量门禁。

没有用户明确授权时，不向生产服务器部署、不修改生产数据库、不提交、不打 tag、不 push。

### 1.4 测试机角色与重启接续

区分控制/证据机（Mac）、Windows 被测机（win10-gs，原 VM 已全部停用）、隔离 API/SQLite 环境。真实远控需独立控制端，不能对同一 GUI 自连证明 E2E。

- 安装/升级、破坏文件、删服务和禁用系统组件均在 win10-gs 执行，前提是已完成完整备份并空跑恢复成功；断电/关机类注入必须能自动回到可 SSH 状态，否则记 BLOCKED。
- 断网不能切断唯一远控入口（termark SSH）；只能用限定目标、限定时长并自动复原的方式注入，先保存恢复方法。
- 重启前持久化 `resume.json`，包含 run_id、当前用例、备份标签、源码/包摘要、最后动作和下一步；恢复后先查实际状态，不盲目重做。
- 每次从备份恢复后核对逐文件哈希、属性、ACL、服务定义、测试身份（installation_id、`--get-id`、key_pair）和后台绑定；agent 启动代次不得回退，否则后台会以 409 拒绝上报。
- 证据复制到控制机再清理 win10-gs；不删除唯一备份；不得删除或改动 `D:\build-env`、`D:\github`。缺少可用备份或带外入口则破坏性项记 BLOCKED，继续独立低层测试。

## 2. 测试对象矩阵

### 2.1 安装包矩阵

必须覆盖以下四种目标：

| 编号 | edition | package kind | agent target key |
| --- | --- | --- | --- |
| `STD-EXE` | standard | EXE | `windows-x86_64-exe-standard` |
| `STD-MSI` | standard | MSI | `windows-x86_64-msi-standard` |
| `SOS-EXE` | sos | EXE | `windows-x86_64-exe-sos` |
| `SOS-MSI` | sos | MSI | `windows-x86_64-msi-sos` |

同一台机器不能在未清理的情况下切换 standard/SOS 或 EXE/MSI。`RustDeskManagementAgent` 服务名共用，混装会使测试结论失真。

`STD-EXE`、`SOS-EXE` 两包当前没有 R3 保底切换、R4 离线包修复和 agent 备份恢复路径（`local_repair_schedule.rs`、`rescue_assets.rs` 只接受 `windows-x86_64-msi-*`），依赖这些能力的用例在 EXE 包上记 `NOT_APPLICABLE`，原因写“实现仅支持 MSI”，不能记 `PASS`。

### 2.2 测试层级

| 层级 | 目标 | 必须通过的内容 |
| --- | --- | --- |
| L0 | 测试设计 | 失败条件、测试数据、清理方式和证据格式先确定 |
| L1 | Rust 单元测试 | 状态机、包选择、签名、hash、事务、退避、迁移和序列化 |
| L2 | 后台单元/静态测试 | PHP/Python/Go 语法、协议解析、边界校验、broker 单测 |
| L3 | API/SQLite 集成 | 真实 CRUD、真实数据库值、认证、防重、策略、命令和事件 |
| L4 | Rust/后台跨仓库集成 | 两端签名、字段、状态、版本、命令和事件一致 |
| L5 | Flutter/UI E2E | 管理页面、状态展示、确认、错误、响应式和持久化 |
| L6 | 实际 Windows 包 | EXE/MSI 安装、升级、服务、卸载、agent 和 bootstrap |
| L7 | 故障注入 E2E | 主程序救活、fallback、agent 自身恢复、事务接续和回滚 |
| L8 | 提交前全量门禁 | 全仓库格式、测试、静态检查、构建、包和文档证据 |

低层测试不能替代高层验收。Rust 单测通过不能证明 MSI 安装、Windows 服务恢复或真实远控成功。

### 2.2.1 测试编号

所有能力清单中的 `test_case_ids` 必须引用以下稳定编号；新增能力必须新增编号并补充预期结果：

| 编号范围 | 内容 |
| --- | --- |
| `AG-UNIT-*` | agent 状态、包、签名、事务、退避和序列化单测 |
| `AG-API-*` | agent API、认证、公钥、策略、命令、事件和 SQLite |
| `AG-BROKER-*` | stream broker 认证、容量、snapshot 和回退 |
| `AG-CONTRACT-*` | RustDesk 与 API 固定签名/JSON/版本兼容 fixture |
| `AG-FRESH-STD-EXE-*` | standard EXE 全新安装与能力 |
| `AG-FRESH-STD-MSI-*` | standard MSI 全新安装与能力 |
| `AG-FRESH-SOS-EXE-*` | SOS EXE 全新安装与能力 |
| `AG-FRESH-SOS-MSI-*` | SOS MSI 全新安装与能力 |
| `AG-UPGRADE-STD-EXE-*` | standard EXE 旧版迁移 |
| `AG-UPGRADE-STD-MSI-*` | standard MSI 旧版迁移 |
| `AG-UPGRADE-SOS-EXE-*` | SOS EXE 旧版迁移 |
| `AG-UPGRADE-SOS-MSI-*` | SOS MSI 旧版迁移 |
| `AG-RECOVERY-R0-*` 到 `AG-RECOVERY-R6-*` | 主程序救活阶梯 |
| `AG-BOOTSTRAP-*` | agent 自身 SCM/bootstrap/任务恢复 |
| `AG-INTERRUPT-*` | 事务中断、断电、重启、接续和回滚 |
| `AG-REMOTE-*` | 真实远控连接与主程序能力 |
| `AG-GATE-*` | 提交前、构建前、发布前全量门禁 |

### 2.3 能力清单与需求共同确定全量范围

“全部 agent 能力”和“全部主程序能力”不能通过一句“功能正常”验收。实现完成时必须在构建产物旁生成版本化能力清单，例如 `capabilities.json`，并由新 agent 的 status 上报返回同一 `capability_manifest_version` 和实际 `capabilities`。测试以这份清单为准，不能手工漏测能力。

能力清单至少包含：`id`、`version`、`component`、`risk`、`requires_network`、`requires_admin`、`interrupts_session`、`test_case_ids`、`minimum_package_kind`。能力清单必须经过构建检查，不能由测试人员事后编写一份与代码脱节的列表。

agent 能力清单至少覆盖：

| 能力组 | 必测能力 |
| --- | --- |
| 自身生命周期 | SCM failure action、周期 bootstrap、服务修复、二进制备份恢复、事务接续、单实例（由 SCM 服务模型与计划任务 `IgnoreNew` 保证，副作用段由安装互斥串行，不是进程级锁）、卸载不复活 |
| 观测 | 服务状态、binpath、账户/启动类型、文件完整性、版本/edition/package kind、意图、迁移、事务、rescue readiness |
| 本地恢复 | R0 IPC 重连、R1 启动/重启、R2 服务修复、R3 保底切换（仅 MSI）、R4 离线包（仅 MSI）、R5 在线固定目标、R6 退避与错误可见性（熔断与告警未实现，见 9.0 用例） |
| 远程命令 | inspect、start、restart、repair、reconcile、verify、reinstall、update、diagnostics、network probe、agent update、client enable |
| 安全 | Ed25519 签名、nonce、防重、时间窗口、身份/实例/代次绑定、包签名/hash/size、ACL、路径/重解析点检查 |
| 事务 | claim、状态转换、迁移、helper 接管、断电接续、回滚、outbox、重复事件、结果补报 |
| 配置策略 | policy revision、主动停用、本地禁止、远程暂停、权限、熔断、退避、活动会话策略 |

主程序能力清单至少覆盖：

| 能力组 | 必测能力 |
| --- | --- |
| 生命周期 | Windows 服务创建/启动/停止/重启、开机启动、未登录桌面运行、正常退出和异常退出 |
| 在线链路 | rendezvous 注册、在线状态、断线重连、ID/UUID 保持、API heartbeat、sysinfo、network payload |
| 更新链路 | policy stream、短轮询回退、check、download、manifest 选择、EXE/MSI 安装、成功/失败/回滚/重启建议 |
| 远控会话 | 连接建立、画面/视频、键盘鼠标、剪贴板、文件传输、音频、权限/UAC、会话断开和重连 |
| 配置与意图 | `stop-service`、服务权限、固定配置、策略应用、更新策略 revision、主动停用和重新启用 |
| 安装迁移 | 正常安装、卸载、保留 agent、旧版在线升级、ProgramData agent 替换、bootstrap、身份和配置迁移 |

只有产品/平台明确不支持的能力可记 `NOT_APPLICABLE`；缺少设备、多屏、音频、权限或驱动工具属于 `BLOCKED`。能力范围来自当前源码、UI 和发布契约；上表是最低集，不允许缩减清单漏掉原有能力。构建支持与当前进程可执行分别记录；暂时关闭不能伪装不支持。

### 2.4 安装/升级与系统矩阵

四种包 × 全新安装/旧版升级，共 8 条；Windows 10/11 各执行基础八条，共 16 条。无第二个系统时明确该 OS 未覆盖。升级来源覆盖实际在用版及支持范围最早版，二者相同可合并，旧包必须保留实际文件摘要。

另测旧 state 缺字段、旧 agent 停止/401、客户端保底目录运行、主动停用、普通/管理员安装、无桌面登录。同版本重装、同版本更高 build_seq、连续两次升级、回滚再升级、重复命令和卸载再安装有独立用例。跨 EXE/MSI、standard/SOS 是拒绝路径。

升级必须走“后台命令 → 旧程序 → 下载 → 新包 → 新 agent”真实链路；手工启动新包只能算安装测试，不能替代在线升级。

## 3. TDD 强制流程

### 3.1 行为拆解

每一项新能力先写成行为表：

| 行为 | 前置条件 | 触发 | 预期结果 | 失败结果 |
| --- | --- | --- | --- | --- |
| 服务停止自动救活 | 意图允许，agent 可运行 | 停止 `RustDesk` | 服务恢复（SCM `Running`）、R1 事件上报；主程序 ready 需另行核对，agent 只确认 SCM 状态转换 | 保留旧文件、记录错误、按预算退避 |
| 主安装损坏切换保底 | 保底包可信（仅 MSI） | 删除主安装 | 保底服务启动并标记 degraded | 不启动不可信文件 |
| 离线包恢复 | API 不可达，本地包可信（仅 MSI） | 两个客户端路径缺失（只识别缺失，不识别内容被篡改） | 从本地包恢复 | agent 保持可管理，不循环重装 |
| 旧版在线迁移 | 旧主程序可更新 | 执行一次在线升级 | 新 agent 实际运行并上报全部能力 | 旧 agent/主程序可回退或接续 |
| agent 自身崩溃 | bootstrap/SCM 可用 | 终止 agent | agent 恢复 | 有界重试；单实例由 SCM 服务模型与计划任务 `IgnoreNew` 保证 |

没有行为表，不开始实现或手工验收。

### 3.2 先写失败测试

代码修改前必须新增或修改测试，使测试先失败。失败原因必须是“需求尚未实现”，不能是测试环境配置错误。

最低要求：

- 纯逻辑用 Rust `#[test]` 或 `#[tokio::test]`。
- API 用本地 SQLite fixture 和真实 HTTP 请求。
- broker 用 Go 测试服务器和批量 snapshot 测试。
- 页面用 Playwright 场景。
- Windows 行为用真实程序/安装包在 win10-gs 上驱动。

### 3.3 最小实现、回归和覆盖率

实现使新增失败测试通过后，再补充正常、边界、失败、并发和恢复路径。新增业务分支覆盖率目标不低于 80%，不能通过排除文件、删除测试、空过滤器或只跑单个快乐路径达到目标。

每次修复都要：

1. 重现原失败。
2. 先确认测试失败。
3. 实现最小变更。
4. 运行局部测试。
5. 运行跨模块测试。
6. 执行本手册第 10 节提交前全量门禁。

## 4. 客户端 Rust 测试

### 4.1 纯逻辑覆盖

新增或修改 `src/management_agent.rs`、`src/management_agent_bin.rs`、`src/platform/windows.rs`、`src/updater.rs` 和 `libs/base/src/update.rs` 时，必须覆盖：

- `Edition`、`PackageKind`、target key 和正常/保底路径选择。
- 缺失 target、错误 edition、错误包类型和不兼容架构。
- manifest schema、签名 key id、签名值、SHA-256、大小和 URL host 校验。
- primary、允许 mirror、重定向越界和非 HTTPS。
- `retry_stage`、指数退避、抖动、熔断、成功复位和时钟回拨。
- 服务状态：absent、stopped、start pending、running、stop pending、paused、unknown、marked for deletion。
- 服务 binpath、账户、启动类型、edition/package-kind 参数的精确解析和修复。
- `stop-service` 有效、缺失、损坏、权限拒绝和 intent unknown。
- `disable-recovery` 本地优先、服务端策略关闭、远程启用不覆盖本地禁止。
- R0 到 R6 fallback 的进入条件、成功终止升级、失败进入下一层和总预算。
- 同一安装事务的状态转换、非法回退、重复启动、断电接续和回滚。
- `migration_id`、`transaction_id`、`boot_id`、sequence、installation_id 的持久化和恢复。
- 身份读取失败不得生成新 ID/密钥冒充恢复。
- 命令正文摘要、重复命令、过期、未知类型/version/args、claim 和终态。
- outbox 重试、重复事件、乱序事件、终态后旧事件和 API 不可达。
- capabilities 和 rescue readiness 的序列化、未知字段兼容和旧 state 迁移。

每个失败断言至少检查错误分类或状态，不只检查 `is_err()`。

### 4.2 客户端最小命令

`management_agent` 通过独立的 `rustdesk-management-agent` binary 编译，不是 `rustdesk` lib 中可直接按模块过滤的测试。在 RustDesk 根目录执行：

```powershell
cargo test --locked --bin rustdesk-management-agent -- --nocapture
cargo test --locked -p base -- --list
cargo test --locked --workspace --no-fail-fast
```

先用 `--list` 确认 `base` crate 的实际 update 测试名称，然后执行匹配的测试；必须确认匹配数大于零。`--list` 只能用于发现，不能替代实际测试。agent 测试必须以当前 `Cargo.toml` 的 binary target 名为准。

### 4.3 Rust 并发与持久化测试

必须增加测试证明：

- 两个 agent 检查线程不能同时执行安装。
- helper 和 bootstrap 竞争时只有一个事务所有者。
- mutex abandoned 后优先恢复原事务，不创建新安装。
- 状态文件写入中断不会留下不可解析且无备份的状态。
- outbox 写成功但 HTTP 失败时只重试上报，不重做动作。
- HTTP 成功但本地结果写入失败时进入 reconciliation_required。
- API 返回旧 revision 时不覆盖新状态。
- agent 重启后动作预算、熔断和迁移阶段仍保持。

## 5. 后台 API、SQLite 和 broker 测试

### 5.1 PHP/Python/Go 静态检查

在 `D:\github\rustdesk-api` 执行：

```powershell
php -l sqlite/index.php
php -l sqlite/lib.php
python -m compileall tests tools
```

Go broker 位于独立 module，工具调用以 `stream-broker` 为工作目录执行，独立保存退出码：

```powershell
go test -count=1 ./...
```

实际目录若有更多 PHP、Python 或 Go 文件，必须对受影响目录递归检查。Go broker 的测试必须实际执行，不接受 `no test files` 作为覆盖证明。

### 5.2 SQLite 集成测试

所有新增接口必须在 disposable SQLite 数据库上测试，不能只检查 HTTP 状态码。每个场景都要同时断言：

- HTTP 状态和 JSON 字段。
- SQLite 的行、列、值、类型、时间、JSON、BLOB 大小和行数。
- 事务成功/失败后的旧数据保留结果。
- 重启 API 后数据仍存在。
- 旧 schema 迁移后旧客户端字段仍可读。
- 未知 JSON 字段在 round trip 中保留。

至少覆盖：

- agent enrollment、已登记公钥、密钥变化、撤销、重新授权。
- 时间偏差、nonce 重放、签名路径、正文 hash、身份 mismatch。
- agent policy 启用/禁用、revision 冲突、旧 agent 读取和 stale runtime 回写。
- agent status 最新投影、sequence、boot_id、installation_id 和旧状态拒绝。
- 命令创建幂等、批量创建、过期、claim、重复 claim、同 ID 异内容。
- 命令终态、事件 outbox、重复事件、乱序事件、attempt 和 transaction_id。
- package resolve 对 product、edition、arch、package_kind、target_key、build_seq 和摘要的校验。
- agent 状态不污染 `device_reports.last_heartbeat` 和主程序 presence。
- 删除设备、撤销绑定、旧命令和旧 outbox 的处理。
- 管理员会话、CSRF、权限、设备身份、批量操作和审计事件。

当前测试文件不能用宽泛 `*_test.py` 自动发现：这会包含处于冻结状态的 MySQL suite。逐个执行适用的 SQLite 测试，并记录各自测试数：

```powershell
python tests/integration_test.py -v
python tests/upgrade_script_test.py -v
python tests/provisioner_test.py -v
python tests/offline_bundle_test.py -v
python tests/installer_test.py -v
python tests/docker_entrypoint_test.py -v
```

`integration_test.py` 默认 `/tmp/rustdesk-local-runtime/frankenphp`，Windows 不能假设存在。其非 frankenphp 分支支持 PHP CLI，可将 `FRANKENPHP` 指向本机 PHP 绝对路径，先检查 sodium/pdo_sqlite：

```powershell
$env:FRANKENPHP = (Get-Command php -ErrorAction Stop).Source
php -r "exit(extension_loaded('sodium') && extension_loaded('pdo_sqlite') ? 0 : 1);"
python tests/integration_test.py -v
```

PHP 单进程开发服务器不能代替 Nginx/broker/并发容器验收。执行 `.sh`/POSIX fixture 的 suite 应在 WSL/Linux 环境运行，原生 Windows 不支持记 BLOCKED。新增测试显式追加文件清单，继续排除冻结 MySQL。

### 5.3 后台 Playwright 和容器命令

当前 Playwright 配置位于 `playwright.config.js`，测试目录为 `tests/e2e`，仓库根目录没有 `package.json` 固定测试脚本。新机器先确认 Node、npm/npx 和 `@playwright/test` 已安装，再执行：

```powershell
$env:RUSTDESK_TEST_URL = "http://127.0.0.1:17000"
$env:RUSTDESK_ADMIN_PATH = "/ops-x9"
# 替换为已确认属于本轮的真实隔离容器名称。
$env:RUSTDESK_API_CONTAINER = "rustdesk-agent-test-api"
npx playwright test --config=playwright.config.js
```

必须覆盖 `tests/e2e/admin.spec.js`、`tests/e2e/installer-generator.spec.js` 的全部现有场景，并新增 agent 详情、状态、恢复命令、失败状态、刷新持久化和 390px 场景。不能只执行单个 title。依赖未安装时记录 `BLOCKED`，安装项目锁定版本后重试。

Compose 检查使用 disposable env fixture，不能把生产 secret 放入命令或证据：

```powershell
docker compose --env-file .env.test -f docker-compose.yaml config --quiet
```

随后用唯一项目名启动测试容器，完成内部端口 80、配置管理路径、public fallback、SQLite volume 重启，再只清理本轮资源。没有 Docker 时记录 `BLOCKED` 和本地 HTTP/SQLite 替代证据。

现有 admin.spec.js 使用 docker exec 核对 SQL，必须设置实际 `RUSTDESK_API_CONTAINER` 和 fixture 管理用户；仅 URL 不足以运行。部分已有页面测试直接注入数据库事件，只验证 UI/数据库；另需真实 agent 执行动作再回报的全链路 E2E。

当前 Compose 含固定 container_name/network 和相对数据卷，仅 `-p` 不够隔离。实施测试驱动须生成本轮专用 Compose，使用本地待测 image、唯一名称/端口/网络和 disposable 数据卷；不得挂载生产 data 或用远端 latest 替代待测源码。`.env.test` 是待创建 fixture，当前仓库不存在。

仓库目前无 Playwright package lock，实施阶段必须固化测试依赖版本和恢复入口；不能让 npx 下载不确定最新版本。只读预检若缺该资产，报告缺口，不宣称命令即开即用。

### 5.4 stream broker 测试

必须覆盖：

- 认证 agent stream 和未认证旧 stream 的权限差异。
- agent stream 的容量、单设备连接数、批量大小、超时和 Retry-After。
- agent 命令不能通过未认证 stream 下发。
- broker 重连、snapshot 失败、PHP 500、API 429 和 stream 生命周期。
- policy revision、command cursor、重复事件、过期命令和断线补发。
- agent 长轮询不能占用 PHP-FPM worker。
- broker 重启后客户端回退短轮询，命令不丢失、不重复执行。

## 6. RustDesk 与后台跨仓库契约测试

### 6.1 固定协议向量

客户端和后台必须共享固定 fixture，至少包括：

- GET `/rd/update/v1/agent/policy`。
- POST `/rd/update/v1/agent/enroll`。
- POST `/rd/update/v1/agent/status`。
- GET `/rd/update/v1/agent/commands`。
- POST command claim。
- POST agent events。
- package resolve。
- 旧 `/rd/update/v1/check`、`/rd/update/v1/events` 和 policy stream。

每个 fixture 包含请求方法、逻辑路径、查询参数、正文、canonical payload、签名 headers、响应 JSON、错误码和数据库预期状态。

### 6.2 兼容性矩阵

| 客户端 | 后台 | 必测结果 |
| --- | --- | --- |
| 旧客户端/旧 agent | 新后台 | 旧 policy、check、events 继续工作；不能收到新命令 |
| 新客户端/新 agent | 新后台 | 新状态、命令、事件和迁移完整工作 |
| 新客户端/新 agent | 旧后台 | 明确降级为旧能力；不因未知字段崩溃，不宣称新能力 |
| 旧客户端 | 新后台策略关闭 | 原旧策略语义保持 |
| 新 agent | API 401/5xx/断网 | 本地恢复继续运行，结果 outbox 保留 |

未知字段、旧 schema、旧 state、旧 event 和缺少可选字段都要测试。新 agent 不得把旧后台返回的未知字段当作可执行命令。

## 7. 全新安装路径 `FRESH_INSTALL`

### 7.1 每个组合的前置清理

在 win10-gs 上执行（先完成完整备份）。必须确认：

- 没有 `RustDesk`、`RustDesk Service`、`RustDeskManagementAgent` 服务。
- `C:\Program Files\RustDesk` 和 `C:\ProgramData\RustDesk` 已清理。
- 没有旧计划任务、RecoveryBootstrap、临时 helper 和残留安装注册表。
- 备份标签和安装包 hash 已记录。

只能使用当前测试轮创建的清理资源。不得删除用户已有数据或生产资源。

### 7.2 安装和基础能力

对 `STD-EXE`、`STD-MSI`、`SOS-EXE`、`SOS-MSI` 各执行：

1. 安装并记录退出码。
2. 核对 RustDesk 和 agent 服务存在、账户、启动类型、状态和完整 binpath。
3. 核对 agent `edition`、`package_kind`、`target_key`、正常安装路径和保底路径。
4. 核对主程序、agent、bootstrap、任务和缓存文件摘要。
5. 等待 agent 至少两个检查周期，确认 state 没有被循环改坏。
6. 确认 agent policy 签名请求成功，后台记录正确身份和公钥。
7. 确认主程序 heartbeat/sysinfo、版本和 package identity 正确进入后台。
8. 通过真实远控连接验证画面、键鼠、剪贴板、文件、音频/权限等该版本声明的能力。
9. 确认 agent 状态不会伪造主程序 heartbeat，页面分别显示两种状态。

### 7.3 全新安装能力清单

- service.inspect、service.start、service.restart、service.repair。
- service 停止后的 R1 自动救活。
- binpath 错误后的 R2 修复。
- 主安装缺失后的 R3 保底切换（仅 MSI；不识别“文件在但内容被篡改”）。
- 本地批准包 R4 恢复（仅 MSI）。
- API 正常和 API 不可达时的 R5/R6 行为。已知限制：自动 R5 无镜像回退、有 `rescue\current` bundle 时不运行；R6 只有有界退避，无熔断态与告警。
- agent 进程崩溃、服务停止、服务删除后的 bootstrap/SCM 恢复；agent 文件“缺失”可恢复（仅 MSI、同 build），文件“损坏”不恢复。
- 本地禁止、主动停用、远程暂停恢复和卸载后的不复活。
- 状态、事件、outbox、重启和后台页面投影。已知限制：本地恢复事件只有 R1/R3/R4，R0/R2/自动 R5/R6/bootstrap 只写 `agent.json` 或日志，不发事件。

## 8. 旧版本升级路径 `LEGACY_UPGRADE`

### 8.1 升级前基线

每个组合必须从真实已安装旧版本开始，不用全新安装代替。记录：

- 旧客户端文件版本、build_seq、agent 文件版本和实际运行 PID。
- EXE/MSI、standard/SOS、正常安装路径和 package kind。
- RustDesk ID、UUID、公钥指纹、配置文件摘要和 policy revision。
- 旧 agent 支持的接口、旧 state schema、是否有保底目录和本地恢复包。
- 旧主程序是否在线、agent 是否在线、后台设备记录和最近 heartbeat。

### 8.2 一次在线升级

1. 后台发布固定目标版本和精确 target key。
2. 旧程序接收既有命令；测试驱动保存旧 command/transaction ID，新包入口建立 migration_id 并关联，不能要求旧版先实现新字段。
3. 记录原文件、身份和旧 agent 基线；首次迁移 bootstrap 可不存在，新包必须在首次停服前部署接续入口和回滚资料。
4. 旧程序启动下载目标包，包内新 `--update`/MSI 入口部署独立 helper 并执行 bridge。
5. 主程序更新、ProgramData agent 替换、服务参数更新和 bootstrap 安装必须在同一迁移事务中协调。
6. 新 agent 启动并读取旧 state；禁止静默生成新 ID/UUID/密钥。
7. 新 agent 完成后台登记、status 上报和 capabilities 上报。
8. 真实重启后再次核对版本、服务、任务、身份、能力和远控连接。
9. 后台只有收到所有必要完成阶段后才能显示迁移完成。

### 8.3 升级中断矩阵

在 win10-gs 上逐项中断并重启（每项前确认备份可用，结束后恢复并核对身份）：

- 下载完成但安装未开始。
- 主服务停止前。
- 主服务停止后。
- 主程序文件替换前后。
- agent 停止前后。
- agent 文件替换前后。
- bootstrap 注册前后。
- 新 agent 首次启动前后。
- identity bound 前后。
- capabilities 上报前后。
- complete 落盘前后。
- HTTP 结果上报前后。

每个中断必须判断接续、回滚或 degraded 是否正确，不能只看进程最终 Running。

### 8.4 升级成功的硬门槛

旧版升级只有同时满足以下条件才是 `PASS`：

- 主程序和 agent 都是目标构建，实际运行文件摘要正确。
- ProgramData agent 已被替换，不能只看到 Program Files 主程序更新。
- 服务 binpath、账户、启动类型、edition、package kind 正确。
- ID、UUID、公钥指纹和原配置保持不变。
- 新 agent 实际签名上报并被后台绑定。
- capabilities 覆盖该构建声明的全部适用能力。
- RecoveryBootstrap、离线包、保底客户端和事务状态符合 readiness。
- 重启后仍可管理，主程序和 agent 都能继续上报。
- 至少一次真实远控连接通过。

任一条件不满足，结果是 `FAIL` 或 `BLOCKED`，不能写成“主程序升级成功”。

## 9. 故障注入和 fallback 测试

下面是验收流程，不是已存在的自动化脚本。新接口、bootstrap、capabilities 和 migration 目前仍属方案；未实现时对应测试记 FAIL（缺能力）或 BLOCKED（依赖未具备），不能临时模拟能力给出 PASS。实施交付须提供受版本控制的 fixture/驱动/测试入口，将这些步骤绑定到真实程序。

### 9.0 带编号的验收用例

每个用例从已验证的备份状态开始、结束后恢复，前置必须包括精确包/身份、意图、最后可用资产、后台策略、当前没有活跃事务。除用例显式关闭的条件外，不手工运行 `check-now`、安装器或 Start-Service 帮产品恢复。

在已装生产 agent 的 win10-gs 上跑 `#[ignore]` 真机测试二进制时，测试构建取的是 `Global\RustDeskAgentTestInstallGuard`，与生产 agent 的 `Global\RustDeskManagementAgentInstall` 互斥名不同，两者不会互斥。不先规避就会让测试动作与生产 agent 的 R1–R4 并发修改服务/安装目录，故障注入结论作废。前置二选一：停用生产 agent 的自愈，或置 `maintenance.json`。

| 编号 | 操作 | 必须观察与断言 | 截止/复原 |
| --- | --- | --- | --- |
| AG-RECOVERY-R1-001 | 直接 SCM 停主服务，不写主动停用。前置无需手建 `service-recovery-enabled`：agent 服务启动时在该标记缺失且门禁允许（意图 `Enabled`、无 `disable-agent`/`maintenance.json`/purge 标记）的情况下会自动补写 | agent 自动启动；原 ID/UUID/配置不变；只有必要动作；恢复成功的判据是 SCM `Running`，主程序 ready 另测 | 180 秒初始目标；复原不能靠删标记——删掉后下一次 agent 服务启动会重新写入，要关闭恢复只能改意图或置本地禁止标记 |
| AG-RECOVERY-R2-001 | 停服并将 binpath 改为不存在路径 | 恢复原固定契约、参数与账户；无任意路径执行；判据为 SCM `Running`，主程序 ready 另测。已知限制：R2 只写 `agent.json` 的恢复尝试/结果，不发本地恢复事件 | 实测门限，不反复删建 |
| AG-RECOVERY-R3-001 | 隔离 API/下载，停服并使主安装缺失（仅 MSI；主安装目录与 `RustDesk.exe` 都在、只是内容被替换时 R3 不切换） | 完整保底启动，degraded；文件/版本来源正确 | 恢复网络后真实远控 |
| AG-RECOVERY-R4-001 | 主/保底不可用，网络隔离，保留批准包（仅 MSI） | 本地修复无下载依赖；agent 最终恢复；身份保留 | ≤15 分钟试点目标，不手工救援算通过 |
| AG-RECOVERY-R5-001 | 本地包无效，允许网络、主对象返回 404 | 已知限制：自动 R5 只取 manifest 的 `target.primary`，不读镜像；404 后按 5/15/30/60 分钟退避重试同一 primary。后果是 primary 长期 404 时永远用不到同域备用路径或镜像，直到 primary 恢复或运维下发远程命令；设备上存在 `rescue\current` 签名 bundle 时自动 R5 整体不运行，该场景直接无动作。镜像回退只在远程命令路径生效；接入自动路径并补 R4→R5 跃迁需要新阶梯规则，须维护者批准。本用例按现码预期 `FAIL` | 按下载/安装预算，恢复 fixture |
| AG-RECOVERY-R6-001 | 使所有客户端恢复选项不可用，保留 agent | 有界退避（30/120/300 秒后固定 3600 秒，无次数上限）、准确错误、agent 联络保留。后果是后台看不到“已熔断/冷却至何时/需要人工”，只有一个 `service_recovery_error` 字符串。熔断态、冷却期与告警事件均需新增状态与生命周期规则，须维护者批准；本用例按现码只验收退避与联络保留 | 观测退避阶梯，不宣称熔断 |
| AG-BOOTSTRAP-001 | 终止 agent，再使主程序故障 | SCM/bootstrap 恢复 agent 后执行主程序自愈。无双实例的取证以 SCM 报告的 PID 加进程列表比对，不是查找互斥对象 | agent ≤7 分钟试点目标，不能同存双实例 |
| AG-BOOTSTRAP-002 | 分两子场景：①删除 agent exe（MSI、同 build、门禁打开）②agent exe 存在但哈希不符 | ①本地固定契约/批准备份恢复，原身份不变，应 `PASS`；②bootstrap 记 `agent integrity unavailable; no start attempted`，既不启动也不替换，属已知限制，本子场景预期 `FAIL` | 不临时下载未知 exe |
| AG-BOOTSTRAP-DTF-001 | 正常停止主服务与 agent；文件和注册保留，不调用 sc start | 定时/开机入口先启动 agent，再自动恢复主服务；主心跳/agent 联络分别恢复 | 开机和周期两条路径各测，无人工启动算通过 |
| AG-BOOTSTRAP-DTF-002 | 恢复批准文件后服务仍 stopped；启动再次被拒绝/启动类型 disabled | 文件存在不伪造在线；启动检查与有界退避；disabled 不擅改。已实现的是“不擅改 + 无动作”：agent 服务为 `Disabled` 时 bootstrap 直接 `Observe`，计划任务退出码为成功、不写预算、不留错误记录。要求从产品侧取得“明确错误”的那一半属已知限制 | 用隔离 fixture，不操作安全软件排除项 |
| AG-INTERRUPT-001 | helper 停 agent 后注入崩溃，再重启 win10-gs | 事务接续/回滚；服务、版本、配置均核对 | 不人工补 agent；控制台留证 |
| AG-UPGRADE-STD-EXE-001 等四包 | 后台真实命令触发旧版到新包 | 主/agent 实际文件和运行版本；能力、任务、ID/UUID/pk 一致 | 重启复测，八路径各独立报告 |
| AG-API-001 | 重投同命令/claim，丢掉返回 ack | 一次副作用；稳定事件补报；SQLite 行数/摘要一致 | 不凭界面成功判通过 |
| AG-API-002 | 注入签名/时间/nonce/身份错误 | 明确错误码；不执行、不污染可信在线；旧数据保留 | 复原后合法请求正常 |
| AG-API-003 | status 到达顺序 10、12、11，切换 boot/实例 | 新投影不倒退；旧实例拒绝；重启后 sequence 不重用 | API/SQLite/页面三者一致 |
| AG-CONTRACT-001 | 新后台配旧程序/agent，再回退 API | 旧接口正常；新接口显式不可用；本地恢复及 outbox 留存 | 所有 pair 的 fixtures 留证 |
| AG-REMOTE-001 | 控制端连接测试桌面，输入随机 token | 动态画面确有更新；鼠标坐标/键盘输入可在被控端确认 | UI截图＋双方日志，非静态截图 |
| AG-REMOTE-002 | 双向剪贴板、多文件含 Unicode/空文件 | 内容一致；传输前后 SHA-256 一致；拒绝权限路径准确 | 只使用本轮文件 |
| AG-REMOTE-003 | 锁屏、登录切换、UAC/提权 | 正确桌面画面与输入、授权语义、服务继续运行 | 需独立控制端/人工可验证交互 |
| AG-REMOTE-004 | 活动会话期间更新/恢复命令 | 默认 deferred；会话结束后重评估；不切断健康会话 | 记录会话数及前后路径 |
| AG-GATE-001 | 读取能力目录与测试结果索引 | 每项适用能力都有真实用例/证据，无未测项 | 任一缺项阻断完整保护 |

CASE-ID 中的四包升级应分别展开为 `AG-UPGRADE-STD-EXE-001`、`AG-UPGRADE-STD-MSI-001`、`AG-UPGRADE-SOS-EXE-001`、`AG-UPGRADE-SOS-MSI-001`。表中未列的能力依第 2.3 节追加，不把代表用例当作全量结束。

所有远控会话能力都覆盖启用正常、关闭后拒绝、失败可见及重连；文件、剪贴板、音频、提权不能仅断言“按钮可点击”。多屏/显示切换、分辨率、隐私模式、远程打印/隧道等如当前产品支持，也追加实际 E2E；无硬件/驱动则 BLOCKED，不因本次未改这些模块删掉回归。

### 9.1 R0–R6

按顺序测试，并验证成功后不继续进行更高破坏性动作：

1. R0：业务进程仍在但注册卡住，测试 IPC 重连。
2. R1：停止主服务，等待自动启动/重启。
3. R2：在 win10-gs 上删除服务或改为确定不存在的测试 binpath，等待修复；不指向真实第三方程序。成功判据是 SCM `Running`。
4. R3：破坏正常安装但保留可信完整保底客户端，验证降级启动。只识别主安装目录或 `RustDesk.exe` 缺失；目录与文件都在、内容被替换时不会切换。仅 MSI 包具备该路径。
5. R4：断网并破坏主/保底客户端，用救援包内的 MSI 本地修复。仅 MSI 包具备该路径。前提是 Windows Installer 已登记的 MSI 源（ProductCode 与包名）和本机 Agent 哈希都与救援包一致：客户端更新器升级 MSI 后旧救援包的 ProductCode 已不在机器上，修复会报 `installed MSI source query failed: 1605`；20261026.2 起首装线程会在 Agent 构建号高于救援包时自动轮换救援包，轮换完成前救援状态报 `bundle_build_mismatch`。
6. R5：恢复网络但让 API 失败、下载主路径失败、镜像路径成功。已知限制：自动路径只取 `target.primary`，不读镜像；有 `rescue\current` bundle 时自动 R5 整体不运行。按现码该用例预期 `FAIL`。
7. R6：使主/保底/包资产不可用，但保留 agent、bootstrap、身份及证据，验证退避与错误可见性。已实现的是有界退避（30/120/300 秒后固定 3600 秒）与 `service_recovery_error`，没有熔断态、冷却期和告警事件。连 agent 全删属于不能自恢复的边界，不能要求仍上报。

### 9.2 agent 自身

- `taskkill` agent 进程，验证 SCM failure action 和 bootstrap。
- 停止 agent 服务但不删除主程序，验证启动器按意图恢复。
- DTF：同时正常停止两个服务，在文件恢复状态下等待周期入口；另做重启入口，确认无需用户登录或手工 sc start。
- DTF：主程序明确主动停用时 bootstrap 可恢复 agent 观测，但主程序仍保持停用；信任配置不是解除停用意图。
- 删除 agent 服务配置，验证固定契约重建。
- 破坏 agent 文件，验证本地 agent 备份恢复。已实现的只是“文件缺失”：exe 存在但哈希不符时 bootstrap 记 `agent integrity unavailable; no start attempted`，既不启动也不替换，只能人工删掉坏文件后由下一轮走缺失恢复。仅 MSI、同 build、门禁打开时缺失恢复可用。
- 让 agent 检查循环停滞，验证核心进度超时和有界重启。尚未实现：主循环是单线程 `recv_timeout(30s)` 循环，没有进度戳或看门狗，SCM 与 bootstrap 都只看到 `Running` 而不动作。该用例按现码预期 `FAIL`，无现成注入 fixture。
- helper 在每个阶段退出，验证事务接续。
- 禁用 Task Scheduler，验证 degraded 和 SCM fallback，不宣称双入口仍正常。SCM fallback 可验收；degraded 上报未实现，`rescue.components.recovery_entry` 恒为 `not_assessed`，后台无法区分“双入口正常”与“只剩 SCM”。
- 执行彻底卸载并重启，确认所有入口遵循卸载标记，不复活 RustDesk。

### 9.3 网络和服务端

使用本地 mock 或隔离网络故障，不依赖线上服务制造失败：

- API 200、401、403、409、429、500、503。
- API DNS 失败、TLS 失败、证书不匹配、超时和连接重置。
- 下载 404、403、部分响应、错误大小、错误 SHA-256、错误签名。
- API 正常但 broker 不可用，验证短轮询。
- API 整体不可用，验证本地自治和 outbox。
- 服务端返回旧 policy revision、重复 command、未知 command version。
- 时钟偏差、nonce 重放、旧公钥、新公钥和设备身份 mismatch。

### 9.4 测试签名和网络隔离

Defender 安装选择新增测试规范见 [Windows Defender 安装说明](windows-defender-install.zh.md)。当前用户禁止本开发机真实 Defender 测试：只执行 mock，不能 Get/Add/Set/Remove 真实偏好、启用防护或改变防篡改；真实写入/UAC 项原定在独立 VM 执行，VM 已停用；用户 2026-10-23 授权改在 win10-gs 开启 Defender 后实机执行：执行前导出偏好与策略，测完按记录恢复，不改变防篡改；Defender 无法开启时如实记 BLOCKED。Windows 11 项因无测试机记 BLOCKED。

阶段实现和已运行证据以 [实施进度](agent-recovery-implementation-status.zh.md) 为准，不能将方案用例当作已实现驱动。`scripts/test-agent-foundation.py` 是现有可执行底座入口；Defender选择mock用 `tests/test_defender_installer.py`，无真实Defender/注册表IO。完整包和平台门禁仍需另行执行。

单元/集成使用测试 key 和本地 fixture；生产更新公钥保持不变。实际包恢复 E2E 使用正常签名发布的候选包，由已验证摘要的隔离缓存/服务供应。若使用测试编译器注入可信 key/endpoints，须明确区分测试构建，最终生产参数包另做 smoke，不能把它假装正式包。

禁止改 hosts 将固定生产域名映射不可信 HTTP，再关闭 TLS 验证求通过。断网按 API、下载、ID 服务分别隔离，不切整个管理网；网络注入必须可自动复原且有宿主控制台。

### 9.5 长期稳定性、容量与竞态

- 每种 edition/包类型至少 24 小时健康/周期检查，另做 48 小时小规模试点。确认无误恢复、日志有上限、CPU/内存/线程/句柄/任务不持续增长。
- 源码 fixture 可以加速退避边界，但真实包至少覆盖一个完整正常周期、失败预算和重启后预算持久化，不能全部把 1 小时改为 1 秒验收。
- 四种安装入口与 auto recovery/远程命令/双 helper 并发：记录事务号，证明最多一个变更；等待者不抢占活跃安装。
- outbox 达到条数/字节上限、磁盘写拒绝、日志卷满、下载中断、备份不完整、资源被文件锁占用。断言可靠终态和唯一救援资产未删。
- 1,000 台等效 HTTP 签名负载、broker 重连峰值、SQLite busy/WAL checkpoint、任务批量下发；压测只打本地 test endpoint，不打线上 API。
- 记录 P50/P95/P99、请求/错误数、设备数、失败原因、硬件和软件版本。延迟不足 20 次样本只报告实测范围，不宣称有统计意义的 P95。

## 10. 提交前、编译前和发布前全量门禁

先执行源码/静态/集成测试，再构建候选包，再以该实际包做 E2E，最后审查才能提交为完成。真实包 E2E 不可能发生在任何编译之前；“编译前测试”指发布构建前已有低层红绿证据，构建候选包不代表发布授权。

同一两仓库源码摘要、依赖/工具配置、构建参数及产物 SHA-256 的通过证据可在提交/发布检查复用，不机械重复无变化长测；任一改变使受影响证据失效。不能用改动前的旧包 E2E 验收新包。

### 10.1 客户端 Rust 最低检查集

在 `D:\github\rustdesk` 执行：

```powershell
cargo fmt --all -- --check
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

要求：

- 三条命令都实际执行；退出码为 0。
- `cargo test` 不能是零测试或错误过滤器造成的空通过。
- 记录编译 profile、target、测试数量和失败数量。
- 若有 Rust 代码变更，必须包含对应 Rust 测试；删除测试必须给出替代覆盖说明。
- Windows 平台代码还必须执行对应目标的实际构建或交叉编译。

### 10.2 Flutter 最低检查集

在 `D:\github\rustdesk\flutter` 执行：

```powershell
flutter analyze
flutter test
```

涉及桌面导航、窗口、服务桥接、管理页面或状态展示时，还必须：

- 增加并执行对应 `integration_test` 或等价真实桌面驱动测试。
- 在 Windows、Linux、macOS 至少一个真实桌面目标运行。
- 覆盖加载、空状态、网络错误、权限错误、状态刷新、确认框、键盘焦点和 390px 页面布局（如果是 Web/管理页面）。
- 不用 mock widget 的通过结果替代真实桌面桥接。

### 10.3 后台最低检查集

在 `D:\github\rustdesk-api` 执行适用命令：

```powershell
php -l sqlite/index.php
php -l sqlite/lib.php
python -m compileall tests tools
python tests/integration_test.py -v
python tests/upgrade_script_test.py -v
python tests/provisioner_test.py -v
python tests/offline_bundle_test.py -v
python tests/installer_test.py -v
python tests/docker_entrypoint_test.py -v
```

另在 stream-broker 工作目录执行 `go test -count=1 ./...`，POSIX/Shell suite 在 WSL/Linux 执行。再执行 SQLite HTTP/SQL、Playwright、待测本地容器和本轮专用 Compose 检查，记录：

- 实际执行的测试总数。
- SQLite fixture 路径和是否为 disposable 副本。
- API、数据库、页面三者结果是否一致。
- Playwright 桌面、390px 移动、失败、CSRF、权限、刷新和持久化结果。
- 容器内部端口 80、配置管理路径、公开 fallback 路由和 volume 重启持久化。
- 不运行 MySQL 扩展门禁，因为该后台当前明确冻结 MySQL 行为。

### 10.4 候选包构建与实际包检查

至少生成并检查：

- Windows standard EXE。
- Windows standard MSI。
- Windows SOS EXE、SOS MSI（本功能四包都须生成）。
- 涉及共享更新协议/打包时，覆盖 Linux AppImage manifest 选择并在 Linux 构建运行；纯 Windows 改动不为满足清单擅改 Linux 打包。

涉及 Windows agent 时必须额外确认：

- agent 二进制确实进入 EXE/MSI 包。
- 主程序和 agent 使用同一 source commit/build identity。
- agent 文件版本资源、build_seq、edition 和 package kind 正确。
- MSI WiX 组件、服务控制、MajorUpgrade 和永久组件规则符合测试预期。
- EXE 目标包包含 ProgramData agent 替换所需的 helper/bootstrap 资源。
- manifest 的 target key、size、SHA-256、签名和下载路径与实际包一致。
- 产物不是 debug、错误 edition/架构包。Windows 无商业 Authenticode 证书不自动判失败，但 manifest 签名必须有效；Android debug/unsigned 永不发布，按固定长期证书合同验收。

以 `.github/workflows/flutter-build.yml` 为 canonical 构建来源，记录版本、build_seq、edition、arch、features、SDK/vcpkg 和源码摘要。agent 最小编译入口是：

```powershell
cargo build --locked --bin rustdesk-management-agent --release
```

这只构建 agent。主程序采用 workflow 的 `build.py --portable --flutter --skip-portable-pack --hwcodec` 及真实矩阵参数；EXE 由 `libs/portable/generate.py` 打包完整 dist；MSI 由 `res/msi/preprocess.py --arp --edition`、NuGet restore、x64 MSBuild 构建。执行前读取当前 workflow 完整命令和元数据准备，不省略目标参数。

MSI preprocess 会改写模板，须在临时构建副本执行或精确恢复本轮生成内容，不丢弃用户改动。四包使用独立产物目录，每包记录 hash/size/来源/参数与 manifest 验证。

### 10.5 提交前总门禁顺序

推荐严格按以下顺序执行：

1. 读取最终 diff，确认只修改必要文件，没有混入无关清理。
2. 运行 Rust fmt、test、clippy。
3. 运行 Flutter analyze、test 和必要桌面集成测试。
4. 运行后台 PHP/Python/Go 静态检查和 SQLite 集成测试。
5. 运行后台 Playwright E2E 和容器门禁。
6. 构建实际 Windows EXE/MSI 及适用 Linux 包。
7. 在 win10-gs 执行四种全新安装路径。
8. 在 win10-gs 执行四种旧版本升级路径。
9. 执行 R0–R6、agent 自身和事务中断故障注入。
10. 重新读取后台 SQLite，核对 API、页面、agent state、事件和 outbox。
11. 执行最终代码审查、`git diff --check` 和测试证据清点。
12. 清理本轮临时资源，确认没有测试进程、容器、计划任务、临时服务和测试数据库残留，win10-gs 已恢复到测试前状态。

### 10.6 提交前阻断条件

出现以下任一情况，禁止提交为完成、禁止发布或扩大灰度：

- 旧版本主程序升级了，但 ProgramData agent 没有升级或实际运行版本未知。
- 主程序服务停止后只依赖后台命令才能救活，没有本地自治路径。
- fallback 只写了代码路径，没有在 win10-gs 真机走通。
- API 不可用时 agent 因等待上报而停止本地恢复。
- agent、helper、bootstrap 之间可并发执行两个安装事务。
- ID、UUID、公钥、配置或原版本丢失且没有明确失败结果。
- 新 agent 不启动、能力上报不完整、登记失败或后台仍显示旧 agent 正常。
- 任一 required gate 为 BLOCKED 但没有批准的替代证据。
- 测试命令匹配零测试、测试被跳过、失败被允许、只检查 HTTP 200。
- 没有清理当前测试创建的临时资源，或测试日志不足以复现失败。

每条原生命令单独保存退出码；PowerShell 后续 `Pop-Location`、日志输出或清理成功不代表前面的 cargo/go/python 成功。使用 `try/finally` 复原环境时，原始失败必须保留；长进程定期输出状态但不能因超时直接记回滚成功。

源码门禁先列出适用平台/feature 组合，保留 AGENTS 要求的全量命令。若 `--all-features` 或 Windows 运行环境无法构建某平台原生依赖，记录精确 target、feature、依赖错误，在适用构建机补齐；不关闭 feature、修改既有功能或缩小测试以伪造通过。

## 11. 测试记录模板

每个测试阶段复制以下模板，保存为 `test-evidence/<date>/<case-id>.md`。测试证据目录不提交密钥、密码、设备私钥或生产数据。

````markdown
# Test case: CASE-ID

- Status: PASS / FAIL / BLOCKED / NOT_APPLICABLE
- Path: FRESH_INSTALL / LEGACY_UPGRADE / RECOVERY_FAILURE_INJECTION / PRE_COMMIT_FULL_GATE
- Package: STD-EXE / STD-MSI / SOS-EXE / SOS-MSI
- Client commit:
- API commit:
- Build number / build_seq:
- Windows version / architecture:
- win10-gs backup tag:
- Started at:
- Finished at:

## Preconditions

-

## Commands and actions

```text

```

## Expected result

-

## Actual result

-

## Evidence

- Service query:
- Process/path/version:
- SHA-256:
- agent state:
- API response:
- SQLite query:
- Screenshot/log/event path:
- Fallback level:
- Transaction/migration ID:

## Failure preservation

- Old client preserved:
- Old agent preserved:
- Identity preserved:
- Configuration preserved:
- Recovery entry preserved:

## Cleanup

- Removed:
- Intentionally retained:
- Reason:
````

## 12. OpenCode 执行协议

新机器上的 OpenCode 执行此手册时必须：

1. 先检查两个仓库路径、当前 commit、工作区状态和可用工具。
2. 读取两仓库 `AGENTS.md` 并遵守主分支、发布、测试和清理约束。
3. 创建本轮唯一测试根目录，例如 `test-evidence/<timestamp>-agent-recovery/`。
4. 在执行破坏性 Windows 测试前，在 win10-gs 上做完整备份（逐文件哈希、属性、ACL、服务定义）并空跑一次恢复，记录备份标签。
5. 一次只运行一个改变服务/安装状态的测试；并行只允许用于互不影响的静态检查或只读报告。
6. 每完成一层测试立即写证据和状态，不把全部步骤做完后凭记忆补写。
7. 任一失败先停止扩展测试，保留现场，判断是否需要修复后重新备份。
8. L8 汇总 L0–L7、候选构建和最终审查，按第 10.5 节顺序推进，不形成“先 E2E 后允许构建”的循环依赖。
9. L8 通过后仍需执行最终 diff、资源清理和测试证据审查。
10. 未经明确请求不 commit、tag、push、部署或改生产。

### 12.1 提交的可执行测试资产

实施交付必须包含受版本控制的能力—测试目录、非生产固定 fixtures、实际 Windows 桌面/服务驱动、隔离 API/Compose 准备入口、依赖锁定、断点 resume 支持和机器可读 results。本文当前是规范，不能声称这些驱动已经存在。

单包测试数据记录路径/摘要，runner 参数明确 `fresh/upgrade`、edition、kind、OS、旧包/新包、API test URL、snapshot 和 evidence root。preflight/list 为只读；执行变更前验证指定机器是本轮目标。无自动化的 UAC/画面交互可形成有证据的人工测试，不包装为自动化通过。

结果文件至少保存 case_id、path、OS、package、capability、状态、退出码、实际测试数量、源码与包摘要、错误、证据路径和清理结果。OpenCode 恢复会话读取结果索引和 resume，再采样实际系统；不凭上下文摘要推断前序测试通过。

### 12.2 最终报告

最终 `summary.md` 分别列：源码门禁、四包/两路径/两 OS 矩阵、主程序能力、agent 能力、故障注入、兼容性、长测、缺口和临时资源。给出 PASS/FAIL/BLOCKED/NOT_APPLICABLE 计数，以及“候选构建通过”“实际包验收通过”“异地完整保护可发布”三种不同结论。

缺少源码的纯安装机可以复用同摘要构建机证据，需注明来源和未在本机执行的门禁；不能要求新机器装全链工具，也不能声称本机运行了并未运行的检查。任何高风险 required case 失败/阻塞都不能标完整保护。

## 13. 最终发布判定

Management Agent 只有在以下条件全部满足时，才能标记为“异地可用”：

- 四种全新安装包通过。
- 四种实际旧版本升级通过，并由新 agent 实际上报全部适用能力。
- 主程序下线可由本地 agent 自动逐级救活。
- agent 自身有 SCM 和 bootstrap fallback。
- 至少一个离线恢复路径和一个在线恢复路径真实走通。
- API 整体不可用时，本地恢复不停止；恢复后 outbox 可补报。
- 更新、重装、回滚、断电和 helper 中断都有事务证据。
- ID、UUID、公钥、配置和旧版本保留规则通过。
- 后台状态不混淆主程序在线、agent 在线、远控注册和 API 可达。
- Rust、Flutter、后台、SQLite、Playwright、实际包、win10-gs 真机和提交前全量门禁全部通过。
- 没有未解释的 FAIL，也没有把未执行项伪装成 PASS。

本手册不代表测试已经完成。它定义的是实现完成后必须执行的测试流程和发布门禁。
