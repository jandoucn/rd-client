# SYSTEM 终端 · 使用说明与安全边界

面向运维管理员。协议细节见 `rd-client/docs/terminal-protocol.zh.md`；实现进度见 `docs/terminal-rd-api-progress.zh.md` 与 `rd-client/docs/terminal-tdd-ledger.zh.md`。

---

## 1. 这是什么

在管理后台用网页打开设备上的 **SYSTEM 身份**交互 shell（PowerShell / pwsh / cmd），效果接近本机「以 SYSTEM 开终端」。

- 入口：管理后台侧栏 **设备运维 → 终端**（路径 `/{管理前缀}/terminal`）
- 通道：独立 WebSocket，**不走** Agent 命令通道
- 身份：设备上以 `NT AUTHORITY\SYSTEM` 跑 shell

---

## 2. 谁能用

必须同时满足：

| 侧 | 条件 |
|---|---|
| 浏览器 | 已登录的**全局管理员**（`address_book_scope=all`） |
| 浏览器 | 有效 CSRF；同源请求（跨站 Origin 直接拒） |
| 设备 | 已开通 Management Agent 且绑定有效 |
| 设备 | 近期 status 里声明能力 `system.terminal@1` |
| 设备 | Agent 已连上终端 WS（页面会显示「终端通道未连接」否则） |

非全局管理员看不到可用入口；未声明能力的旧 Agent 在设备下拉里显示为不可用。

会话**只属于创建它的管理员**：别人既不能附着，也不能关。

---

## 3. 怎么用

1. 打开 **终端** 页，点 **刷新** 拉设备列表与已有会话。
2. 选设备、选 Shell，点 **打开终端**。
3. 在 xterm 里正常打字；工具栏支持：
   - **复制选中** / **粘贴**
   - **发送 Ctrl+C**（中断当前前台进程）
4. 关标签时二选一：
   - **结束会话**：终止设备上的 shell 及子进程树
   - **只关闭标签**：会话仍在设备上，本页「未在本页打开的会话」里可 **继续**
5. 游离会话也可在列表里直接 **结束**。

多标签可同时开多台/多个会话，互不抢输入。

窄屏（如 390px）：终端区域可横向滚动，页面本身不应被撑破。

---

## 4. 超时与上限

| 项 | 值 |
|---|---|
| 单管理员会话数 | 最多 8 |
| 单设备会话数 | 最多 4 |
| 空闲 | 30 分钟无输入/输出 → 自动结束（`idle`） |
| 最长存活 | 24 小时（`error`） |
| 握手 | 10 秒 |
| 单条 WS 消息 | 256 KB |
| 命令 `timeout_seconds` | **不约束**终端会话 |

断线：浏览器会自动重连并用 `last_seq` 向 broker 补齐环缓冲内的输出；离线过久可能丢一段并提示缺口。

---

## 5. 安全边界（必读）

### 5.1 授权例外

Management Agent 默认禁止「未批准脚本 / 任意路径 / 任意进程」。  
**SYSTEM 终端是唯一授权例外**：管理员可在已授权设备上交互执行会话内交互命令、使用任意工作目录。

例外**只**存在于：

- 浏览器 `/{admin}/api/terminal/ws`
- 设备 `/rd/update/v1/agent/terminal/ws`

`/rd/update/v1/agent/commands` 及其它 Agent 命令**仍然禁止**未列入能力清单的执行，不得借终端通道扩大命令通道能力。

### 5.2 本地门禁优先

设备上若存在任一情况，**拒绝** `open` 与 `input`，只允许 `attach`/`close` 清理：

- `disable-agent`
- 维护状态（`maintenance.json` 等）
- 用户主动停止意图
- （实现侧还可能含 disable-recovery）

页面会显示类似「设备本地已暂停远程维护」的说明；审计记 `terminal_gate_refused`。

### 5.3 审计写什么、不写什么

写入 `agent_operations_audit` 的只有：

- 会话创建（管理员、设备、shell、行列）
- 会话结束（原因、时长）
- 门禁拒绝（错误码）

**禁止**把终端输入或输出写入：审计表、请求日志、或任何持久化存储。

### 5.4 架构约束

- PHP-FPM 只做短鉴权与审计；**不**为终端会话占 worker
- IO 由 stream-broker 异步承载；broker **不**直接访问 SQLite
- 终端连接不得挤占更新策略 SSE 为签名设备保留的容量
- 公网不可达内部路径 `/_rustdesk-terminal-internal/*`（仅本机/内网回环）

### 5.5 运维注意

- 能打开终端 ≈ 该设备上的 SYSTEM 权限；只授给可信全局管理员
- 结束会话会杀整个 shell 进程树（含它拉起的子进程）
- 生产启用前确认：nginx 已把终端 WS/会话 API 转到 broker；镜像含本功能版本

---

## 6. 常见提示对照

| 界面/错误 | 含义 |
|---|---|
| Agent 未开通远程管理 | 先在客户端页绑定/授权 Agent |
| Agent 尚未上报状态 | 等 status 或检查 Agent 是否在跑 |
| 该设备的 Agent 版本不支持终端 | 升级带 `system.terminal@1` 的 Agent |
| Agent 离线 / 终端通道未连接 | Agent 未连上终端 WS 或 status 过旧 |
| 设备本地已暂停远程维护 | 本地门禁；在设备上解除维护/停用后再开 |
| 会话数已达上限 | 先结束不用的会话（8/4） |
| 设备上没有所选的 shell | 例如未装 pwsh，换 powershell/cmd |
| 设备连接已断开 | Agent WS 掉线，会话结束 |
| 连接中断，正在重连… | 浏览器侧重连；恢复后补齐缓冲内输出 |

---

## 7. 本轮范围外

- RustDesk 客户端内嵌 SYSTEM 终端窗口
- 文件上传/下载
- Agent 重启后跨进程恢复同一 PTY 会话（v2）
- 作业式 `system.run` 脚本（半成品另议）
