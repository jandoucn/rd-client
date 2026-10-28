# RD-Client / RD-API TDD 总台账（索引）

> 本文件是跨会话的 TDD 总索引，汇总仓库内所有 TDD 计划与执行台账，方便一次看全。
> 每个条目给出：覆盖范围、当前状态、详细台账文件。详细 RED→GREEN 证据见各自文件，本文件只做索引与状态汇总。
> 最后更新：2026-10-27。

## 状态图例

⬜ 未开始 · 🟨 进行中 · ✅ 完成并验证 · ⚠️ 已知限制（有意不修） · ⛔ 阻塞

## 一、详细台账清单

| 台账文件 | 范围 | 日期 | 状态 |
|---|---|---|---|
| [agent-recovery-tdd-plan-20261016.zh.md](agent-recovery-tdd-plan-20261016.zh.md) | Agent 恢复、自动升级、桌面回归的 TDD 计划 | 2026-10-16 | 历史计划，部分并入后续台账 |
| [agent-tdd-execution-ledger-20261016.zh.md](agent-tdd-execution-ledger-20261016.zh.md) | Agent 恢复/升级执行总账与接续队列 | 2026-10-16 | 滚动检查点，非最终交付 |
| [android-four-changes-tdd.zh.md](android-four-changes-tdd.zh.md) | Android 四项行为（禁用双指平移、手势提示、滚轮减速、显示名） | — | 源码与单测完成；APK 真机与覆盖安装未验收 |
| [tdd-ledger-20261022.zh.md](tdd-ledger-20261022.zh.md) | 重装登记、远程操作静默、系统盘等一轮交付 | 2026-10-22 | 见该文件 |
| [tdd-ledger-20261027.zh.md](tdd-ledger-20261027.zh.md) | 本轮：UAC、系统盘、MSI 目录锁定、终端免密、query_uac、后端 UAC 面板 | 2026-10-27 | 🟨 进行中，见下「二」 |

## 二、本轮（2026-10-27）交付汇总

详细 RED→GREEN 与真机证据见 [tdd-ledger-20261027.zh.md](tdd-ledger-20261027.zh.md)。本节只给结论。

### rd-client（客户端 / Agent）

| 代号 | 功能 | 状态 | 关键证据 |
|---|---|---|---|
| U | 安装与 Agent 关闭 UAC（四键写 0，不重启，卸载不恢复） | ✅ | `uac_policy` 单测 win10-gs 13/13；MSI 真机：四键 1/5/1/1 → 全 0，msiexec 退出 0 不重启 |
| D | 不写死系统盘，改 `windows_system_dirs` 运行时查询 | ✅ 单元 / ⚠️ 真机 | `system_dirs` 单测 5/5；无 D 盘系统盘真机，仅单元覆盖；集成测试入口已补模块声明 |
| M | MSI 安装目录锁定、Defender 默认执行、目录页文案修正 | ✅ | 读 MSI 数据库核对：无改目录入口、无 Defender 勾选；传其他目录仍装 Program Files；文案已改 |
| T | 管理员终端免 OS 密码（固定密码 + 一次性密码皆可） | 🟨 | 单测通过；A 方案（旧设备回退对话框）已实现；新控制端连旧设备真机未测 |
| Q | `system.query_uac` 查询能力 + 心跳携带 UAC 快照 | ✅ | 与 UAC 合并，Mac/win10-gs `uac_policy` 全通过 |
| 门禁 | UAC 命令受本地门禁（disable-recovery/agent/maintenance/停止意图），区分错误码 | ✅ | `local_gate_refusal`、`command_error_code` 单测先 RED 后 GREEN |

### rd-api（后端 / 后台）

| 代号 | 功能 | 状态 | 关键证据 |
|---|---|---|---|
| UI | 设备详情「本机 UAC」面板（查询、一键关闭、状态展示） | ✅ | E2E `agent_uac_panel.spec.py` 5 场景桌面+390px 全通过 |
| 入库 | 心跳 UAC 快照校验后原样入库、按四键推导 fully_disabled、保留未知字段 | ✅ | `agent_status_test.php` 96 断言，含未知字段保留与矛盾自报两条新测 |
| 文案 | UAC 错误码中文文案、失败原因展示、刷新不回退、弹窗按命令类型显示 | ✅ | typecheck + lint 通过；E2E test_04 失败原因、test_03 刷新防回退（变异验证有效） |
| 其他 | 批量升级、安装包预检、更新记录、安装包身份落库（另一会话负责） | 🟨 | 审查 5/6/7/8/9 项由对应会话按 TDD 修复中 |

### 收尾门禁状态

| 项 | 状态 |
|---|---|
| rd-client Mac 单测（uac_policy/system_dirs 等） | ✅ |
| rd-client win10-gs agent/lib 测试 | ✅ 环境门禁外全通过 |
| rd-client win10-gs clippy | ✅ 本轮新增代码 0 诊断 |
| rd-client win10-gs 集成测试（26 目标） | ✅ system_dirs 编译断裂全部修复并重验，无代码逻辑失败；⚠️ 余留 VM driver/guest driver 门禁与预置演练证据类用例待改造成 win10-gs 真机门禁 |
| rd-client Flutter analyze + test | ✅ |
| rd-api PHP 单测（17 文件） | ✅ |
| rd-api Python HTTP/集成（565 项） | ✅ 除环境缺失跳过 |
| rd-api 前端 typecheck/build/lint | ✅ |
| release-notes.zh.md | ⬜ |
| AOCI 对齐 | ⬜ |
| 安全复审 | 🟨 code-reviewer 已出报告，security-reviewer 未做 |

## 三、跨机器测试环境约定

win10-gs 真机测试环境、PHP/HTTP 夹具变量、实验室门禁开关、SYSTEM 缓存路径等，统一记在仓库根 `AGENTS.md` 的「Windows 本地编译环境」与「测试与验收约束」小节。所有带 VM 的历史约束已按用户要求改为以 win10-gs 真机为准。
