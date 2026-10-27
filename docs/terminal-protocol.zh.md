# SYSTEM 终端协议契约

状态：实现依据。Agent、后端、前端三方以本文件为准，任何一方不得单方面改字段。

## 0. 总体链路

```
浏览器 xterm.js
   ↕ WebSocket（管理员会话鉴权）
后台 broker（WS 中转，复用现有 stream-broker 进程）
   ↕ WebSocket（设备签名鉴权）
Agent（SYSTEM 服务）
   ↕ 命名管道（本机）
终端 helper 进程（ConPTY）
   ↕ ConPTY
powershell / pwsh / cmd
```

两段都用 WebSocket，理由：终端是高频双向交互，任何轮询方案都会引入打字延迟。broker 现有的 SSE 已经同时支持「管理员会话」与「设备签名」两种鉴权，WS 复用同一套。

## 1. 会话身份

- `session_id`：32 位小写十六进制，由 broker 生成，全局唯一。
- 一个会话 = 一个 helper 进程 = 一个 PTY = 一个 shell 进程树。
- 会话归属：`{admin_id, device_id, device_uuid}`；只有创建它的管理员能读写。
- 单管理员最多 8 个会话，单设备最多 4 个。

## 2. 浏览器 ↔ broker

**端点**：`GET /{admin_path}/api/terminal/ws`（WebSocket upgrade）

**鉴权**：握手时带管理员会话 Cookie 与 `?csrf=<token>`；任一缺失或不匹配则 `401` 并立即关闭。

**消息格式**：JSON 文本帧。所有服务端消息含 `seq`（从 1 递增，会话内单调，跨重连保持连续）。

### 客户端 → 服务端

| type | 字段 | 说明 |
|---|---|---|
| `open` | `device_id`, `device_uuid`, `shell`(`powershell`/`pwsh`/`cmd`), `cols`, `rows`, `cwd`? | 新建会话。`device_id` 与 `device_uuid` 必须同时给出。 |
| `attach` | `session_id`, `last_seq` | 重连已有会话，补发 `last_seq` 之后的输出。 |
| `input` | `session_id`, `data` | 键盘输入，base64 编码的原始字节（含控制字符）。 |
| `resize` | `session_id`, `cols`, `rows` | 窗口尺寸变化。 |
| `close` | `session_id` | 主动结束会话。 |
| `ping` | — | 心跳，服务端回 `pong`。 |

### 服务端 → 客户端

| type | 字段 | 说明 |
|---|---|---|
| `opened` | `seq`, `session_id`, `shell`, `cols`, `rows`, `started_at` | 会话已建立。 |
| `output` | `seq`, `session_id`, `data`, `stream`(`stdout`/`stderr`) | 终端输出，base64 原始字节（含 VT 转义序列）。 |
| `exited` | `seq`, `session_id`, `exit_code`, `duration_ms` | shell 已退出。 |
| `closed` | `seq`, `session_id`, `reason`(`user`/`idle`/`device_gone`/`error`) | 会话已结束。 |
| `error` | `seq`, `session_id`?, `code`, `message` | 错误。`code` 见第 5 节。 |
| `pong` | `seq` | 心跳回应。 |

**字节语义**：`output` 的 `data` 是**原始字节的 base64**，不是文本。前端解码后直接喂给 xterm.js，避免中文/二进制被 UTF-8 截断。分片边界可能落在一个多字节字符中间，前端必须用字节流缓冲，不能逐帧当字符串处理。

## 3. broker ↔ Agent

**端点**：`GET /rd/update/v1/agent/terminal/ws`（WebSocket upgrade）

**鉴权**：复用设备签名（与 `/rd/update/v1/agent/commands` 同一套 header：`X-RustDesk-Device-ID`、`-Public-Key`、`-Timestamp`、`-Nonce`、`-Signature`），并对查询参数 `client_id`、`client_uuid`、`agent_installation_id`、`enrollment_generation` 做与命令通道一致的校验。

**消息格式**：与浏览器侧同构，但由 broker 翻译：

| broker → agent | agent → broker |
|---|---|
| `open`（含 `session_id`、`shell`、`cols`、`rows`、`cwd`） | `opened` |
| `input`（`session_id`、`data`） | `output`（`session_id`、`data`、`stream`） |
| `resize` | `exited` |
| `close` | `closed` |
| `ping` | `error`、`pong` |

Agent 侧 `seq` 由 agent 为每个会话自行维护，broker 原样透传给浏览器（浏览器看到的 `seq` 即 agent 的 `seq`）。

## 4. 断点续传与背压

- 每个会话在 broker 与 agent 各保留一份**环形输出缓冲**：broker 2 MB、agent 4 MB，按字节。
- 缓冲按 `seq` 索引；`attach` 时若 `last_seq` 仍被保留，补发其后全部输出；若已被丢弃，返回 `error` 且 `code = terminal_gap_too_large`，前端提示「已丢失部分输出」并从当前 `seq` 继续。
- **背压**：浏览器侧 socket 发送缓冲超过 1 MB 时，broker 暂停向 agent 发送 `output` 请求；降回 256 KB 以下恢复。agent 侧同理。

## 5. 错误码

| code | 含义 |
|---|---|
| `terminal_auth_failed` | 握手鉴权失败 |
| `terminal_not_found` | 会话不存在或不属于当前管理员 |
| `terminal_session_limit` | 超出会话数上限 |
| `terminal_shell_unavailable` | 指定 shell 不可用（如 `pwsh` 未安装） |
| `terminal_start_failed` | PTY 或子进程创建失败 |
| `terminal_device_offline` | 设备不在线 |
| `terminal_local_gate` | 设备本地门禁拒绝（`disable-agent` / 维护态 / 停止意图） |
| `terminal_capability_missing` | 设备 Agent 未声明 `system.terminal@1` |
| `terminal_gap_too_large` | 缓冲已丢弃要求的续传点 |
| `terminal_internal` | 其他 |

## 6. 门禁与审计

- **本地门禁优先**（沿用现有规则）：`disable-agent`、`maintenance.json`、用户停止意图任一存在时，`open` 被拒（`terminal_local_gate`）；已存在的会话**允许 `attach` 与 `close`**，便于清理，但**拒绝 `input`**。
- 审计事件（写现有 `agent_operations_audit` 与客户端本地事件）：
  - 会话创建：谁、哪台设备、什么 shell、行列数
  - 会话结束：原因、时长
  - 被门禁拒绝：门禁名
- 不记录终端输入输出内容到审计表（内容可能含敏感信息）；只在 agent 本机保留会话缓冲，会话结束后删除。

## 7. 超时

| 项 | 值 | 说明 |
|---|---|---|
| 会话空闲 | 30 分钟 | 无输入且无输出则结束，`reason=idle` |
| 会话最长存活 | 24 小时 | 到时强制结束，避免长期残留 |
| 握手超时 | 10 秒 | 建连后 10 秒内未发 `open`/`attach` 则断开 |
| 心跳 | 25 秒 | 任一侧 25 秒未收到任何帧则发 `ping`；60 秒无响应断开 |
| 单条消息上限 | 256 KB | 超过则断开并记 `terminal_internal` |

**终端会话不受现有命令 `timeout_seconds` 上限（900 秒）约束**：它是长连接，不是一次性命令。

## 8. 能力声明

Agent 在 status 的 `capabilities` 中新增 `system.terminal@1`。后台对未声明的设备不显示终端入口，并给出 `terminal_capability_missing` 原因。

## 9. 与现有部分的关系

| 现有 | 关系 |
|---|---|
| `/rd/update/v1/agent/commands` | 不用于终端；终端走独立 WS，避免污染命令状态机与审计语义 |
| stream-broker 的 SSE | 复用其鉴权与连接管理代码，不共用连接 |
| `portable-pty` fork | 直接使用，不新增 PTY 依赖 |
| `parent_handoff` / `helper_runtime` | 复用其 helper 拉起与校验模式 |
| RustDesk 客户端终端 | 本轮不改；下一轮在其上增加 SYSTEM 模式 |

## 10. 实现澄清补丁 v1（2026-10-27，rd-client 提出，待 rd-api 核）

本节只填补 §1–§9 的空白、不改已定字段。第 5、7 条含设计取舍，rd-api 有异议请回。

1. 错误码以 §5 为准（全部 `terminal_` 前缀）。§4、§8 行文中的简写已就地改为 §5 值；wire 上的 `code` 字段一律用 §5 值。
2. `seq` 归属：`seq` 是会话内、由 agent 拥有、从 1 单调递增，只打在 **agent 来源**且属于有序输出流的帧上（`opened`、`output`、`exited`）。broker 自己产生的控制/传输帧（`pong`、`opened` 之前或 `device_gone` 等由 broker 发的 `error`/`closed`）一律 `seq=0`，不属于可续传流、不参与 `attach` 的 `last_seq` 比对。§2「所有服务端消息含 seq」据此理解：`seq=0` 表示“非有序流控制帧”。
3. 背压：不新增流控帧，靠 TCP 背压 + 两级环形缓冲。broker 浏览器侧发送缓冲 >1MB 时，broker 停止从 agent WS 读取（不再 drain），经 TCP 流控回压到 agent，agent 的 OS socket 写阻塞时输出暂存进其 4MB 环形缓冲；浏览器侧降到 256KB 以下恢复读取。§4「暂停向 agent 发送 output 请求」更正为此（output 是 agent 推送，无请求）。
4. 单条读上限：broker/前端从环形缓冲取 `attach` 回补时，单条消息不超过 §7 的 256KB；缓冲按 §4 大小保留，续传分多条发，`seq` 连续。
5.【提案待核】agent 连接模型：agent 对 broker 保持**一条**出站终端 WS（每设备一条，多会话按 `session_id` 复用），在声明 `system.terminal@1` 后、由现有策略流（policy SSE）通知“有待处理 open”时按需拨号建立；broker 不主动连 agent。`terminal_device_offline` = 该终端 WS 当前未连接（不按心跳 20s 判定）。终端 WS 的会话配额（单管理员 8 / 单设备 4）由 broker 独立计数，**不**占用策略流为签名设备保留的 4096/每设备 4 容量。rd-api 若认为“常驻一条 WS”或“命令通道通知”更合适，请回。
6. 设备签名（broker↔agent 的 GET upgrade）复用命令通道串，精确拼接为：
   `rustdesk-update-auth-v1
method=GET
path=/rd/update/v1/agent/terminal/ws
client_id={id}
client_uuid={uuid}
timestamp={ts}
nonce={nonce}
command_id=
body_sha256={sha256("")的 hex}
`
   —— `command_id` 为空串，`body` 为空（`body_sha256` = 空串的 sha256 hex）；Ed25519 `sign_detached` 后 base64 放 `X-RustDesk-Device-Signature`，其余 `X-RustDesk-Device-ID/-Public-Key/-Timestamp/-Nonce` 及 query `client_id/client_uuid/agent_installation_id/enrollment_generation` 校验与命令通道一致。method 取大写。
7.【提案待核】审计与会话列表的内部端点（均 loopback，仅本机）：
   - broker → PHP：`POST {内部}/terminal/audit`，由 PHP 写 `agent_operations_audit`（创建/结束/门禁拒绝），broker 不直接碰 SQLite。
   - PHP → broker：`GET {内部}/terminal/sessions?admin_id=`、`POST {内部}/terminal/close`，供 PHP 代理管理员的会话列表/关闭（会话状态只在 broker 内存）。
   具体内部路径/端口由 rd-api 定，定后回填本节。
8. CSRF 与跨站防护：`?csrf=<token>` 保留，但该 WS location 的 nginx access log 必须不记 `$args`（避免 token 落日志）；并**强制校验 Origin 同源**，防跨站 WS 劫持。§2 鉴权三要素更正为：管理员会话 Cookie + CSRF + 同源 Origin，缺一即 `401` 关闭。


### §10 定稿（2026-10-27，rd-api 已核）

- 第5条【定】Agent 连接模型 = 常驻出站 WS：Agent 声明 `system.terminal@1` 且绑定有效时常驻一条出站终端 WS（每设备一条，多会话按 `session_id` 复用），断开指数退避重连（1s 起、上限 60s、抖动）；同一设备新连接替换旧连接（旧的由 broker 关闭）。`terminal_device_offline` = 该 WS 当前未连接。broker 对 Agent 终端连接单独计数（全局 2048），不占策略流 4096/每设备 4。心跳按 §7：broker 每 25s 发 WS ping，60s 无帧断开；Agent 可发 `{"type":"ping"}`，broker 回 `{"type":"pong","seq":0}`。（更正 v1 提案「经策略流通知按需拨号」——Agent 不持有策略 SSE，故改为常驻。）
- v1 不做断线续会话：Agent 断开时 broker 立即对该设备全部会话向浏览器发 `closed(reason=device_gone, seq=0)` 并结束审计；Agent 每次(重)连成功后结束本地仍存活的旧会话、全新开始。T1.6/V3「Agent 重启后会话可重连」需新增 broker→agent `attach{session_id,last_seq}`，属改契约，**留 v2**。
- 第7条【定】端点（不做 PHP→broker 调用；会话状态只在 broker）：
  - 公网经 nginx→broker(127.0.0.1:8787)：浏览器 WS `GET /{admin_path}/api/terminal/ws?csrf=`；会话列表 `GET /{admin_path}/api/terminal/sessions?csrf=`；关闭 `POST /{admin_path}/api/terminal/sessions/{session_id}/close`（头 `X-CSRF-Token`）；Agent WS `GET /rd/update/v1/agent/terminal/ws?client_id&client_uuid&agent_installation_id&enrollment_generation`。
  - 公网 PHP：`GET /{admin_path}/api/terminal/devices`（声明 `system.terminal@1` 的设备+在线+绑定）。
  - 内部 loopback（broker→PHP，容器内 127.0.0.1:8081，公网同路径固定 404）：`GET /_rustdesk-terminal-internal/admin-auth`（Cookie+CSRF+scope=all → admin_id/username）；`GET /_rustdesk-terminal-internal/agent-auth`（设备签名+nonce+精确身份+绑定+最新 status 含 `system.terminal@1` → 身份，失败 401/403 不升级）；`POST /_rustdesk-terminal-internal/audit`（写 `agent_operations_audit`，action ∈ `terminal_open`/`terminal_close`/`terminal_gate_refused`/`terminal_open_failed`，不含输入输出内容）。
- 第2条【定】agent 来源 `closed` 的 seq 由 agent 决定（0 亦可）；broker 只把 `seq>0` 的帧放进 2MB 环。opened=seq1、output 依次递增、exited=末位 seq；pong 及 agent 偶发 error/closed 用 seq=0。
- 第8条【定】该 WS location 的 nginx access log 不记查询串；broker 强制 Origin 与 Host 同源。
