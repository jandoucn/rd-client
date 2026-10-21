# Management Agent 远程救援与保活能力评估

本文汇总对定制 rustdesk 客户端 `/Users/olly/github/rustdesk` 的 Management Agent 与配套后端 `/Users/olly/github/rustdesk-api`（PHP+SQLite+Go broker）的远程救援、保活、兜底恢复、升级能力的只读代码评估，给出逐维判定、待完善缺口（按优先级）、传输设计建议与"少构建"策略。结论基于本仓源码与测试，非上游 rustdesk 假设。

## 0. 总结

远程救援 / 保活 / 兜底恢复 / 升级的**主体链路均已实现且有测试**，全模块 `unimplemented!/todo!/stub` 零命中。待完善项集中在五处：① 两个服务都未配 SC failure actions（进程崩溃无秒级自愈）；② status 投影误报可恢复性；③ 救援命令非近实时（无命令创建唤醒）；④ 单文件损坏无自主修复（缺文件级批准清单）；⑤ total-loss 真装 commit 无 CI。其余为已知设计边界。

## 1. 架构:两套更新链 + 两套命令面(先厘清,避免误判)

- **客户端自带更新器** `src/updater.rs`：客户端进程触发（`flutter_ffi.rs:78`、`rendezvous_mediator.rs:284`），自校验签名/SHA/大小，负责客户端自我更新与 Android/macOS 安装。与 agent 无调用关系，仅共用签名密钥与下载域。25 个测试。
- **Management Agent 远程命令驱动的 MSI 重装/更新** `src/management_agent/`：本文主体。
- **救援命令面** = 设备 Ed25519 签名**轮询** `GET /agent/commands` → `/claim` → `POST /events`（`operations_runtime.rs:423,455,560`）。**不是 SSE 推送。**
- **SSE** `/policy/stream`（Go broker 推）= 仅 `policy_revision` 变更唤醒 + legacy update-command。且 `agent_command_create` 既不 bump `policy_revision` 也不发"命令待取"事件 → 新命令只能等 agent 下一次轮询取到。

## 2. 逐维判定

### 2.1 保活 / 自保护

| 维度 | 状态 | 证据 / 缺口 |
|---|---|---|
| 主服务检测+拉起（SCM 层，agent→主服务） | ✅ 全 | 30s 主循环 `management_agent.rs:1926` `inspect_and_start_service`；`recovery::decide`（`recovery.rs:42`）状态机；`repair_service`（`windows.rs:201` `sc.exe`）；RetryBudget 退避 30/120/300/3600s。测试 `tests/management_agent_recovery.rs`(13) |
| 主程序"Running 但卡死"探活 | ⚠️ 部分 | 核心 `decide` 只看 SCM 状态；进程级探活仅 `local_repair_schedule.rs:385` 走命名管道健康探针 `\\.\pipe\RustDesk\management-health-v2`。卡死不走核心保活路 |
| agent 开机自启 | ✅ 全 | `start= auto`（`src/platform/windows.rs:1690`） |
| **agent/主服务 崩溃秒级自愈（SC failure actions）** | ❌ 缺 | 全仓 `sc failure/SERVICE_FAILURE/SC_ACTION` **0 命中**。进程被杀/崩溃无 SCM 自动重启，最长等 5 分钟计划任务复活 |
| agent 兜底复活（计划任务 + bootstrap） | ✅ 逻辑 / ⚠️ 测试 | 计划任务 `RustDeskManagementAgentRecovery`（Boot PT1M + 每 5 分钟），动作 `rustdesk-recovery-bootstrap.exe bootstrap` → `run_once`（`bootstrap.rs:708`）Stopped→启动/缺失→重建/exe 缺失→哈希恢复。真实 SCM 启动/重建无直接单测，仅策略判定 + py 间接 |
| 主↔agent 互相保活 | ⚠️ 单向 | agent→主有；主→agent 运行期无（主仅提供健康管道，不拉起 agent） |
| recovery 链总开关 | ⚠️ 门禁 | 全链挂 `service-recovery-enabled` 标记，仅停服意图==Enabled 时创建；非 Enabled 时 bootstrap+主服务 recovery 同休眠，只剩 `start= auto` + rescue R3 |

### 2.2 兜底恢复 / 重装

**关键澄清**：`status --json` 的 `ready=false`/`full_recovery_supported=false`/installed_msi `local_repair unsupported` **是 status 投影层的刻意/陈旧契约，不等于执行能力缺失**。`rescue_status.rs:107-137` `projection()` 把 `ready`/`full_recovery_supported` 硬编码为 false（测试 `management_agent_rescue_projection.rs` 锁死）；而恢复执行引擎（schedule→local_repair→total_loss→msiexec）**另成一套、确实支持 MSI 重装**。installed_msi 的 `unsupported` 标记是执行引擎 10/18 加支持后 status 未同步的滞后广告。

| 恢复场景 | 能否自动恢复 | 路径 / 证据 | 缺口 |
|---|---|---|---|
| 单文件损坏（目录在） | ❌ 否（自主） | 无触发器；`normal_missing()` 为 false 即 Observe | 无文件级批准清单（`approved_file_manifest_unavailable`），只能靠远程全量重装 |
| normal 目录整体缺失 | ✅ 是 | `schedule.rs:375-452`→`local_repair` 重装 MSI + 还原 identity（`local_repair.rs:1473,1579` msiexec） | happy-path commit 仅 VM 实测 |
| Total loss（normal+fallback 全无） | ✅ 是 | `schedule.rs:364-374`→`run_total_loss`→`total_loss_runtime.rs:603-631`（停服→msiexec→还原 agent/bootstrap/identity） | commit 仅 `r4_total_loss_red.rs` owner-VM 测，无 CI |
| MSI（installed_msi / schema2） | ✅ 是（执行层） | schedule `:251-285` 显式接受 | **status 仍标 unsupported（投影滞后）→ 运维误判不可恢复** |
| EXE（agent/bootstrap 自身二进制） | ✅ 是 | `agent_restore.rs:176`、`bootstrap_update.rs:197` | — |
| 客户端服务挂（保活非重装） | ✅ 是 | R3 `rescue_recovery`（`management_agent.rs:1790-1854`）启动签名 fallback 服务（degraded） | 保活不重建 normal |

### 2.3 远程命令 / 认证 / 门禁 / 审计

全链硬强制，无"声明要求但未强制"的安全空洞：

- **认证**：设备 sodiumoxide 签名（`operations_runtime.rs:301,223`）+ 身份绑定 + SHA256 digest + command_id + 过期窗口。命令类型白名单（`operations.rs:161`），其它一律 unsupported。
- **本地门禁优先**（两条路径都在动手前拦）：service.*/recovery 走 `windows.rs:137` `mutation_allowed`；package.* 走 `preparation.rs:135` `check_local_state`（调 3 次防 TOCTOU）+ 事务 `locally_blocked`。拦截 disable-agent/disable-recovery/maintenance/stop-intent/install-mutex，均有拒绝测试（`windows.rs:317` 断言 service state/pid/account 全不变）。
- **固定对象/域名**：`package_gate.rs:114` 强制 `download.yan.life/rustdesk/stable/`、无 query/traversal；target_key 仅 `windows-x86_64-msi-standard|sos`；服务名/account/路径全固定；**无任意脚本/路径/端口/进程/env 执行通道**。
- **事务/审计**：`journal.rs`、`transaction.rs`、`command_transaction.rs`、`local_events.rs`，失败断言错误类型+旧数据保留（大量 `*_preserve_evidence` 测试），真 PHP+SQLite+journal E2E。
- **唯一 gap**：`operations_runtime.rs:253` `fixed_integrity_report` 只算哈希、`verified:false`、`reference_status:"approved_file_manifest_unavailable"`——"固定对象校验对照批准清单"未落地（诚实降级，不产生虚假信任，非安全洞）。

### 2.4 升级 / agent 自升级

| 能力 | 状态 | 缺口 / 说明 |
|---|---|---|
| 主程序 MSI 更新（下载/校验/应用/上报/回滚/resume/恰好上报一次） | ✅ 全 + e2e | `install_execution.rs:89`、`native_install.rs`、`package_download.rs`、`package_gate.rs`；e2e `operations_runtime.rs:828-864`（真实本地 PHP+SQLite + 真 MSI） |
| 主程序 EXE 形态远程更新 | ❌ 未接通 | 命令只收 msi key（`operations.rs:335`），`native_install.rs:187` 拒非 MSI。EXE 引擎仅服务 `windows_exe_migration` 迁移 |
| agent 自身更新（handoff/保活/回滚） | ✅ 全 + 测试 | `parent_handoff.rs:60`、`handoff.rs`、resume 任务；版本比较 + 签名 + 回滚 + commit 后重启探针。**= 整包 MSI 重装,无 agent.exe 增量路径** |
| RecoveryBootstrap 随 agent 同步升级/回滚 | ✅ | `bootstrap_update.rs` |
| 批准包重装 package.reinstall | ✅ 全 + 测试 | 与更新同路；**重新下载**（`download.yan.life`），不从 rescue offline_package 取；rescue 快照仅供日后 total-loss 自主修复 |

### 2.5 后端编排

全链端到端 + 真实签名/数据库级测试（见 `tests/agent_*`、`stream-broker/main_test.go`）：命令 create→签名轮询→claim→事件状态机；批准签名包管线严格（固定域名/固定 key/64 字节签名/exact package_id/active stable 匹配/预锁+再锁/回滚包）；实例绑定 CAS、按类型权限门、`management_agent_enabled` 策略门、全程审计；broker 过载 503+Retry-After 不回源、配额、心跳、幂等。

边界：① 救援命令是签名轮询非 SSE 推；② 整套 agent-ops 接口 **SQLite 独占**（MySQL→404，`agent_operations.php:394`）；③ 包管线仅 Windows x86_64；④ 批量 package 救援有意未开（`agent_batches.php:28`，单机可用）。

## 3. 传输设计:SSE vs WebSocket vs 更好方式

**结论:保留"SSE 唤醒 + 签名轮询拉取",不要换 WebSocket。** 唯一值得做的是给命令创建加唤醒。

- 现设计正确：PHP-FPM 扛不住长连接→已有 Go broker 扛 SSE；命令带签名+幂等+claim+事件状态机+事务地"拉"，天然扛漏推/重复/离线/半失败；SSE 穿代理/防火墙稳、自带重连、broker 已做过载保护。
- 不上 WS：客户端→服务端低频且必须签名+事务，用 HTTP POST 即可，WS 双向优势浪费；WS 徒增握手/心跳/nginx 缓冲坑/被企业代理封/难无状态扩展，还要重造已白拿的重连/认证/幂等。
- MQTT 是"从零起、大规模 IoT"的正解（pub/sub/QoS/retained/LWT），但要整套新 broker+认证+运维，当前不值得推翻已测透的 Go broker。gRPC 双向流/APNs 式推送对本场景无实质增益。
- **唯一建议改**：命令入库时在现有 SSE policy 流发轻量 `command_pending`（或 bump 一个 revision）→ agent 收到即刻签名轮询 → 救援命令从"等轮询"变秒级，保留签名拉取健壮性。后端小改 + 客户端零新能力（poll 本就是已有能力）。

## 4. "少构建"/预留接口策略

- agent 是**双向白名单、拒未知**的固定能力执行器（后端 `agent_command_types` 12 类白名单 + `type_version!==1`/未知字段拒绝；创建命令时校验 `in_array(type.'@1', 客户端上报 capabilities)`）。这是安全设计，**"后端下发新逻辑让 agent 执行"被故意堵死**。
- 三个层次：① 选哪个批准签名包/版本下发 = 后端全权，无需碰客户端（已建好且测过）；② 已有能力的变体/编排/目标/批量/策略 = 后端 policy/data 下发；③ **全新动作类型 = 必须构建客户端**。
- **真正的"少构建"杠杆 = `agent.update`（已实现）**：构建一次（CI Tag Build）→ 发布签名包 → 全机群 `agent.update` 自升，省掉手动逐机。即使加新能力，也只需构建一次、发布一次、全群自升。
- 建议：把需客户端改动的待完善项（见第 5 节 P0/P1 中需构建者）**攒成一次客户端构建**发布，经 agent.update 铺开。

## 5. 按优先级的待完善清单

**P0（真缺口，影响核心诉求）**
1. **两个服务都配 SC failure actions**：给 `RustDesk` + `RustDeskManagementAgent` 配 `sc failure ... actions= restart/N reset= M`，进程崩溃秒级自愈，取代"最长等 5 分钟计划任务"。纯增量、小改。
2. **修 status 投影误报**：让 installed_msi 的 `local_repair` 反映真实支持、`ready`/组件状态反映 verified 引擎能力（或后端/控制台不把 `ready=false` 当"不可恢复"）。直接影响远程救援的可观测与决策。

**P1（能力完善，中优先）**
3. **命令近实时**：命令创建加 SSE 唤醒（第 3 节）。后端小改 + 客户端零新能力。
4. **单文件损坏自主修复**：发布带已批准文件清单（sha256），agent 做文件级校验+定点修复，补 `approved_file_manifest_unavailable`。需客户端构建。可一并解决 2.3 的 integrity gap。
5. **total-loss 真装 commit 加 CI/VM 冒烟**：现仅 owner VM 手测（`r4_total_loss_red.rs` vm_only），补可控回归保发布信心。

**P2（已知边界 / 设计选择，按需）**
6. EXE 安装形态不在远程更新覆盖（仅 MSI）。生产是 MSI，影响小。
7. `agent.update` = 整包 MSI 重装，无 agent.exe 增量。安全的设计选择，除非省带宽才动。
8. 后端 agent-ops SQLite-only（MySQL→404）、包管线仅 Windows、批量 package 救援未开。生产 SQLite+Windows+单机救援，当前不影响。
9. 主↔agent 单向保活；recovery 链挂 `service-recovery-enabled` 门禁（停服意图非 Enabled 时休眠）。属设计，知悉即可。

## 6. 约束

- 以上为能力评估，未改任何代码。真要补 P0/P1 须按 TDD（先失败测试→最小实现→补边界）；本地 disable-recovery/disable-agent/签名哈希校验/回滚门禁优先于远程命令；不借补能力改协议/字段语义/固定配置/默认权限。固定配置（密码/ID 服务器/API/下载域/公钥）不变。
- 需客户端改动的项攒成一次构建，经 `agent.update` 铺开，贯彻"少构建"。
