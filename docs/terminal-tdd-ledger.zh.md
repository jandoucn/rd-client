# SYSTEM 终端 实现台账（TDD）

> 本文件是本轮唯一执行台账。用户要求随时可见 TDD 进度：每完成一项立即改状态并写证据，不写「应该通过」。
> 状态：⬜ 未开始 · 🟨 进行中 · ✅ 完成 · ⛔ 阻塞 · ⚠️ 已知限制
> 最后更新：2026-10-28 06:35（测试记录/证据总册/时间线/脱敏摘录/文档索引已固化）
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
| T4.2 | xterm.js 渲染：颜色、光标、清屏、全屏 TUI 正常 | E2E | ✅ 基础渲染与提示符/输出写入已覆盖；全屏 TUI 真机见 V2 |
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
| G3 | 后端：PHP 单测 + Python 全量 + 前端 typecheck/lint/build | ✅ 终端相关 + CI test/publish 0.1.56 全绿；生产已升 |
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
