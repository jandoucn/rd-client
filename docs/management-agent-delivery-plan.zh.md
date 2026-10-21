# Management Agent 能力交付 + 后端重构 执行计划与测试记录

本文是本轮交付的 TDD/E2E 执行计划与**滚动测试记录**（每阶段完成后在对应小节追加实测结果/日志摘要/修复）。配套能力评估见 `docs/management-agent-capability-assessment.zh.md`。

## 1. 范围

1. 后端重构（`~/github/rustdesk-api`）：① 能力注册表数据化；③ 后端镜像客户端 status schema（rescue/files），修 status 误报。纯后端，不卡 Windows 构建。
2. 验证现有安装/升级/Management Agent 全能力流程，抓日志修现有 bug。
3. P0 客户端（`~/github/rustdesk`）：SC failure actions（崩溃秒级自愈）+ status 投影修正。

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
- 实测记录：_（待填）_

### Phase 4 — P0 客户端 SC failure actions
目标：给 `RustDesk` + `RustDeskManagementAgent` 服务注册 SC failure actions（崩溃自动重启），缩短崩溃恢复从分钟级→秒级；纯增量，不改既有恢复链。
- 失败测试（Rust）：服务安装/注册路径产出含 failure actions 的配置；禁用/维护意图下不改变（与既有门禁一致）；回归既有 service 安装测试。
- 机器验收（win10-gs）：kill agent/主进程 → 观测 SCM 自动重启秒级；与计划任务兜底不冲突。
- 实测记录：_（待填）_

### Phase 5 — P0 客户端 status 投影修正
目标：`rescue_status.rs` 投影让 installed_msi 的 `local_repair`/组件状态反映引擎真实支持（与后端 Phase 2 对齐），不再误报"不可恢复"；`ready` 字段契约保持（如仍 false 则文档化语义），重点是组件级真实性。
- 失败测试：installed_msi + 引擎支持 total-loss/local_repair 时，投影不得输出 `installed_msi_local_repair_not_supported`；组件 verified 正确透出。
- 机器验收：win10-gs 实测 status 与真实恢复能力一致。
- 实测记录：_（待填）_

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
- 机器分工（编译+实测 win10-gs / 后端联调 NTServer-SH）以本文为准，可按用户调整。
