# 物理机自动升级后的 Agent 实测记录与完善需求

整理日期：2026-10-16。文中服务器记录时间均为北京时间。

本文用于交给后续 AI 分析和完善实现，不代表新增功能已完成或所有恢复能力已验收。本次只新增记录文件，不修改客户端、agent 或 API 代码。

## 一、用户的目标

机器分散在异地，不能因为 RustDesk 主程序意外掉线就只能到现场维修。希望 management agent 成为独立、可观测、可远程管理的兜底通道，并具备恢复自身的后备入口。

用户接受修改、升级甚至较大调整 agent，但要求设计细致、实现完整。后续尽量通过 API 的策略、任务编排和批准包管理扩展运维流程，避免每次都重新发布客户端。

期望覆盖：

- 主程序文件存在但服务停止：能识别、拉起，并确认实际就绪和注册。
- 服务丢失、配置异常或程序损坏：能在固定 RustDesk 对象范围内修复。
- 主目录客户端不可用：有经过验证的 fallback，而不是仅判断某个 EXE 是否存在。
- 主程序文件丢失：能通过批准包完成恢复，保留设备身份及既有配置。
- 主程序离线、agent 在线：API 仍可检查、诊断、下发固定服务维护、重装、更新等任务，并取得真实回执。
- 主服务与 agent 同时停止：有独立 bootstrap 等入口负责恢复 agent，再由 agent 恢复主程序。
- 网络不可用或在线下载失败：有经过签名和完整性验证的离线恢复资源，或明确报告无法恢复的原因。
- 安装失败、进程中断、断电和重启：能接续事务或回滚，不能仅留下“正在安装”。

全部运维权限仍受本地停止意图、禁用标记、维护、互斥和校验约束。这里只要求固定 RustDesk 对象和批准发布包的完整运维能力，不授权任意脚本、任意路径、任意进程或防护绕过。

## 二、机器身份必须分清

本次用户贴出的物理机命令输出属于 **Tiaoma，ID `510206970`**，当前安装类型是 **SOS / MSI**。不能将其当作 ZYR 或 Bbraun 的本机输出。

| 机器 | ID | 本轮证据来源 |
| --- | --- | --- |
| Tiaoma | `510206970` | 用户提供的本机服务查询和 `status --json` |
| ZYR | `497695003` | 2026-10-16 生产库只读检查、线上权限判定函数结果 |
| Bbraun | `193002918` | 用户更新日志、生产更新事件、主程序和 agent 上报 |

不同机器的授权、安装路径、agent 实例、包类型和救援资源不能互相代替证明。

## 三、用户观察到的自动升级

用户在 rustdesk-api 后台开启自动更新、定时检查更新，随后观察到物理机主程序和 agent 升级至：

```text
版本：1.5.3
构建号：20261015.1
build_seq：2026101501
```

这证明用户实际运行到了新版本，而不只是安装目录里出现了新文件。具体是哪一次定时检查、哪个安装进程和哪个事务完成升级，仍需用对应设备的事件和安装日志关联，不能仅根据先后顺序断言。

版本字符串仍是 `1.5.3`，本次是否升级要看 build_seq，不能只比较显示版本。

## 四、Tiaoma 本机服务实测

用户在物理机 PowerShell 执行后得到：

```text
Name      : RustDeskManagementAgent
State     : Running
StartMode : Auto
StartName : LocalSystem
PathName  : C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe --service --edition sos --package-kind msi --normal-install-path "C:\Program Files\RustDesk"

Name      : RustDesk
State     : Running
StartMode : Auto
StartName : LocalSystem
PathName  : "C:\Program Files\RustDesk\RustDesk.exe" --service
```

此前 `sc.exe query` 显示两者均为 `4 RUNNING`，服务退出码均为 `0`；`sc.exe qc` 显示两者均为 `2 AUTO_START`、`LocalSystem`。

这些结果确认当前服务运行、启动类型、账号和参数符合该机器的 SOS MSI 安装身份。它们不能单独证明进程所有访问权限、服务器全部授权、Defender 排除或灾难恢复都正常。

### PowerShell 使用问题

- PowerShell 中 `sc` 是 `Set-Content` 别名，查服务必须写 `sc.exe`。
- 不要复制 `PS C:\Users\print>` 提示符，也不要将 JSON 输出再次作为命令执行。
- 管理员 PowerShell 的执行身份不等于后台 LocalSystem 服务身份。CLI 读取设备凭据失败时，不能直接断言后台服务也失败。

## 五、Tiaoma 本机 Agent 状态实测

用户实际执行：

```powershell
& 'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe' status --json
```

核心结果如下，来自用户提供的同一份 JSON：

| 字段 | 实测值 |
| --- | --- |
| `client_id` | `510206970` |
| `agent_installation_id` | `9a242209f126ea6d099a7adbfee4fc4c` |
| `observation_source` | `local_cli` |
| `session_generation` | `1` |
| `agent.version` | `1.5.3` |
| `agent.build_seq` | `2026101501` |
| `agent.executor_version` | `1` |
| `application.version` | `1.5.3` |
| `application.build_seq` | `2026101501` |
| `application.state` | `ready` |
| `application.registered` | `true` |
| `application.probe_supported` | `true` |
| `service.state` | `running` |
| `service.start_type` | `AutoStart` |
| `service.account` | `LocalSystem` |
| `service.trusted_path` | `true` |
| `service.binpath` | `"C:\Program Files\RustDesk\RustDesk.exe" --service` |
| `files.normal_present` | `true` |
| `files.fallback_present` | `false` |
| `package_identity` | `x86_64 / sos / msi` |
| `intent.state` | `enabled` |
| `recovery.effective_enabled` | `true` |
| `recovery.applied_policy_revision` | `1` |
| `operation.state` | `idle` |
| `operation.command_id` / `transaction_id` | `null` / `null` |
| `service_recovery_error` | 空字符串 |
| `last_service_recovery_at` | `null` |

声明的能力共 12 项：

```text
agent.status@1
service.inspect@1
service.start@1
service.restart@1
service.repair@1
recovery.reconcile@1
package.verify@1
diagnostics.collect@1
network.probe@1
package.reinstall@1
package.update@1
agent.update@1
```

### 已确认与尚未确认

已确认：主程序和 agent 都是新构建；服务运行；主程序就绪并已注册；本地恢复启用；当前空闲；没有记录服务恢复错误。

尚未确认：12 项能力在这台机器全部实际执行成功、API 当前实例绑定和全部授权、bootstrap 完整可用、离线恢复资源完整、真实失败回滚与断电恢复。

`last_service_recovery_at=null` 只表示没有记录该时间，不能当作恢复成功或失败的证据。`fallback_present=false` 只表示该文件不存在，不能单独证明恢复链失效；但也不能声称已准备好一份可立即启动的 fallback。

## 六、核心缺口：完整救援尚未就绪

Tiaoma 的实际结果：

```json
{
  "state": "unavailable",
  "ready": false,
  "verification_supported": true,
  "full_recovery_supported": false,
  "freshness": "unverified",
  "error_code": "waiting_approval",
  "blocked_reason": "full_recovery_not_confirmed",
  "checked_at": null,
  "source_reference": null,
  "source_digest": null,
  "verification_interval_seconds": 86400,
  "evidence": "cached_full_verification",
  "current_integrity": "not_rechecked_by_status"
}
```

组件情况：

| 组件 | 上报状态 | ready |
| --- | --- | --- |
| 客户端恢复资源 `client` | `unknown`，声明支持 | false |
| 离线包 `offline_package` | `unknown`，声明支持 | false |
| 凭据备份 `credentials` | `unknown`，声明支持 | false |
| agent 备份 `agent_backup` | `unknown`，声明支持 | false |
| bootstrap 备份 `bootstrap_backup` | `unknown`，声明支持 | false |
| 服务恢复 `service_recovery` | `not_assessed` | false |
| 在线来源 `online_source` | `not_assessed` | false |
| 恢复入口 `recovery_entry` | `not_assessed` | false |

这不等于 Windows 权限不足，也不等于管理员没给 agent 远程授权。当前本地服务恢复有效和完整救援未就绪可以同时成立。

源码线索：

- `src/management_agent/rescue_first_install.rs`：首次准备读取 `release-cache/first-install.json`；该文件缺失时返回 `waiting_approval`。
- `src/management_agent/rescue_status.rs`：在救援包缺失且首次安装状态记录为 `waiting_approval` 时投影该提示；当前投影明确不承诺完整恢复就绪。
- `src/management_agent/status.rs`：健康、能力、恢复门禁及 rescue 状态组合上报。

上述是当前工作区检查结果。后续 AI 必须核对 **发布构建 `2026101501` 对应源码**、当前工作区与实际安装包的差异，不要直接把当前源码行号或最新未发布能力套到现场版本。

## 七、另外两台机器的交叉证据

### ZYR：权限具备，运行在恢复目录

2026-10-16 13:35～13:38 生产只读检查：

- 主程序和 agent 都为 `2026101501`，主程序在线，agent 有新状态上报。
- 实例绑定有效、公钥指纹匹配，授权未撤销。
- 7 项服务端权限均已授予：`observe`、`service`、`install`、`agent_update`、`enable`、`batch`、`rebind`。
- 服务运行，自动启动，LocalSystem，主程序 `ready` 且 `registered=true`。
- `normal_present=false`，`fallback_present=true`，服务运行于 `C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe`。
- 线上权限投影允许服务维护、诊断、校验、网络探测；批准包查询允许重装、主程序更新和 agent 更新。
- 当前 agent 未声明 `client.enable@1`，该命令被判定为 `capability_missing`，即使服务器授予 `enable` 权限也不能执行。
- 当时没有 agent 命令或执行事件记录，不能把“命令可创建”当成“实际执行验收通过”。
- 同样存在 `rescue.waiting_approval`，完整救援未就绪。

#### 后续补充：ZYR 物理机本地查询

用户随后提供同一 ID `497695003` 的物理机输出，未提供精确采集时间。它是另一份观测，不能用此前服务器快照覆盖：

| 检查项 | 现场结果 |
| --- | --- |
| agent 服务 | Running / Auto / LocalSystem |
| agent 服务参数 | `--service --edition standard --package-kind msi --normal-install-path "C:\Program Files\RustDesk"` |
| 主服务 | Running / Auto / LocalSystem |
| 主服务实际路径 | `"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe" --service` |
| Agent 版本/构建 | `1.5.3 / 2026101501` |
| 主程序构建号 | 空，未取得 |
| 主程序健康状态 | `unknown` |
| 主程序已注册 | 空，未取得；不能解释为 `false` |
| 服务路径可信 | True |
| 主目录程序存在 | False |
| 恢复目录程序存在 | True |
| 本地启用状态 | enabled |
| 服务恢复已启用 | True |
| 当前任务状态 | idle |
| 服务恢复错误 | 空 |
| 完整救援已就绪 | False |
| 救援资源提示 | `bundle_missing` |
| 救援核验状态 | `unverified` |
| 能力声明 | 同前述 12 项，无 `client.enable@1` |

本次只能确认 SCM 主服务在运行且路径可信，不能确认当前应用健康或注册成功。主程序构建号和注册字段为空，很可能随健康探测没有取得结果而缺失，不能据此说主程序未安装或版本为零。

排查应区分实际健康变化、探测暂时失败、CLI 与 LocalSystem 服务的调用上下文差异，以及安装版本与探测协议匹配情况。必须取同一时段的服务端 agent 状态、主程序心跳、本地原始 JSON 和错误日志对照，不能直接认定主程序掉线或权限不足。

`bundle_missing` 与此前服务器的 `waiting_approval` 都没有救援就绪证明，但不是同一错误。应核对同一时刻两个执行上下文看到的救援目录和首次安装状态记录，判断资源是否缺失、初始化是否没有完成或状态投影是否不同，不能无证据声称资源后来被删除。

后续验收新增一项：**当主服务已切换到 fallback 路径且主目录缺失时，本机 CLI 与后台健康检查都必须有清晰、可核对的结果；探测失败不能伪报 ready，也不能触发无依据的重装。** 服务参数里的 normal-install-path 仍指向 Program Files 是正常目录定位信息，不单凭与当前 fallback binpath 不同就认定配置错误。

### Bbraun：升级已落地，成功终态缺失

对应 ID `193002918`，用户贴出的日志跨两天：

| 北京时间 | 事实 |
| --- | --- |
| 10-15 18:05:28～18:05:29 | `check` 命令 accepted → started → completed；只证明检查完成 |
| 10-16 13:33:33 | 普通更新事件 started |
| 10-16 13:34:36 | failed，`all_sources_failed` |
| 10-16 13:34:36 | 随后又出现 started |
| 10-16 13:35:02 | downloaded，来源 primary；随后 installing |
| 10-16 13:42:16 | 新 agent 上报自身和应用都是 `2026101501`，应用 ready、registered |

查询时事件表没有对应 installed 成功终态，而新版本已实际运行。两次下载尝试的事务关联仍需核查，不能无证据断言是哪种自动重试机制。

`all_sources_failed` 是获取并验证更新包流程的概括性错误，不能只凭这个码断定网络故障、权限不足、Defender 拦截或具体镜像失败原因。

## 八、交给后续 AI 的完善要求

### 1. 从真实发布包补齐救援初始化

核查普通安装和在线升级完成后，批准的包元数据、安装身份、离线包、客户端恢复资料、agent 和 bootstrap 备份是否都应自动准备。

明确 `first-install.json` 的合法生成者、签名来源、事务时序、持久化位置和生命周期。调查为什么真实在线升级后仍是 `waiting_approval`。不要靠手工伪造批准文件或直接把 ready 改成 true 消除提示。

升级后的资源必须对应同一 edition、架构、包类型、安装实例和可验证版本；旧资源不能冒充新资源。初始化失败必须保留当前可用主程序和 agent，给出可行动的诊断。

### 2. 区分各级健康与可恢复性

分别展示：服务运行、应用就绪、ID 已注册、主程序心跳、agent 鉴权、绑定有效、远程能力授权、包可获取、回滚资源可用、离线救援和 bootstrap 就绪。

不能用 HTTP 200、LocalSystem、文件存在、能力声明或一次升级成功代替全部验收。`verification_supported=true` 与 `ready=true` 必须严格区分。

### 3. 修复阶梯与 fallback 应可验证

对服务停止、服务缺失、服务路径错误、主程序损坏、主目录缺失、主目录及 fallback 双缺失逐项验证恢复路径。安装包不能只包含 RustDesk.exe，还要验证它实际依赖的运行文件。

服务 Running 之后还要验证真实应用健康及注册；注册失败要区分网络异常和进程故障，不能反复重装或破坏身份。

服务 Disabled、本地主动停止、维护、禁用标记、会话活动及安装互斥必须按项目合同处理。不能通过删除这些门禁“修复”故障。

### 4. Agent 自身也需要兜底

确认 bootstrap/任务确实注册且可运行，代理备份可验证，双服务停止、agent 文件缺失、机器重启、周期恢复均有真实测试。代理完全停止时仍应有独立入口；若文件被杀毒隔离或策略持续拒绝执行，应明确报告能力边界。

不得终止防护软件或规避其自保护。Defender 排除项及 360 信任区是单独配置，不证明恢复资源准备完成。

### 5. 远程运维完整，但执行对象保持固定

服务端权限、客户端能力、当前实例和本地执行门禁应形成一致判断。重点验证现有 12 项声明能力，梳理 `client.enable` 等服务端有类型而客户端未实现的差距，明确是否支持、如何呈现。

预留稳定的类型化 JSON 协议、能力版本协商、幂等命令、状态查询、结果补报和任务编排。只改 API 可以组合已有原语或调整批准策略，但不能声称能凭新 JSON 让旧执行器获得尚未实现的能力。

不引入任意 cmd/PowerShell、任意进程或任意路径执行。默认授权已实现的固定运维能力，同时遵守审计及本地约束。

### 6. 在线更新必须闭环

追踪 ordinary updater 与 agent 安装事务分别如何保留成功收据、恢复网络上报、确认新版本已运行及上报最终事件。处理 Bbraun 已升级但界面停在 installing 的情况。

区分“检查命令完成”“下载完成”“安装器退出”“主程序就绪”“agent 回来”“回滚完成”；不能把 check completed 当作安装成功。保留首次失败和后续成功两种真实事实，不删除失败记录。

### 7. 现场验收要验证实际结果

在隔离 VM 或明确授权的测试机使用真实发布安装包，至少覆盖：

1. 旧构建升级到 `2026101501` 或后续修复构建，主程序/agent/bootstrap 版本一致，ID、UUID、密钥和配置保留。
2. SOS/standard 与 MSI/EXE 分别按支持范围测试，能力声明与实际支持一致。
3. 停主服务、删服务、破坏固定路径、客户端损坏及双目录缺失的分层恢复。
4. 主程序离线但 agent 在线时，真实远程命令接收、执行、终态回执和 UI/SQLite 一致。
5. 双服务停止和 agent 损坏后的独立入口恢复。
6. 网络故障、下载失败、大小/哈希/签名错误、安装失败、恢复失败和回滚失败。
7. 带资源损坏、过期资源、资源身份不一致的救援状态，不能误显示 ready。
8. 安装中断、断电、系统重启、事件丢 ACK、重复命令和续传/补报。
9. 本地禁用、用户停止、维护、安装互斥和活动会话等门禁。
10. 明确区分 Windows 10/11、实际桌面及安装包验收与 mock/单元测试。

禁止在生产机器上未经授权做停服、删文件或故障注入。先 TDD，再最小实现，最后独立代码审查和真实包验收。没覆盖的矩阵要保留并如实说明。

## 九、可复用的现场只读检查命令

在 PowerShell 中复制整行，不包含提示符：

```powershell
Get-CimInstance -ClassName Win32_Service -Filter "Name='RustDesk' OR Name='RustDeskManagementAgent'" | Format-List Name,State,StartMode,StartName,PathName; & 'C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe' status --json
```

查询 Defender 排除项：

```powershell
(Get-MpPreference).ExclusionPath
```

这些命令不会下发重启、重装或更改排除项。不要上传 TOML 私钥、密码或完整凭据文件。

## 十、与现有文档的关系

本文补充实际物理机观察，不覆盖已经存在的完整升级方案、测试手册和后续测试证据。后续工作应交叉阅读：

- [完整升级方案](agent-recovery-upgrade-plan.zh.md)
- [全量测试手册](agent-recovery-test-runbook.zh.md)
- [实施进度与测试证据](agent-recovery-implementation-status.zh.md)
- [VM 实测记录](agent-vm-system-driver.zh.md)

历史会话中曾把 401 归因为固定反代路径、公钥不一致、主服务被安全软件停止，以及保证重启一定上线；这些结论部分缺乏直接证据，不能作为本次实现依据。HTTP 401 需具体鉴权诊断；服务和文件状态需本机证据；重启恢复需真实验收。

**本次最重要的现场结论：真实物理机自动升级后，主程序和 agent 新构建已运行，本地服务恢复启用，但完整救援仍是 `waiting_approval / unverified / ready=false`。需要补齐救援初始化与验证、远程运维实测和更新终态闭环，而不是只把权限开关改成最大。**
