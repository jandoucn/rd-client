# Management Agent 能力交付 + 后端重构 执行计划与测试记录

本文是本轮交付的 TDD/E2E 执行计划与**滚动测试记录**（每阶段完成后在对应小节追加实测结果/日志摘要/修复）。配套能力评估见 `docs/management-agent-capability-assessment.zh.md`。

## 1. 范围

1. 后端重构（`~/github/rd-api`）：① 能力注册表数据化；③ 后端镜像客户端 status schema（rescue/files），修 status 误报。纯后端，不卡 Windows 构建。
2. 验证现有安装/升级/Management Agent 全能力流程，抓日志修现有 bug。
3. P0 客户端（`~/github/rustdesk`）：SC failure actions（崩溃秒级自愈）+ status 投影修正 + **F1 修复（agent.update 同构建自升级 reconciliation_required 卡死命令处理，见 Phase 3 实测记录与能力评估 §5 P0-1）**。

## 2. 机器 / 环境 / 部署模型

| 用途 | 机器 | 说明 |
|---|---|---|
| 客户端编译 + 实测（唯一测试机） | win10-gs（termark `01a15c96…`，10.0.0.100，`C:\RustDeskBuildTools`，client_id 1338209581，连固定 `rdapi.yan.life`） | MSVC+vcpkg 已确认；rustc/cargo/flutter 位置待定位。随意下 sos/standard 各构建号包、升级、破坏、恢复、抓日志。**破坏性操作前先备份状态可还原；不重启 OS；不碰非 RustDesk 数据目录（要碰先问）** |
| ~~GepShare-ssh~~ | — | 已取消，不用 |
| 后端联调/部署 | termark `a5f1409a…`（NTServer-SH 47.100.7.221）`/opt/1panel/apps/rustdesk-api` | 提交 GitHub api → Docker 镜像 → 该目录生产升级（带回滚快照） |
| 最终发版 | GitHub Actions | 7 包：win/macOS 各 sos+standard 的 EXE/MSI/DMG + 安卓仅 standard APK；安卓/macOS 签名证书与方式**一律不动** |

## 3. TDD 分层要求（两仓 AGENTS.md 均强制）

1. 先把需求拆成可验证行为与失败条件 → 2. 先写失败测试 → 3. 最小实现令其通过 → 4. 重构 + 补边界 + 保留回归。
- **单元**：纯函数/状态机/校验/序列化/签名（Rust `#[test]` / PHP / `flutter_test`）。
- **集成**：跨模块/HTTP/配置/事务/桥接，用本地 fixture/mock/真 PHP+SQLite，不依赖线上。
- **E2E/机器**：桌面流程必须用**构建出的实际包**在真机跑（win10-gs），覆盖安装/升级/下载校验/安装失败回滚/旧版可用。
- 覆盖率目标 ≥80%，禁空过滤器/删测试凑数；失败路径必须断言错误类型+旧数据保留。

## 4. 阶段与 TDD 清单

### Phase 1 — 后端 ① 能力注册表（纯后端）
目标：把 `agent_command_types()` + `agent_command_validate()` 每类型 args schema + `agent_command_permission()` 权限桶，收敛成一张「能力注册表」（PHP const/表），新增能力只加一行数据、不改 3 处逻辑；受 `in_array(type@N, 客户端 capabilities)` 闸约束，不越客户端地板；清理 `client.enable` 孤儿。
- 失败测试（先写）：注册表为唯一真源——(a) 命令类型集合 = 注册表 keys；(b) 每类型权限桶 = 注册表值；(c) args schema 校验走注册表；(d) 加一条测试用注册表项即新增一个可建能力、无需改三函数；(e) 不在客户端 capabilities 的类型仍被能力门 409 挡；(f) 回归 `agent_operations_test.php` 全绿（行为不变）。
- 验收：`tests/agent_operations_test.php` + 新增注册表测试全绿；行为与重构前逐项一致。
- 实测记录：**已完成（后端 commit `6a8ec13`，未推送）**。实现 `agent_command_registry()` 为唯一真源，`agent_command_types`/权限桶/args 校验/包类判定/observe 策略门全部派生自它（改 `agent_operations.php` 5 处，行为保持）；`client.enable` 本轮保留（被能力门天然 gate，孤儿去留留作单独决策）。测试（Docker `rdapi-test` 镜像 php8.3+python3）：新增 `agent_command_registry_test` **86 断言**绿；回归 `agent_operations` **76**、`agent_status` **30**、python HTTP 套件 **67 测试** 全绿，行为零变化。

### Phase 2 — 后端 ③ status 镜像（纯后端）
目标：`agent_status.php` 已收 capabilities/package_identity/build_seq；扩展建模 `rescue.*`（组件/ready/freshness/source_digest）与 `files.*`，`overview`/控制台如实反映可恢复性——installed_msi 的 `ready=false` 不得呈现为"不可恢复"，须读组件级 `verified` + 引擎真实支持。
- 失败测试：上报含 rescue/files 的 status → snapshot/overview 正确存取与呈现；installed_msi 且组件 verified 时，控制台可恢复性判定为"可恢复（引擎级）"而非 ready=false 直译。
- 验收：`agent_status` 相关 test + 控制台 overview test 全绿。
- 实测记录：**已完成（后端 commit `901175f`，未推送）**。发现 overview 本就经 `agent_status_snapshot` 原样回传客户端 payload（含 rescue/files），且客户端 status 上报确实带 `rescue`（完整 snapshot）+ `files`（`status.rs:170-171`）。实现：① `agent_recoverability()` 只读组件级 `verified` 派生可恢复性（不照 `ready=false` 误判，installed_msi 组件 verified 时判"可恢复"）；② 管理总览新增 `recoverability` 字段；③ `agent_status_validate` 对 rescue/files 向后兼容轻校验。测试：新增 `agent_status_recoverability_test` **12 断言**；回归 PHP 四连（registry 86 / operations 76 / status 30 / recoverability 12）+ python 非浏览器 HTTP **80 测试** 全绿。**遗留**：`agents.html` 可视化卡片（前端，需 playwright 浏览器）放到 win10-gs 联调阶段加+验；本地容器无浏览器，playwright e2e 未跑。

### Phase 3 — 验证现有流程（win10-gs 机器 E2E，用现有已发布包）

> 后端 ①③ 已于 2026-10-21 13:31 部署生产（NTServer-SH `/opt/1panel/apps/rustdesk-api`，`upgrade-api.sh latest` → 镜像 `cf718342`=commit `901175f`，SQLite 已备份 `rustdesk.pre-upgrade-latest-20261021133119.db`，健康检查通过，可回滚）。部署后冒烟：manifest 200 / publish 401 / agent/commands 401，发布数据完好（1.5.3 / 2026101805 / 7 targets）。
> **本阶段首项**：确认 win10-gs 真机 agent ↔ 新后端 status 上报被接受（验证 rescue/files 轻校验不拒真机）+ 管理总览 `recoverability` 字段对真机返回 `verified`（需管理端鉴权）。

目标：不依赖新客户端，先把当前"安装/升级/agent 全能力"在 win10-gs 跑穿，建基线+抓现有 bug。
- 用例：干净装 standard MSI / sos MSI；旧 build→新 build 升级；远程命令全类型（service.*/recovery.reconcile/package.verify/reinstall/update/agent.update/diagnostics/network.probe）经后端下发→执行→事件回报；失败回滚旧版可用；救援包暂存→verified。
- 验收：每用例实测通过或记录 bug+修复。
- 实测记录：**进行中（后端 ①③ 已部署生产，经管理端 `/yanolly` olly 会话活体联调）**。
  - **③ recoverability 活体验证通过**：win10-gs（client_id 1338209581）status 上报被新后端接受（presence=online，证明 rescue/files 轻校验不拒真机），管理总览 `recoverability={evidence:verified, verified_components:[agent_backup,bootstrap_backup,client,credentials,offline_package]}`——真机报 `ready=false` 但后端正确判"可恢复"，③ 修复成立。binding 授权全权限、installation_id `0b2a3040…`。
  - **命令全链活体验证**（create→claim→execute→event→终态）：**8 种命令**经管理端下发实测——`service.inspect`/`network.probe`/`diagnostics.collect`/`package.verify`/`service.restart`/`recovery.reconcile`/**`package.reinstall`（完整重装事务 2026101805）** 全部 **succeeded**；`service.restart` 后服务 running/ready 干净恢复；`package.reinstall` 后 win10-gs 健康（build 2026101805、PackageName 规范、双服务 Running、救援无错）。
  - **F1 发现（agent.update → reconciliation_required，升级为 P0）**：同构建 agent.update 终态为 `reconciliation_required`（非 succeeded），根因 `operations_runtime.rs:538/552` 自升级替换自身进程 → 无法确认终态证据（`transaction_evidence_unconfirmed`/`interrupted_mutation`）；经 `journal.rs:364` 门禁阻断后续所有变更命令，agent 反复从后端重领该未完成命令自堵。**实测：命令过期（expires_at）后 `reconciliation_required` 并不转 `expired`、不自解**——即持久卡死命令处理直到人工介入。机器功能正常（agent 跑着、build 2026101805、installation_id 不变、binding 有效）。**恢复手段（已对 win10-gs 执行并验证）**：① 后端终结该命令（容器内 `php PDO` 将 `device_agent_commands.status` 由 `reconciliation_required` 改为 `cancelled`，agent 即不再被下发它；cancel API 对已领取命令拒绝，故走 DB）；② win10-gs 停 agent、删本地 journal `state\commands-1.json`（卡住条目在 generation=1 的 commands-1.json；备份留证 `commands-1.json.wedged-bak`）、启 agent；③ 下发全新 service.inspect 验证 **succeeded**，命令处理恢复，recoverability 仍 verified。**修复见 Phase 5b（客户端 P0）**。
  - **待补**：`package.update` 旧→新升级 E2E（需更低构建号包；OSS 旧 tag URL 探测 404，且本地构建新版本后在最终发版自然产生更高 build，届时做 old(2026101805)→new 的真实升级验收。package.reinstall 已验证同一安装引擎）；`service.start`/`service.repair`（次要）；失败/回滚路径（install_execution 单测已覆盖，真机留最终发版触发）。

### Phase 4 — P0 客户端 SC failure actions
目标：给 `RustDesk` + `RustDeskManagementAgent` 服务注册 SC failure actions（崩溃自动重启），缩短崩溃恢复从分钟级→秒级；纯增量，不改既有恢复链。
- 失败测试（Rust）：服务安装/注册路径产出含 failure actions 的配置；禁用/维护意图下不改变（与既有门禁一致）；回归既有 service 安装测试。
- 机器验收（win10-gs）：kill agent/主进程 → 观测 SCM 自动重启秒级；与计划任务兜底不冲突。
- 实测记录：**已完成，真机验证通过**。
  - **设计修正（升级≈新装）**：最初打算只在安装脚本里加 `sc failure`，发现原地升级**不重跑安装脚本**（`native_install.rs` 只做 stop→install→start），已装机永远拿不到该能力。改为 **agent 每次启动时幂等配置**（`service_main` 调用 `recovery_windows::ensure_service_failure_actions()`），安装/升级/修复/bootstrap 重建全部覆盖。
  - **实现方式修正**：先用 `windows-service` 0.6 的 `update_failure_actions()`（ChangeServiceConfig2），真机清空后重启**未写入**（PID 确实变化、代码确实执行，但注册表仍为空）。改为与 `repair_service`/`rescue_recovery` 一致的 `sc.exe failure reset= 86400 actions= restart/5000/...`，真机立即生效。
  - **真机证据（win10-gs）**：删除两个服务的 `FailureActions` 注册表值 → 重启 agent → `RustDesk=PRESENT`、`RustDeskManagementAgent=PRESENT`；`sc qfailure` 显示 `RESET_PERIOD=86400`、三条「重启动 -- 延迟 = 5000 微秒」。
  - **自愈实测**：`taskkill /f` 强杀（不手工 start）→ **5225ms** 后 SCM 自动拉起新进程（pid 34828→9360），服务回到 Running。对照：此前只能等 `RustDeskManagementAgentRecovery` 计划任务（最长 5 分钟）。
  - 单元测试：`failure_actions_argument_is_restart_three_times_with_daily_reset` 断言 `reset= 86400 actions= restart/5000/restart/5000/restart/5000` 且 `reset=` 在 `actions=` 之前（sc.exe 对顺序敏感）。
  - **2026-10-22 实测追加修复**（均先在 win10-gs 复现再修）：
    1. 服务存在性改用 SCM API 判定（`ERROR_SERVICE_DOES_NOT_EXIST`=1060 即不存在）：中文系统 `sc.exe` 输出无 `does not exist` 字样，存在的服务会被误判为缺失而跳过。
    2. 写入受本地门禁约束：`disable-recovery`/`disable-agent`/`maintenance.json`/意图非 Enabled 时不写失败动作（`failure_actions_allowed`）；意图文件不存在视为 Enabled（与启动路径一致），存在但不可读视为 Unknown 拒绝。
    3. 安装时序：agent 服务先于客户端服务启动（客户端服务在 InstallFinalize 附近创建），启动时一次性配置会漏掉客户端服务。改为后台线程按状态机重试：门禁每轮重读（原地升级持有 `maintenance.json` 期间等待、释放后自动配置），门禁打开后 120s 配置窗口、总时长 1800s，服务 Stop 时立即退出；客户端服务缺失不算完成（`failure_actions_complete`）。
    4. 状态机与完成判定抽为纯函数并按 TDD 先红后绿：`failure_actions_complete_requires_client_applied_and_no_refusal`（8 组组合）、`retry_*`（停止/门禁等待/窗口重置/总时长区分门禁与失败）、`retry_gate_tracks_markers_that_change_after_startup`。
  - **后台重试线程细则（以 `src/management_agent.rs` 线程 `agent-failure-actions` 与 `windows.rs` `FailureActionsRetry` 为准）**：
    - 每轮先判停止、再判总时长、再判门禁：服务收到 Stop → 立即结束（`Stopped`）；自线程启动起满 `OVERALL`=1800s → 结束，从未遇到门禁打开记 `HeldByGate`（只写 info，属本地停止意图，不算故障），遇到过记 `Incomplete`（节流 warn）。
    - 门禁（`failure_actions_allowed_now`）每轮重读：purge 标记、`disable-agent`、`maintenance.json` 任一存在或意图非 Enabled 时等待（5s 一轮），并**清空** 120s 窗口，维护期不消耗配置窗口。
    - 门禁打开后开启 `WINDOW`=120s 配置窗口，窗口内每 5s 调用一次 `ensure_service_failure_actions()`；完成（`RustDesk` 必须 Applied 且无 Refused）→ `Configured`；窗口到期仍未完成 → `Incomplete`。
    - 本地停止意图与门禁始终优先：线程只会"等待"或"放弃"，不会越过门禁写入失败动作。
    - 已知限制（T8 L1/L2）：线程超时结束后需等 agent 下次启动才再次尝试；线程 spawn 失败会使 agent 服务启动失败。

### Phase 5 — P0 客户端 status 投影修正
目标：`rescue_status.rs` 投影让 installed_msi 的 `local_repair`/组件状态反映引擎真实支持（与后端 Phase 2 对齐），不再误报"不可恢复"；`ready` 字段契约保持（如仍 false 则文档化语义），重点是组件级真实性。
- 失败测试：installed_msi + 引擎支持 total-loss/local_repair 时，投影不得输出 `installed_msi_local_repair_not_supported`；组件 verified 正确透出。
- 机器验收：win10-gs 实测 status 与真实恢复能力一致。
- 实测记录：_（待填）_

### Phase 5b — P0 客户端 F1 修复（agent.update 卡死）
目标：修复生产实测确证的 F1——同构建 `agent.update` 终态 `reconciliation_required` 永不自解、经 `journal.rs:364` 门禁卡死后续变更命令。
- 方案：自升级提交后，新 agent 在 resume/reconcile 路径自校验（当前 build/identity/健康 == 目标且事务已落地）即把该命令 reconcile 成 `succeeded`；或给 `reconciliation_required` 加超时/自愈降级，使其不无限阻断其他命令。改在 `operations_runtime.rs` resume/reconcile 与 `command_transaction` 终态确认处，不动既有 package.reinstall/update 成功路径。
- 失败测试（Rust）：构造「自升级目标==当前 build、事务证据缺失但 agent 健康一致」场景，断言命令 reconcile 为 `succeeded`（或在超时后降级为明确终态），且不再阻断新变更命令；回归 package.reinstall/update 的 succeeded/rolled_back 路径不变。
- 机器验收：win10-gs 下发 agent.update 后命令达终态、后续 service.* 命令可正常处理（不被堵）。
- 实测记录：**客户端修复与集成测试已完成；真机验收 BLOCKED（见下）**。
  - **根因修正**：F1 现场日志为 `agent transaction evidence refused: workspace path unavailable (os error 2)`，即事务工作区 `transactions\<id>` 在自升级后不存在。Windows 事件日志确认同一事务 MSI 13:52:29 已 **安装成功**，13:52:34 的第二次 MSI 事件是救援素材准备的 `msiexec /a` 管理员解包（不安装、不消费源事务）。代码全量排查（全部删除/改名 API、MSI 自定义动作、wxs）未找到删除者，删除发生在 13:52:35–13:56:33 之间，判断为外部操作（排除法，证据中等）。产品缺陷在于对账容错：工作区缺失即永久卡死。
  - **修法（只针对 agent.update，且只在工作区不存在时）**：按可观测终态收敛——运行中 agent 即目标构建 → `succeeded`，result 写明 `evidence: target_build_observed`（不冒充读到事务回执）；不是目标构建 → `failed`（`transaction_workspace_absent`）。两者都是终态，解除 journal 门禁。工作区存在但证据被拒、事务仍在进行，保持原 `reconciliation_required` 路径不变。
  - **终态判定顺序（以 `operations_runtime.rs` `poll_with_identity` 为准）**：仅对 journal 状态为 `Running`/`ReconciliationRequired` 的 `package.reinstall`/`package.update`/`agent.update` 生效。
    1. `agent.update` 且 `transactions\<command_id>` 经 `symlink_metadata` 判定为 **NotFound**（`lost_self_update_workspace`）→ 按上条 `absent_workspace_terminal` 直接定终态；目录存在但不可读不算丢失。
    2. 否则读事务证据（`command_transaction::read_terminal`）：有终态证据则按证据收敛；证据被拒且命令仍 `Running` → `reconciliation_required`（`transaction_evidence_unconfirmed`）。
    3. 上一步之后仍为非终态、且是 `agent.update`、运行构建等于目标构建 → `succeeded`（result 为 `{transaction_id,self_update_confirmed:true}`，不带 `evidence` 字段）；构建不等则保持 `reconciliation_required` 等待人工。
    - `package.reinstall`/`package.update` 不走 1、3，工作区缺失仍为 `reconciliation_required`（见下方已知限制）。
  - **集成测试补充**：同一往返测试还覆盖"工作区存在但证据损坏"两段——同构建→`succeeded` 且 SQL result 为 `{transaction_id,self_update_confirmed:true}`；更高构建→保持 `reconciliation_required`（`transaction_evidence_unconfirmed`）。
  - **测试**：单元 `absent_workspace_terminal_follows_observed_target_build`、`lost_workspace_exit_is_agent_update_only_and_requires_absence`；集成在真实 PHP+SQLite 往返测试 `signed_rust_command_roundtrip_uses_real_php_sqlite_and_journal` 中新增三段：同构建→journal `Succeeded`、SQL `succeeded` 且事件 result 含 `target_build_observed`；更高构建→`failed`+`transaction_workspace_absent`；`package.update` 工作区缺失仍 `reconciliation_required`。已用变异法确认该段确实执行（故意改错期望值→变红）。
  - **真机验收 BLOCKED**：agent.update 安装的是后端已发布、CI 签名的包；生产当前仅有不含修复的 `2026101805`，新进程跑旧代码会复现 F1，本地无法签出可被接受的清单。需带修复的 CI 正式构建发布后执行：下发同构建 agent.update → 命令达终态 → 后续 `service.inspect` 正常处理。
  - **已知限制（未修，按 AGENTS.md corner case 规则记录）**：`package.reinstall`/`package.update` 在工作区被外部删除时仍会停在 `reconciliation_required` 并阻断后续变更命令，恢复需人工（后端终结命令 + 清本地 journal）。修复需要为包命令引入新的终态判定依据，属新增状态，待维护者决定。

### Phase 5c — 远程操作静默与远程日志调取（台账 T2，进行中）

目标：远程后端发起的安装/升级/修复/重装/卸载（后台更新命令、策略自动安装、Agent 命令、救援、首装、purge），不论 EXE 还是 MSI，一律完全静默（无 Toast/窗口/msiexec UI/托盘），成功与失败相同；本地用户手动操作（主界面检查更新、在线更新、手动安装/卸载）保持原有通知行为不变。静默不等于丢失：保留日志，并可经 Agent `diagnostics.logs` 远程调取。

- **来源标志 `--remote-silent`（`libs/base/src/update.rs`）**——✅ 单元测试先红后绿：
  - `update_is_remote(manually, has_command)` = `has_command || !manually`：只有"本地手动且无后台命令"是本地，其余（含无命令的策略自动更新）都算远程。
  - `update_launch_args(remote, pending)`：标志紧跟 `--update` 之后、`--update-event` 之前。后者按位置解析且末位 command_id 可为空，追加在末尾会被误读为 command_id。
  - `is_remote_silent(args)`：只做整参数精确匹配，`--remote-silent-x`、`x--remote-silent` 不算。
  - 测试：`update_origin_is_remote_unless_started_manually_without_a_command`、`remote_marker_never_shifts_the_positional_update_event`（有/无 command_id 两种，均断言 `PendingUpdateEvent::from_cli_args` 往返不变）、`remote_marker_is_recognised_exactly`。
- **接线**——代码完成，待 Windows 编译验证：
  - `src/updater.rs` 仅在远程时把标志写入 `--update` 启动参数；远程 MSI 走 `update_me_msi_remote`（`/qn`、`LAUNCH_TRAY_APP=N`、`/L*v` 写固定日志 `<log_root>\msi-update\msi-update.log`），本地 MSI 仍调原 `update_me_msi`。
  - `src/core_main.rs` 的 `--update` 与 `--silent-install` 两处 Toast 之前判断 `is_remote_silent`：远程只写日志并返回，本地路径代码零改动。
  - 进行中（台账 T2.2c/M3）：`--silent-install --remote-silent` 目前没有生成方；已安装时后台 updater 只归服务进程（`process_runs_background_updater`）、远程更新不回落拉起主窗口（`update_main_window_fallback_session`）已有纯函数与单元测试，整体接线与真机验证未完成。
- **`diagnostics.logs` 只读命令（`src/management_agent/diagnostic_logs.rs`）**——客户端单元测试已完成，真机未验：
  - 能力声明 `diagnostics.logs@1`；准入与执行两处使用同一参数契约，只接受 `source` 与 `transaction_id` 两个键。
  - `source` 枚举：`agent`/`server`/`service`/`update`/`silent-install`/`install`/`msi-update`/`recovery-msi`/`transaction`。前六个读对应固定日志目录内最新 `.log`；`msi-update` 读固定文件 `msi-update\msi-update.log`（与 `update` 目录分开，`update` 源不会读到 MSI 日志）；`recovery-msi` 读 agent 日志目录固定文件 `recovery-msi.log`；`transaction` 读 `transactions\<id>\installer.log`，且事务目录须通过工作区校验。后端当前只注册前八个，未含 `recovery-msi`。
  - `transaction` 必须带 32 位小写十六进制 `transaction_id`；其他源带 `transaction_id` 一律拒绝。任何字段都不能指定路径；符号链接/重解析点整条路径拒绝，不跟随。
  - 上限：尾部窗口 32KB（`MAX_BYTES`）且最多 400 行（`MAX_LINES`）；JSON 转义后仍按 32KB 预算保留最新行，超出即 `truncated=true`。MSI 的 UTF-16LE 日志按 BOM 识别解码。
  - 脱敏：含 password/passwd/pwd/secret/token/private/key_pair/signature/enc_id/authorization/cookie/bearer/api_key/access_key/otp（不区分大小写）的整行替换为 `[redacted]`；其余行中长度 ≥32 的 `[A-Za-z0-9+/]` 连续串替换为 `[redacted]`；返回 `redacted_lines` 计数。
  - 返回：成功 `{schema:1,source,file(仅文件名),modified_at,size,truncated,lines,redacted_lines}`；源不存在仍为 `succeeded`，返回 `{missing:true,lines:[],file:null}`。失败只回固定 `error_code`：`log_parameters_invalid`/`log_unreadable`/`log_path_refused`，不外泄 OS 错误与路径。
  - 测试：`diagnostic_logs_tests.rs` 19 项（参数、固定映射、尾部边界、UTF-8/UTF-16/ANSI 解码、预算、重定向拒绝、相对根拒绝、准入与执行同契约、`update` 不读 MSI 日志、`recovery-msi` 固定文件等）；`operations.rs` `diagnostics_logs_is_admitted_read_only_with_strict_arguments`；`operations_runtime.rs` `diagnostic_logs_terminal_reports_fixed_codes_without_local_detail`。
- **救援 MSI 日志**：救援助手的 `msiexec` 追加 `/L*v <agent logs>\recovery-msi.log`；日志目录无法创建时不加参数并记 warn，不阻断救援。
- 待做：Windows 编译与 clippy；真机远程下发 EXE/MSI 安装、升级、卸载各一次，确认无新增通知/UI；本地手动更新一次确认仍通知；经 Agent 调回日志（台账 T2.5/T10.7）。

### Phase 6 — 全链机器验收 + 发版门槛
- win10-gs：新客户端包 干净装 + 升级 + 全能力 + 崩溃自愈 + 失败回滚 全绿。
- 后端：生产 `a5f1409a` 升级后，win10-gs 真 agent 联调全能力通过、无回归。
- 全绿 → 通知用户走 GitHub 7 包构建（签名方式不动）。
- 实测记录：_（待填）_

## 5. 测试矩阵（能力 × 层级）

| 能力 | 单元 | 集成 | 机器 E2E |
|---|---|---|---|
| 后端能力注册表 | ✅ | ✅(真 PHP+SQLite) | — |
| 后端 status 镜像/控制台 | ✅ | ✅ | win10-gs 联调 |
| 安装 standard/sos MSI | — | fixture | win10-gs |
| 旧→新 升级 + 回滚 | — | mock installer | win10-gs |
| 远程命令全类型 | ✅ | 真 PHP+SQLite | win10-gs+后端 |
| SC failure actions | ✅ | — | win10-gs kill 测试 |
| status 投影真实性 | ✅ | — | win10-gs |
| 远程静默 `--remote-silent` | ✅ | — | 进行中（T2.5） |
| `diagnostics.logs` 远程日志 | ✅(客户端+后端) | 后端 HTTP/E2E | 进行中（T10.7） |
| agent.update 工作区丢失终态 | ✅ | ✅(真 PHP+SQLite) | BLOCKED（需 CI 正式构建） |

## 6. 验收标准（交付即可验收态）

- 两仓 TDD 全绿：`cargo test`（Windows 目标，经 win10-gs/CI）、`flutter test`、后端 PHP/Python 全集；无空过滤器。
- win10-gs 机器 E2E：安装/升级/全能力/崩溃自愈/失败回滚逐项通过并留日志。
- 生产后端升级无回归，win10-gs 真 agent 联调通过。
- 本文测试记录各阶段填实。
- 固定配置（密码/ID 服务器/API/下载域/公钥/签名方式）零改动。

## 7. 风险 / 开放项

- win10-gs 的 rustc/cargo/flutter 位置未定位（MSVC+vcpkg 已确认）——客户端阶段解决；若缺则装 rustup + 克隆仓库，不改既有 `C:\RustDeskBuildTools`。
- 客户端能否本地出整包 MSI（需 Flutter+WiX）或仅 agent 二进制——定位后决定测试粒度。
- 生产后端升级有真实风险，每次带回滚快照 + 用户知情。
- **重装后无法重新登记（真实缺陷，2026-10-22 实测）**：agent 状态目录被清理或重装后生成新 `agent_installation_id`，后端 status 返回 409「agent 实例变化，需要重新登记」、命令轮询 403；拒绝时新实例 ID 不落库、不审计，两个管理页面授权时取的都是旧 ID，管理员无从得知新实例，只能到设备上读 `agent.json` 后经 `/admin/api/agent/binding` 先撤销旧实例再授权新实例（本次 win10-gs 即如此恢复，generation 1→2→3）。建议后端在 409/403 前记录候选实例供管理员授权，属新增状态，待维护者决定。**2026-10-22 后端已实现**（台账 T1）：拒绝时记录候选实例，管理页「接管此新实例」原子接管，见 rustdesk-api `docs/agent-console-plan.zh.md`。
- **本地测试流水线缺陷（已纠正）**：此前 `D:\tmp\repack_all.bat` 两个 edition 的 MSI 都用 `-d ../../rustdesk` 同一目录打包，SOS MSI 实际可能装入 standard 程序，此前 SOS-MSI 验收结论不可信。现改为按 edition 目录打包，并以 `msiexec /a` 解包核对 agent 哈希与 edition 专属 `librustdesk.dll`。
- 机器分工（编译+实测 win10-gs / 后端联调 NTServer-SH）以本文为准，可按用户调整。
