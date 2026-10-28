# 桌面端管理员终端与 SYSTEM 会话（私人定制）

> 状态：实现已合入工作区；跨机真机验收 **2026-10-28 通过**（见 §7）。  
> 与 **Management Agent 网页 SYSTEM 终端**（`docs/terminal-protocol.zh.md`）是两条独立通道：本文件只描述 **RustDesk 桌面 peer 菜单里的终端连接**。

## 1. 产品定位

本私人客户端默认：

- 固定密码与一次性密码同时启用；
- 远程权限默认全开；
- 安装版 Windows 服务以 **LocalSystem** 常驻。

在此前提下，运维需要两种桌面终端入口：

| 菜单 | 会话身份（已安装 Windows 服务） | 是否再收集本机 Windows 交互登录凭据 |
|---|---|---|
| 终端 | 当前控制台用户（或既有用户令牌路径） | 否（普通终端路径） |
| 终端（以管理员身份运行） | **服务进程身份 = LocalSystem** | **否**（本定制已取消） |
| 终端（SYSTEM） | 与上一行相同后端 | **否**（新增入口，同一后端） |

认证边界：

1. **连接认证**：RustDesk 固定密码或一次性密码（与「控制设备 / 文件传输」同一套 `PeerConfig`、地址簿、默认连接密码与「记住密码」）。
2. **会话身份**：已安装服务下，管理员 / SYSTEM 终端使用 **被控端服务进程令牌**，不再调用本机交互式 `LogonUser` 收集管理员账号。

这样运维只需维护连接密码策略，不必再在每台被控机上准备可交互的本地/域管理员口令；也避免 Win11 微软账户（online）无法走旧 `LogonUser` 校验的问题。

## 2. 用户可见行为

### 2.1 打开方式

- Peer 卡片菜单（Windows 被控）：「终端（以管理员身份运行）」「终端（SYSTEM）」  
- 连接页快捷菜单（桌面）：同上  
- URI / 命令行：`--terminal-admin`、`--terminal-system`（均设置 `IS_TERMINAL_ADMIN=Y`）

「终端（SYSTEM）」为产品命名入口；实现上与管理员终端共用 `setEnvTerminalAdmin()` 与同一登录路径。正式安装包需带本轮 **Flutter + 主库** 才会显示 SYSTEM 菜单项；仅替换 `librustdesk.dll` 时仍可用「以管理员身份运行」。

### 2.2 密码与记住

| 场景 | 期望 |
|---|---|
| 首次连某设备且未记住连接密码 | 只出现与控屏相同的 **连接密码** 框；可勾选记住 |
| 已记住固定密码 / 地址簿密码 / 默认连接密码 | 直接进入终端，不再弹框 |
| 一次性密码 | 与控屏相同：按会话策略使用；不因终端类型额外要 OS 口令 |
| 本机连本机 | 常不走完整远程连接密码流程，**不能**当作「免连接密码」或「免 OS 口令」的验收依据 |

### 2.3 进终端后的身份

已安装 Windows 服务时，管理员 / SYSTEM 终端内：

```text
whoami
→ nt authority\system
```

工作目录常见为 `C:\Windows\system32\config\systemprofile`（或等价 SystemProfile 路径）。

普通「终端」入口保持用户会话身份，行为与改前一致。

## 3. 设计原因（决策记录）

| 日期 | 决定 |
|---|---|
| 2026-10-27 | 管理员终端取消本机 Windows 账号口令收集；只保留连接认证 |
| 2026-10-28 | 新增「终端（SYSTEM）」菜单/URI；连接密码固定/一次性均可，记住逻辑与控屏一致 |
| 2026-10-28 | 验收必须 **跨机**；本机连本机不作连接密码结论 |

技术背景（简述）：

- 上游「以管理员运行」路径会向被控端提交 OS 用户名/密码，被控端 `LogonUser` 后仍可能落到服务上下文；对私人机群而言，这道 OS 口令与连接密码重复，且与微软账户环境不兼容。
- 本定制改为：控制端在管理员 / SYSTEM 模式下提交约定 **会话标记**（非真实 Windows 账户名）与空 OS 密码字段；被控端在 OS 用户名字段非空时选用 **当前服务进程令牌（SelfUser）**，由已安装服务的 LocalSystem 身份承载 PTY。

## 4. 实现落点

| 层 | 位置 | 职责 |
|---|---|---|
| 策略与判定 | `src/uac_policy.rs` | `TERMINAL_ADMIN_OS_MARKER`；`terminal_login_os_credentials`；`terminal_admin_after_hash`（仅 `input-password` 或自动 marker 登录）；旧被控提示文案 |
| 控制端登录 | `src/client.rs` `handle_hash` / `handle_login_from_ui` | 管理员终端不走 `terminal-admin-login*`；连接密码来源与记住与普通连接相同 |
| 被控端令牌 | `src/server/connection.rs` `fill_terminal_user_token` | 用户名字段非空 → `TerminalUserToken::SelfUser`；空用户名仍走原用户令牌路径 |
| Flutter 菜单 | `flutter/lib/common/widgets/peer_card.dart`、`connection_page.dart`、`common.dart`、`mobile/pages/home_page.dart` | SYSTEM 入口；URI |
| Flutter 兼容 | `flutter/lib/models/model.dart` | 若仍收到旧 `terminal-admin-login*`，改为自动登录或只要连接密码 |
| 文案 | `src/lang/*.rs` 键 `Terminal (SYSTEM)` | 中文「终端（SYSTEM）」 |

**生效范围**：控制端与被控端的 **主程序**（Windows 上为 `librustdesk.dll` + 对应 Flutter 资源）。只替换 Management Agent **不会**改变桌面终端行为。

**已知限制**：

- 旧被控端仍按真实账户校验标记时，控制端提示升级（`LEGACY_TERMINAL_PEER_HINT`），不退回 OS 账号对话框。
- 本机热替换 Mac `liblibrustdesk.dylib` 与正式签名包 ABI/依赖不一致会导致应用无法启动；Mac 控制端须走完整 Flutter Tag Build，禁止再用 ad-hoc 抽换 dylib 做验收。
- 「终端（SYSTEM）」菜单依赖 Flutter 资源；仅换 DLL 时用「以管理员身份运行」即可覆盖同一后端。

## 5. 与网页 Agent SYSTEM 终端的关系

| | 桌面管理员 / SYSTEM 终端（本文） | Agent 网页 SYSTEM 终端 |
|---|---|---|
| 通道 | 既有远控连接 + 终端会话 | 独立 WebSocket（`docs/terminal-protocol.zh.md`） |
| 入口 | 客户端 peer 菜单 | 管理后台浏览器 |
| 认证 | 连接密码 + 终端权限 | 全局管理员会话 + CSRF + 设备签名等 |
| 身份 | 已安装服务 LocalSystem | Agent 侧 PTY（SYSTEM） |
| 审计 | 连接/会话既有路径 | `agent_operations_audit`（会话起止，无 IO 内容） |

二者都服务于私人机群维护，协议与权限模型独立，不可混用字段。

## 6. 安全与运维边界（产品语义）

- 能建立连接并具备终端权限的操作者，在已安装 Windows 被控上可打开 **LocalSystem** 交互 shell；这是本定制的明确产品能力，与默认全开远程权限、固定/一次性连接密码策略一致。
- 连接密码仍是门禁：未配置密码、密码错误、权限关闭时不得进入。
- 不引入任意脚本下发通道；桌面终端 IO 走既有终端会话，不写入 Management Agent 命令通道。
- 本地门禁（停服务、维护态等）对 Agent 命令的约束见 UAC/Agent 文档；桌面终端受连接与权限开关约束。

## 7. 真机验收记录（2026-10-28）

### 7.1 部署

| 机器 | 角色 | 动作 | 主库 sha256（`librustdesk.dll`） |
|---|---|---|---|
| win10-gs（`10.0.0.100`） | 控制端 + 曾作被控 | 本机 `cargo build --locked --lib --features flutter,hwcodec,vram --release` 后替换 `C:\Program Files\RustDesk\librustdesk.dll`；服务 Running | `35cec6e31840ff6a39ce0f7ed25698a84e66a5674f657c4a6c6cfe613b2bc229` |
| GepShare（`10.0.0.254`，设备 ID **`83077683`**） | 被控端 | 同上 DLL 经 termark 上传并替换；备份 `D:\tmp\rdtest\main-backup\`；服务 Running | 与上相同 |
| Mac「RustDesk Yan」 | — | 曾热换 dylib 导致无法启动；**已恢复原库**；不作为本轮控制端结论 | — |

脚本参考：`scripts/win10-gs/replace-main-dll.ps1`、`build-main-lib.ps1`（win10-gs 侧）。

### 7.2 无效用例（明确排除）

| 操作 | 结果 | 结论 |
|---|---|---|
| win10-gs 连接 **本机** 管理员终端 | 可进；`whoami` = `nt authority\system` | 仅证明本机会话身份；**本机连接常跳过完整连接密码流程，不作免 OS 口令/连接密码验收** |

### 7.3 有效用例（跨机）

| 项 | 内容 |
|---|---|
| 时间 | 2026-10-28 晚（北京时间） |
| 控制端 | win10-gs 已装主库（上表 hash） |
| 被控端 | GepShare `83077683`，主库同 hash |
| 操作 | 控制端打开「终端（以管理员身份运行）」连 GepShare |
| OS 交互登录框 | **未出现** |
| 连接密码框 | 本次未再输入（控制端侧该 peer 很可能已记住连接密码，与控屏共用记住逻辑；**符合「记住后不再问」**；若需单独验证「未记住时仍要连接密码」，可清除该 peer 已存密码后重测，本轮用户确认可不测） |
| 终端内命令 | `whoami` → **`nt authority\system`** |
| 提示符路径 | `PS C:\Windows\system32\config\systemprofile>` |
| 结论 | **跨机管理员终端：会话身份 LocalSystem；不再收集 Windows 管理员口令。验收通过。** |

### 7.4 单元与库内自检（同日）

- Mac：`cargo test --locked -p rustdesk --lib uac_policy` → **15 passed**（含 `admin_terminal_after_hash_never_opens_an_os_account_dialog` 等）。
- 正式发布仍需：Flutter Tag Build 产出完整 MSI/DMG，使 SYSTEM 菜单与签名包一致上线。

## 8. 发布与回归清单

发版前建议核对：

1. 控制端 + 被控端均为含本改动的主程序包（或已验证 hash 的 `librustdesk.dll` + 匹配 Flutter）。
2. 跨机：管理员终端无 Windows 账号框；未记住时出现连接密码框；记住后可直达。
3. 进终端 `whoami` 为 `nt authority\system`（已安装服务）。
4. 普通「终端」仍为用户身份。
5. 旧被控端：提示升级，不出现 OS 账号框死循环。
6. 网页 Agent SYSTEM 终端回归见 `docs/terminal-tdd-ledger.zh.md`（本能力不替代那条链路）。
