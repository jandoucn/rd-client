# SYSTEM 终端 实现台账（TDD）

> 本文件是本轮唯一执行台账。用户要求随时可见 TDD 进度：每完成一项立即改状态并写证据，不写「应该通过」。
> 状态：⬜ 未开始 · 🟨 进行中 · ✅ 完成 · ⛔ 阻塞 · ⚠️ 已知限制
> 最后更新：2026-10-28 18:31（现网 agent 已换成审查修复后 debug 二进制，生产链路 whoami/resize/attach 冒烟全绿，见 §6 复测）
>
> ✅ 治理已放行（用户 2026-10-27 确认）：「SYSTEM 终端授权例外」已写入 rd-client 与 rd-api 两个 AGENTS.md。T2 按协议走独立 WS（不接命令通道），见 overnight-notes 特殊情况 1。契约《实现澄清补丁 v1》见 `docs/terminal-protocol.zh.md` §10。
>
> ✅ rd-api 侧证据：`go test ./...`；`python3 -m unittest terminal_{http,nginx_config,e2e}_test` 19/19；`node --test channel.test.ts` 12/12；`pnpm typecheck`；`python e2e/terminal.spec.py` 5/5（桌面+390）。修复：SPA 白名单补 `$admin/terminal`；会话列表经分流代理；粘贴异步等待；只掐 Agent WS 测 device_gone；窄屏 overflow 用 clientWidth。
>
> rd-api 详细交接（文件清单 / 复现命令 / 坑 / 未做项）：**`/Users/olly/github/rd-api/docs/terminal-rd-api-progress.zh.md`**。特殊情况见 `docs/overnight-notes-20261028.zh.md`。总交接见 `docs/handoff-20261027.zh.md` §二。
>
> win10-gs 可复用脚本：`scripts/win10-gs/`（`sync-sources.sh` / `run-build.sh` / `build-agent.ps1` / `replace-agent.ps1` / `run-terminal-tests.ps1`）。
>
> 文档索引：`docs/terminal-docs-index.zh.md`。证据总册：`docs/terminal-test-evidence.zh.md`。时间线：`docs/terminal-test-record.zh.md`。脱敏摘录：`docs/evidence/terminal-v*.txt`。

## 0. 已确认的方案（用户 2026-10-27 拍板）

| 项 | 决定 |
|---|---|
| PTY | 项目现有 `portable-pty` fork（`rustdesk/pty_based_0.8.1`），不引入新依赖 |
| 会话承载 | **独立 helper 进程**，复用现有 `parent_handoff` / `helper_runtime` 机制；会话不随 Agent 重启丢失 |
| 传输 | **WebSocket**，带单调序号 + 环形缓冲 + 断点续传（不做 SSE/长轮询折中） |
| 部署 | WS 挂进现有 stream-broker，不新起进程 |
| 顺序 | 先做网页终端；RustDesk 客户端的 SYSTEM 终端窗口放后 |
| 兜底 | 轮询降频保留（3 秒 → 30 秒），WS 断开时页面不失数据 |

## 1. 范围

### 本轮做

1. Agent 侧：新增长期存活的终端 helper 进程（ConPTY + PowerShell/cmd），以 SYSTEM 身份运行；
2. Agent 侧：新增终端会话命令（打开/写入/读取/调整大小/关闭），复用现有签名命令通道；
3. 后端：WebSocket 终端中转，复用 stream-broker 的鉴权与连接管理；
4. 前端：独立的「终端」页面，xterm.js 渲染，多标签、断线重连；
5. 门禁：本地门禁优先（`disable-agent` / 维护态 / 停止意图），只读查询不受阻；
6. 审计：会话开启/关闭/关键操作留审计记录。

### 本轮不做

- RustDesk 客户端内的 SYSTEM 终端窗口（下一轮）
- 文件上传/下载（下一轮）
- 定时/条件触发、安装前/卸载后钩子（另立轮次）
- 作业式脚本执行（`system.run`）：工作区里有半成品 `script_run.rs`，本轮先不用，收尾时决定保留或删除

## 2. 交付阶段与 TDD 条目

### 阶段一：Agent 终端 helper（客户端的合同）

| # | 条目 | 层 | 状态 |
|---|---|---|---|
| T1.1 | ConPTY 会话能创建，三种 shell 均可指定；`pwsh` 不存在明确报错不回退 | 单元（Windows） | ✅ `powershell/cmd_session_prints_computed_output`、`pwsh_runs_when_installed_and_is_reported_otherwise` 真机过 |
| T1.2 | 输入输出字节流往返，原样 | 单元（Windows） | ✅ 含 `chinese_output_is_utf8` / `chinese_input_reaches_shell_as_text` 真机过 |
| T1.3 | 行列尺寸调整生效（`ResizePseudoConsole`） | 单元（Windows） | ✅ `resize_changes_console_window` 真机过；全屏 TUI 见 V2 ✅ |
| T1.4 | 会话关闭/丢弃后子进程树完整终止（Job KILL_ON_JOB_CLOSE），含孤儿子进程 | 单元（Windows） | ✅ `close_terminates_whole_process_tree`、`close_after_shell_exit_still_ends_orphaned_children`、`dropping_session_ends_process_tree` 真机过 |
| T1.5 | 输出环形缓冲：超限丢旧、序号连续、任意保留点续读；按字节计入每块开销，单条读有字节上限 | 单元 | ✅ `output_ring` 10 个用例真机过（含 `tiny_chunks_cannot_exceed_memory_budget`、`reads_stop_at_byte_limit`） |
| T1.6 | Agent 重启后会话可重连 | 集成 | ⏭️ v2（rd-api 已核：v1 不做断线续会话；需新增 broker→agent attach{session_id,last_seq}。v1 用进程内 SessionManager，重连即全新开始） |
| T1.7 | helper 进程身份校验 | 单元 + 集成 | ⏭️ v2（随 T1.6；v1 无独立 helper 进程，PTY 在 Agent 进程内） |
| T1.8 | 空闲/最长超时判定（30 分 / 24 时） | 单元 | ✅ `policy_reports_idle_and_lifetime_expiry`、`default_policy_follows_protocol_timeouts`、`writes_refresh_idle_timer` |

### 阶段二：Agent 终端 WS 帧与门禁（协议优先：独立 WS，不走命令通道）

| # | 条目 | 层 | 状态 |
|---|---|---|---|
| T2.1 | open/input/resize/close/ping 五类帧参数校验（未知键、超长、非法 shell、非法会话号均拒绝） | 单元 | ✅ Mac：`protocol::tests::*`、`manager::tests::validate_open_*`、`transport::tests::open_*` 等 66 项全绿 |
| T2.2 | 本地门禁：`disable-agent` / `maintenance.json` / 停止意图 / disable-recovery 存在时，`open` 与 `input` 被拒并返回 `terminal_local_gate`；`close` 仍可用 | 单元 | ✅ `manager::tests::each_gate_refuses_open_*`、`gate_refuses_input_but_allows_read_resize_close`；`transport::tests::gate_refusal_*`、`input_under_gate_*` |
| T2.3 | 会话数上限 4，超限拒绝而非静默丢弃 | 单元 | ✅ `manager::tests::session_limit_rejects_fifth_but_not_fourth`；`transport::tests::fifth_open_hits_session_limit` |
| T2.4 | 会话开启/关闭/被门禁拒绝均写审计事件（只元数据） | 单元 | ✅ `manager::tests::open_register_emits_audit_*`、`close_runs_slot_close_and_emits_audit`、`each_gate_refuses_open_with_specific_audit`；runtime `log_audit` 只打 info 元数据 |
| T2.5 | 终端长连接不受命令 900 秒超时约束（独立 idle 30 分 / 寿命 24 时） | 单元 | ✅ `session_tests::default_policy_follows_protocol_timeouts`、`policy_reports_idle_and_lifetime_expiry`、`reap_expired_closes_idle_sessions`；runtime 在独立线程、不走 commands 超时 |
| T2.6 | 能力声明新增 `system.terminal@1`；未声明的旧 Agent 后台不显示终端入口 | 单元 + 契约 | ✅ `status.rs` `capabilities_for` 在 windows x86_64 追加 `system.terminal@1`；MSI 能力计数 15 / 非 MSI 12；`runtime::run` 已挂 `service_main` 的 `agent-terminal` 线程 |

### 阶段三：后端 WebSocket 中转

| # | 条目 | 层 | 状态 |
|---|---|---|---|
| T3.1 | WS 握手鉴权：管理员会话 + CSRF；未登录拒绝 | 集成 | ✅ rd-api：broker `TestTerminalBrowserHandshakeRequiresSessionCSRFAndSameOrigin`（缺 CSRF/跨源/缺 Origin 不触达 PHP 即 401）；PHP `terminal_http_test`（admin-auth：全局管理员+CSRF+管理路径）；全链路 `terminal_e2e_test.test_handshake_rejections`（真实 PHP+SQLite，伪造签名 401、未声明能力 403） |
| T3.2 | 一条 WS 连接绑定一个终端会话，会话不属于当前管理员时拒绝 | 集成 | ✅ 实现为同一连接按 session_id 复用（契约 §2 消息均带 session_id），归属按管理员校验：`TestTerminalSessionsBelongOnlyToTheirAdministrator`（他人 attach/input/resize/close 一律 terminal_not_found）、`test_rest_close_and_cross_admin_isolation` |
| T3.3 | 浏览器 ↔ 后端 ↔ Agent 三段转发：输入到达 Agent、输出到达浏览器 | 集成 | ✅ `TestTerminalRelaysOpenInputOutputResizeAndClose`（未知字段不透传）；`terminal_e2e_test.test_full_session_relays_bytes_and_audits_metadata_only`（中文字节往返、审计逐行查库且不含输入输出） |
| T3.4 | 序号与断点续传：断线重连带 `last_seq`，服务端补发缓冲内缺失部分 | 集成 | ✅ 2MB 环只存 seq>0 帧：`TestTerminalAttachReplaysMissedOutputAndReportsGaps`（补发+terminal_gap_too_large+被顶替窗口收到提示）、`TestOutputRingDropsOldestAndTracksGaps`、`test_reconnect_replays_missed_output` |
| T3.5 | 背压：浏览器读得慢时暂停向 Agent 拉取，不无限堆积内存 | 集成 | ✅ 浏览器发送队列 >1MB 停读 Agent 连接、<256KB 恢复（§10 第3条）：`TestTerminalBackpressurePausesAgentReadsUntilBrowserDrains` |
| T3.6 | 会话列表与回收：列出我的会话、关闭、空闲超时清理 | 集成 | ✅ broker 直接提供 `GET /{admin}/api/terminal/sessions`、`POST …/sessions/{id}/close`：`TestTerminalSessionsAPIListsAndClosesOwnSessions`、`TestTerminalIdleAndLifetimeExpiry`（空闲 idle、24h 到期 error）、`TestTerminalStartTimeoutReleasesTheSession`、`TestTerminalDeviceDisconnectEndsSessionsAsDeviceGone` |
| T3.7 | 并发限制与限流：单管理员会话数上限、单设备会话数上限 | 集成 | ✅ `TestTerminalRejectsOfflineDevicesAndEnforcesSessionLimits`（8/4 上限、离线设备）；被本地门禁拒绝的会话不占名额：`TestTerminalLocalGateRefusalIsForwardedAndAudited`、`test_local_gate_refusal_is_audited` |

### 阶段四：前端终端页面

| # | 条目 | 层 | 状态 |
|---|---|---|---|
| T4.1 | 独立「终端」菜单，列出设备与已有会话，可新建 | E2E | ✅ `terminal.spec.py`：菜单可见、设备选择、打开标签；刷新后 detached 列表可「继续」 |
| T4.2 | xterm.js 渲染：颜色、光标、清屏、全屏 TUI 正常 | E2E | ✅ 基础渲染与提示符/输出写入已覆盖（E2E）；全屏 TUI/resize 真机见 V2。⚠️ **未单独做颜色/光标/清屏专项断言**，依赖 xterm 默认行为 + 输出可见 |
| T4.3 | 键盘输入、复制粘贴、Ctrl+C 中断正常 | E2E | ✅ `test_open_type_interrupt_and_end_session`：whoami、粘贴 Get-Process、键盘 Ctrl+C + 按钮各一次 |
| T4.4 | 窗口尺寸变化同步行列，不错位 | E2E | ✅ `test_reconnect_restores_missed_output_and_resize_syncs` 改 viewport 后 Agent 收到 resize |
| T4.5 | 断线自动重连，重连后补齐断线期间输出 | E2E | ✅ 只掐浏览器 WS：状态「重连」→「已连接」，补齐 `while-away-output` |
| T4.6 | 多标签：同时开 2 个会话互不干扰 | E2E | ✅ `test_multiple_tabs_are_independent`：输入只进当前 tab 的 session_id |
| T4.7 | 390px 宽度可用（终端可横向滚动，不撑破布局） | E2E | ✅ desktop+mobile 子测；viewport 内横向滚动、`scrollWidth-clientWidth<=0` |
| T4.8 | 会话关闭/空闲超时后界面明确提示 | E2E | ✅ 结束会话确认框 + 状态「已结束」；device_gone 写「设备连接已断开」；本地门禁写拒绝原因 |

### 阶段五：兜底与降频

| # | 条目 | 层 | 状态 |
|---|---|---|---|
| T5.1 | 设备列表轮询 5 秒 → 30 秒；详情抽屉 3 秒 → 30 秒 | E2E | ⛔ 用户确认取消（2026-10-27）：轮询保持 5s/3s 不变 |
| T5.2 | WS 断开时页面仍能靠轮询获得数据，不出现空白 | E2E | ⛔ 随 T5.1 否决；保留现有轮询即满足“WS 断开不空白” |

## 3. 联调与真机验证

| # | 项目 | 环境 | 状态 |
|---|---|---|---|
| V1 | win10-gs 真机(SYSTEM)：PowerShell/cmd 会话出输出、中文不乱码 | win10-gs | ✅ 2026-10-27 20:48，`nt authority\system` 下 32 用例全绿 |
| V2 | win10-gs 真机：全屏 TUI（`vim` 或 `top` 等价物）正常 | win10-gs | ✅ 另：`fullscreen_tui_survives_large_resize_and_redraws` 单元 exit0（VT 备用屏）；✅ 生产链路 resize 80x24→140x45 后 `$Host.UI.RawUI.WindowSize` 回报 `V2_COLS=140 V2_ROWS=45`；机上无 vim，用 ConPTY+PowerShell 窗口尺寸验证 |
| V3 | Agent 重启后会话可重连 | win10-gs | ⏭️ v2（v1 不做断线续会话，见 T1.6） |
| V4 | ego 浏览器：真实登录后台、打开终端、敲命令看到输出 | ego + 生产/本地 | ✅ SPA 200 + 审计 `terminal_open`；WSS open/input/close：`whoami`=`nt authority\system`、`WindowsIdentity`=`NT AUTHORITY\SYSTEM`、`__V4_OK__`（设备 1338209581）；IO 不落库符合契约 |
| V5 | ego 浏览器：断网重连后输出补齐 | ego | ✅ 断 WS 不 close 会话 → 再 attach 后仍可输入，`V5_AFTER` 回显；marker 回放未在 attach 帧中观测到（last_seq 已近顶端时可能无可补帧），会话续存成立 |
| V6 | 生产后端上线后：用生产后台 + win10-gs 做一次完整演练 | 生产 | ✅ 0.1.56 + 设备 1338209581 caps/`system.terminal@1` + WS ESTABLISHED；V2 resize 140×45、V4 whoami=SYSTEM、V5 attach 后续输入均绿；审计有 `terminal_open` |

## 4. 收尾门禁

| # | 项 | 状态 |
|---|---|---|
| G1 | Mac：相关模块单元测试全通过 | ✅ `cargo test --bin rustdesk-management-agent -- terminal::` 66 passed |
| G2 | win10-gs：`cargo test --locked` 相关目标通过、clippy 无新增 | ✅ build exit0；`terminal::` **71 passed**；现网 agent `3e588920…`（含 terminal 字符串），服务 Running；clippy 仅旧告警 |
| G3 | 后端：PHP 单测 + Python 全量 + 前端 typecheck/lint/build | ✅ **终端相关子集**已绿（Go/Python terminal_* 19 + channel 12 + typecheck + Playwright 5/5）；CI test/publish 0.1.56 绿并生产已升。⚠️ **未在本轮单独重跑仓库级 Python 全量 575**，以 CI test job 为准 |
| G4 | E2E：阶段四、五全部通过（桌面 + 390px） | ✅ 阶段四 `terminal.spec.py` 5/5；阶段五已否决 |
| G5 | 文档：终端使用说明 + 安全边界说明 | ✅ usage + protocol + 台账 + evidence 总册 + test-record + evidence/ 摘录 + client 安全笔记 |
| G6 | 代码审查：code-reviewer + security-reviewer | ✅ api：`rd-api/docs/terminal-security-review.zh.md`；client：`docs/terminal-client-security-notes.zh.md`（0 CRITICAL/HIGH） |
| G7 | AOCI 对齐 | ⚠️ 本会话 aoci MCP handshake 失败（connection closed）；业务与台账已稳，AOCI 待 MCP 恢复后补一次 maintain |
| G8 | 清理：删掉未完成的 `script_run.rs` 或明确保留 | ⚠️ 本轮**保留**不删（作业式半成品，收尾另定；不声明能力） |

## 5. 风险与退出条件

| 风险 | 处理 |
|---|---|
| ConPTY 在服务进程里启动黄死 | 复用项目中已解决的 helper 模式；若仍卡死，退回到「客户端侧 helper 已验证的实现」照搬 |
| Agent 重启导致会话丢失 | helper 独立进程 + 会话落盘；若做不到，降级为「重连时提示会话已失效」并记录 |
| WS 在现有 broker 上难接入 | 退路：WS 单独起一个 broker 进程，鉴权复用现有会话校验 |
| helper 进程被第三方利用 | 只允许 Agent 拉起、只绑定本机固定管道、进程身份校验 |
| 900 秒命令超时杀掉长会话 | 终端会话不走命令超时，改为独立的空闲超时 |

## 6. 2026-10-28 审查修复（客户端 Agent）

审查发现经代码核对与真机/生产复现后，按用户分工：客户端 `terminal/` 由本会话修复；前端与 broker 后续项交给「Terminal 相关改动审查」会话，**需用户在该会话确认后开工**。

| # | 问题 | 证据（修复前） | 修复 | 状态 |
|---|---|---|---|---|
| R1 | shell 自行退出时尾部输出丢失：pump 每轮只读 64KB 就回收会话；conhost 尚未画进管道的最后一段在立即关闭伪控制台时被丢弃 | win10-gs 探针：cmd 输出约 295KB 后立即 `exit`，10 轮丢 1 轮；只加「先收齐再回收」后仍约 1/30 丢失 | `PtySession::finish_output`：先等输出静默 200ms（上限 2s）→ 关伪控制台 → 等读线程读完；`SessionManager::settle_exited` 收尾、`drain_exited` 只回收已收尾会话；`transport::drain_all` 一次推完剩余输出再发 `exited` | ✅ 红：`pump_order_delivers_tail_output_of_exiting_shell`（30 轮）3 次跑 2 次失败；绿：3×30 轮 0 丢失 |
| R2 | WS 握手无读写超时：服务端接 TCP 不回包时终端线程永久阻塞、不再重连 | 代码核对 | 拆出 `runtime::upgrade`，握手期间 TCP 读写超时 10s，握手后恢复原读超时、写阻塞 | ✅ 红/绿：`upgrade_gives_up_when_server_accepts_but_never_answers`（win10-gs） |
| R3 | 24h 到期 `closed.reason` 报 `idle` | 代码核对 | `reap_expired` 按 `Expiry` 映射：Idle→idle，MaxLifetime→error | ✅ `reap_reports_max_lifetime_as_error` |
| R4 | `exited.duration_ms` 恒 0 | 代码核对 | `transport::exit_frames` 取审计事件时长 | ✅ `exit_frames_*` |
| R5 | `next_push` 畸形缺口（oldest=0 或 ≤ 游标）可 panic/游标后退（另一会话独立审查 L5/L6） | 代码核对 | `oldest.saturating_sub(1).max(last)`，doc 更正 | ✅ `malformed_gap_never_moves_cursor_backwards` |
| R6 | `drain_all` 后续轮次遇到缺口且重同步为空时丢掉缺口报告（代码审查 L1） | 代码审查 | 缺口与游标在空结果时也合并 | ✅ `drain_all_keeps_a_late_gap_even_when_resync_is_empty` |
| H1 | 输出积压超过 4MB 环后 pump 永久卡住（另一会话修复 `transport::next_push`） | — | 由「Terminal 相关改动审查」会话完成 | ✅ 该会话 6 条用例 |

测试回执：Mac `terminal::` 83/83；win10-gs `terminal::` 91/91（debug）；Mac clippy 终端文件无告警。⚠️ win10-gs clippy 未能跑完：`rustdesk` lib 既有 16 个 deny 级错误（`src/platform/windows.rs`、`src/privacy_mode/win_virtual_display.rs`），与终端无关；另 `vcvars64.bat` 会把 `VCPKG_ROOT` 覆盖成 C 盘缺头文件的副本，clippy 需在 vcvars 之后重设。

**现网替换 + 生产冒烟（2026-10-28 18:25–18:31）**

| 项 | 结果 |
|---|---|
| 替换 | `replace-agent.ps1`；备份 `D:\tmp\rdtest\agent-backup\rustdesk-management-agent-20261028-182507.exe`（旧 sha `3e588920…`） |
| 现网 sha256 | `39eb81cafd669f4e27e1e0ad0b2918d709fe15894fa520ed8135eabebaf46462`（与 `target\debug` 构建一致） |
| 服务 | `RustDeskManagementAgent` Running；PID 30256 起于 18:25:10；出站 `47.100.7.221:443` ESTABLISHED |
| 设备 | `1338209581`（yan），`terminal.available=true` |
| V4 whoami | `nt authority\system` + `NT AUTHORITY\SYSTEM` + `__V4_OK__` |
| V2 resize | `V2_COLS=140 V2_ROWS=45` |
| V5 attach | 断浏览器 WS 不 close → attach 后 `V5_AFTER` 回显；补发约 4120 字节 |
| 清理 | 临时 `rd_admin` 会话与铸造脚本已删 |

## 7. 2026-10-29 无 ConPTY 备选（GEP / Server 2016）

GEP（`505054514`，Windows Server 2016 Datacenter 14393）网页终端四次 `terminal_open_failed` / `terminal_start_failed`。同版本 Agent 在 Windows 10 上 `terminal_open` 成功。原因是 `CreatePseudoConsole` 从 Windows 10 1809 才有，Server 2016 没有，现有 `portable-pty` 启动必然失败。网页把审计一律标成「✓ 成功」是展示问题，不是这次启动失败的原因。

| 项 | 决定 |
|---|---|
| 现有路径 | 不改。`kernel32` 导出 `CreatePseudoConsole` 时仍只走 `PtySession::open_conpty` |
| 备选 | `console_host::PipeSession`：匿名管道 + `CreateProcessW`，三种 shell 共用 |
| 句柄 | 只把子进程需要的管道端列入 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`，避免继承 Agent 自己的写端 |
| 参数 | 命令行按 CreateProcess 规则加引号。PowerShell 的 `-Command` 含空格，不能按空格拆 |
| 尺寸 | 管道没有控制台窗口，`resize` 成功但不生效。全屏 TUI 仍只在 ConPTY 上成立 |
| 结束 | 仍归入 KillOnClose Job，关闭会话杀整棵进程树 |
| 编译 | 只编 `rustdesk-management-agent`。终端启动不在主客户端里 |

### 实测踩到的四个问题

1. **`CreatePipe` 给的是不可继承句柄**。`CreateProcessW` 带 `bInheritHandles=true` 且列了 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` 时，列出的句柄必须本身可继承，否则整次创建直接失败 `ERROR_INVALID_PARAMETER`，网页只看到 `terminal_start_failed`。修法是对子进程那两端显式 `SetHandleInformation(HANDLE_FLAG_INHERIT)`。
2. **管道下的 cmd 是纯行输入**。逐字不回显，退格也只当普通字节：实测 `wxyz` + 4 个退格 + `hoami` + 回车什么都不执行，因为命令行实际成了 `wxyz\b\b\b\bhoami`。修法是 shell 用 `/Q` 关掉自己的整行回显，由 Agent 本地做行编辑：按键即时回显、退格删一个完整字符、只在回车时把整行加 CRLF 交给 cmd。PowerShell 自己逐字回显，不能补第二遍。
3. **裸 CR 不执行**。xterm 回车发的是 `\r`，cmd 只认 CRLF。PowerShell 能执行裸 CR，但输出行尾缺 LF。
4. **shell 按系统 ANSI 代码页读写**。管道进程没有控制台，cmd 与 PowerShell 都用 ACP（GEP 是 936/GBK），浏览器是 UTF-8，中文全乱码。不能靠 `chcp 65001`：实测它只改输出代码页，cmd 解析 stdin 那条命令行时仍按启动时的 936，UTF-8 中文被拆成非法字节后整条命令变成 `More?`。修法是 Agent 里做双向转码，并处理 DBCS 前导字节被管道读切断的情况。

### 验收

win10-gs 编译（2026-10-29）：`cargo test --locked --release --bin rustdesk-management-agent -- terminal::` 101 passed / 0 failed。构建产物 `4457f5c015000deb82d7482f8d0e68f2855c8f610a64e007ff3cbe5a8ef5a3eb`。

GEP 现网替换后：服务 `RustDeskManagementAgent` Running；`terminal_open` 成功；PowerShell 与 cmd 都能输入、都能回显、命令能执行；`echo 中文测试` 显示正常，中文写入命令可执行。用户在网页上确认「没问题了」。

已知限制：管道没有控制台窗口，`resize` 成功但不生效，全屏 TUI 只在 ConPTY 上成立。

已知限制（代码审查确认，本轮不改）：
- shell 自行退出时 pump 线程逐个收尾，每个约 200ms、最坏 2s 静默 + 2s 读线程；4 个会话同时退出最坏约 16s，期间其他会话的输入与输出暂停（心跳仍在 60s 判死线内）。
- 读线程 2s 内未结束时仍按已收尾处理，之后到达的输出会丢。
- 握手超时按单次读写计算，服务端持续慢速回包时整体握手可超过 10s。
- 测试盲区：Unix 读端在子进程退出后本就不丢尾部，Mac 测不出 R1；R1、R2 的有效用例只在 Windows 编译运行。`pump_order` 用例复刻 pump 顺序而非调用 `runtime::pump`，pump 内的游标写回与帧发送接线无直接用例；`upgrade` 用例只覆盖明文 `ws://`，未覆盖 TLS 握手超时。

未修（后续项，待用户分配）：
- broker：outbox 无上限（已绑定设备可撑爆内存）、attach 补发与实时输出竞态（补发被前端去重丢弃）、慢浏览器阻塞同设备全部会话、鉴权只在握手时做一次（另一会话本轮正在修吊销复核与孤儿 shell）。
- 前端：重连中「结束会话」不发送。
- ✅ rd-api 0.1.57（生产已升级，提交 61b7b07 + 005a067，tag v0.1.57）已复测通过：
  - 被「继续」顶替的旧标签：标签变「已在其他窗口打开」，旧标签输入不在设备执行；新窗口输入正常。
  - 大段粘贴：170KB 与 200KB 均按 32KB 分片完整到达（`LEN-174080`、`LEN-204800`，19s / 23s），之后会话仍可用。
  - 断线重连输入暂停：掐断浏览器 WS 后出现「发送 Ctrl+C 并恢复输入」「恢复输入」，暂停期间输入既不回显也不执行；点「恢复输入」后输入正常。
  - 连带修复（同一版本，未见问题）：鉴权在 open/attach 前复核并每 60 秒复核；重连中「结束会话」走 REST；broker 1013/4013 关闭码与优雅关闭；容器捕获 SIGQUIT，`docker stop` 1 秒内正常退出。
- ✅ broker 三项已修（rd-api 工作区待提交）：outbox 加 8 MB 硬上限并改为会话级投递协程，attach 补发与实时输出按同一游标串行，慢浏览器只暂停它自己的会话。修复过程中另发现并修掉「对端不读时关闭连接自锁」：写锁被卡住的写操作占着，`CloseWithCode` 会一直等同一把锁，调用方还握着会话锁，`finish` 随之挂死。
- ✅ rd-api 前端：工具栏新增「选中即复制」开关（默认关闭、状态存 localStorage），开启后选中即写剪贴板（去抖、同段不重复写、失败只提示一次）；终端内支持 Ctrl+V 与 Cmd+V 粘贴，只处理 keydown 以免 xterm 对同一次按键调两次处理器导致粘贴两遍。
- 工作区未提交的 `system.run` 接入命令通道（operations.rs / operations_runtime.rs）与 AGENTS「命令通道禁止任意执行」冲突，不属终端范围，勿随终端提交。

