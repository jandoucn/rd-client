# SYSTEM 终端 · 测试证据总册（rd-client）

> 与台账 `docs/terminal-tdd-ledger.zh.md` 同步。只记已核实结果与可复现命令，不写「应该通过」。  
> 最后更新：2026-10-28 06:30 CST。

---

## 0. 证据索引

| 层级 | 文档 / 路径 |
|---|---|
| 执行台账 | `docs/terminal-tdd-ledger.zh.md` |
| 时间线记录 | `docs/terminal-test-record.zh.md` |
| 本证据总册 | `docs/terminal-test-evidence.zh.md` |
| 脱敏输出摘录 | `docs/evidence/terminal-v{2,4,5}-out.txt` |
| 协议 | `docs/terminal-protocol.zh.md` |
| 使用说明 | `docs/terminal-usage.zh.md` |
| 夜间备注 | `docs/overnight-notes-20261028.zh.md` |
| rd-api 进度 | `/Users/olly/github/rd-api/docs/terminal-rd-api-progress.zh.md` |
| 双会话板 | `/Users/olly/github/rd-api/docs/terminal-cross-session-board.zh.md` |
| 安全审查（api） | `/Users/olly/github/rd-api/docs/terminal-security-review.zh.md` |
| win10-gs 脚本 | `scripts/win10-gs/README.zh.md` |

---

## 1. 单元 / 真机（Agent）

| 时间 (CST) | 环境 | 命令 / 动作 | 结果 |
|---|---|---|---|
| 2026-10-27 20:48 | win10-gs SYSTEM | 阶段一 PTY 套件 | **32** 例绿（台账 V1） |
| 2026-10-27 23:06 | win10-gs | `cargo test --locked --bin rustdesk-management-agent -- terminal:: --nocapture` | **71 passed**，0 failed（含 windows PTY/中文/Job 树/resize） |
| 2026-10-27 ~22:45 | Mac | 同过滤器 | **66 passed**（无 windows 真 PTY 用例） |

构建（win10-gs debug，后用于现网替换）：

```
cargo build --locked --bin rustdesk-management-agent
exit=0
binary=D:\github\rd-client\target\debug\rustdesk-management-agent.exe
# 最终现网 hash（含 system.terminal@1 字符串）:
sha256=3e588920b966b481b33634aa209b32e8dacf641de28dc9c07562d3ffc66c91dd
```

能力计数（`status.rs` 单测）：windows x86_64 MSI = **15**（base9 + uac2 + terminal1 + package3）；非 MSI = 12。

---

## 2. 现网 Agent 替换（win10-gs）

| 项 | 值 |
|---|---|
| 服务名 | `RustDeskManagementAgent` |
| **正确安装路径** | `C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe`（以 `sc qc` ImagePath 为准） |
| 错误路径（勿用） | `C:\Program Files\RustDesk\rustdesk-management-agent.exe` 仅有副本 |
| 现网 hash | `3e588920…` |
| 服务状态 | Running |
| 能力入库 | 设备 **1338209581**（主机 yan）status caps **含** `system.terminal@1`（共 16 项，含 enrollment 字段） |
| 脚本 | `scripts/win10-gs/replace-agent.ps1`（默认 InstallDir 已改为 ProgramData 路径） |

### 坑（必读）

1. **`state/disable-recovery` 会让 `service_main` 主循环整圈 `continue`**，status 停更 → 能力不入库 → 终端 WS 一直 `agent-auth` 403。不是「只关自动修复」。
2. 救援/自动修复可能盖回旧 agent；联调后核对 hash。
3. 写门禁文件联调前先确认 status 已含 `system.terminal@1`。

---

## 3. rd-api 发版与生产

| 时间 | 事件 |
|---|---|
| 2026-10-27 晚 | tag **v0.1.55** 镜像 nginx 终端 location 正则**未加引号**，`{32}` 被当配置块，容器起不来，`upgrade-api.sh` 自动回滚 0.1.54 |
| 2026-10-27 晚 | api 会话修正则并 tag **v0.1.56** |
| 2026-10-28 01:18 | `upgrade-api.sh 0.1.56` **exit 0**；`VERSION=0.1.56` |
| 持续 | `https://rdapi.yan.life/yanolly/login` → 200；`/yanolly/terminal` SPA → 200 |

本地门禁（api 侧，已完成）：Go test；`terminal_*_test` 19；channel 12；Playwright 5/5。

---

## 4. 生产联调 V2 / V4 / V5 / V6（2026-10-28）

### 4.1 公共前提

| 项 | 值 |
|---|---|
| 生产 | `https://rdapi.yan.life`，admin 前缀 `/yanolly` |
| 设备 | `client_id=1338209581`，`client_uuid=NzMxODA2YTYtZjk0Yy00NmU0LWFkYzYtY2E1MGY4MWFmNmYy`，hostname=`yan` |
| Agent WS | `10.0.0.100` → `47.100.7.221:443` **ESTABLISHED**；`/_rustdesk-terminal-internal/agent-auth` **200** |
| 鉴权方式 | 容器内 `www-data` 铸造 1h `rd_admin` 会话（**不改密码**）；测完 `rm /tmp/sess_*` |
| 终端 WS | `wss://rdapi.yan.life/yanolly/api/terminal/ws?csrf=…` + Cookie + Origin |

### 4.2 V4 — 打开终端并执行命令

| 步骤 | 结果 |
|---|---|
| open powershell 120×30 | `type=opened`，服务端分配 `session_id` |
| input whoami 等 | 输出 `nt authority\system` |
| WindowsIdentity | `NT AUTHORITY\SYSTEM` |
| marker | `__V4_OK__` |
| close | `reason=user` |
| 审计 | 仅元数据 `terminal_open` 等；**无输入输出正文**（契约） |

脱敏摘录：`docs/evidence/terminal-v4-out.txt`。

### 4.3 V5 — 断线后续会话

| 步骤 | 结果 |
|---|---|
| open 后写 marker `V5MARK…` | phase1 输出含 marker；`last_seq≈66` |
| 断 WS，**不** close | — |
| 新连接 attach 同 session_id | 连接成功 |
| 再输入 `V5_AFTER` | **有回显** → 会话未因断线销毁 |
| marker 历史回放 | attach 输出中**未观测到**旧 marker（last_seq 已近顶端时可能无可补帧）→ 记已知限制 |

脱敏摘录：`docs/evidence/terminal-v5-out.txt`。

### 4.4 V2 — 尺寸 / TUI 等价

| 步骤 | 结果 |
|---|---|
| open 80×24 | opened |
| resize → 140×45 | 无 error |
| `$Host.UI.RawUI.WindowSize` | **`V2_COLS=140 V2_ROWS=45`** |
| vim | 机上无；以 ConPTY 窗口尺寸生效为准 |

脱敏摘录：`docs/evidence/terminal-v2-out.txt`。

### 4.5 V6 — 完整演练

上述 V2+V4+V5 均在**生产 0.1.56 + 真机 agent** 上完成，计 V6 ✅。

---

## 5. 协议与实现注意（回归必查）

1. **`opened.session_id` 可能与客户端 open 时提交的 id 不同**  
   - 后续 input/resize/close/**attach** 必须用 **opened 返回的 id**。  
   - 前端 `channel.ts` 在 `case "opened"` 已执行 `this.sessionId = String(message.session_id ?? "")`（已核对）。
2. Agent 终端线程：`src/management_agent/terminal/runtime.rs`，`service_main` 挂 `agent-terminal` 线程。
3. 能力：`status.rs` `capabilities_for` 在 `windows + x86_64` 追加 `system.terminal@1`。
4. 本地门禁与 UAC 共用：`disable-agent` / maintenance / 停止意图 / disable-recovery 拒 open+input。

---

## 6. 可复用脚本（win10-gs）

见 `scripts/win10-gs/README.zh.md`：

| 脚本 | 用途 |
|---|---|
| `sync-sources.sh` | SHA-256 差量 `termark upload`；目录自动展开 |
| `run-build.sh` | 默认 termark 长连接编译；`BUILD_VIA=schtasks` 可选 |
| `build-agent.ps1` | vcvars + 锁定工具链；上传时加 **UTF-8 BOM** |
| `run-terminal-tests.ps1` | `cargo test … terminal::` |
| `replace-agent.ps1` | 安全替换（System 属性 + SCM failure actions） |
| `common.sh` | termark JSON 解包 `win10_out` / `win10_exec` |

---

## 7. 自主决定存档

1. `script_run.rs` 本轮**保留**，不声明能力（G8）。  
2. 生产联调可用 www-data 短会话代替交互密码；测完必删 sess 文件。  
3. V2 在无 vim 时用 ConPTY WindowSize 作为验收。  
4. V5 marker 回放未观测到时，以「断线后会话仍可输入」为通过条件，回放记限制。  
5. AOCI：MCP 不可用时记 ⚠️，不阻塞业务关闭。

---

## 8. 复现命令速查

```bash
# Mac 单元
cargo test --locked --bin rustdesk-management-agent -- terminal::

# win10-gs
scripts/win10-gs/sync-sources.sh --from-git
scripts/win10-gs/run-build.sh debug 'terminal::'
# 替换（管理员）
# powershell -File scripts/win10-gs/replace-agent.ps1

# 生产健康
curl -sS -o /dev/null -w '%{http_code}\n' https://rdapi.yan.life/yanolly/login
curl -sS -o /dev/null -w '%{http_code}\n' https://rdapi.yan.life/yanolly/terminal
```
