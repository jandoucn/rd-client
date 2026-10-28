# Windows 安装与 Agent 关闭 UAC

状态：已实现（MSI CA、EXE after-install、agent `system.query_uac@1` / `system.disable_uac@1`、status 心跳携带 UAC 快照、rd-api 注册与设备详情「本机 UAC」面板）。真机 MSI 与 agent 补写验收待发测试包后执行。

## 目标

Windows 机器在 MSI 安装阶段写入本机关闭 UAC 的策略；Management Agent 提供同等能力，供后台对已装机器补写。不自动重启，等用户下次自行重启后 `EnableLUA` 完全生效。卸载不恢复 UAC。

范围：

- 发布包：`windows-x86_64-msi-standard`、`windows-x86_64-msi-sos`
- 兼容：EXE after-install 路径同步写入（存量 EXE 设备）
- Agent：Windows x86_64 常驻能力，可远程下发一次补写

非范围：macOS / Android；强制重启；卸载回滚 UAC；域 GPO 对抗。

## 策略内容

键路径（64 位机器视图，与现有 `SoftwareSASGeneration` 相同）：

```text
HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System
```

| 值名 | 类型 | 目标 |
| --- | --- | --- |
| `EnableLUA` | REG_DWORD | `0` |
| `ConsentPromptBehaviorAdmin` | REG_DWORD | `0` |
| `PromptOnSecureDesktop` | REG_DWORD | `0` |
| `FilterAdministratorToken` | REG_DWORD | `0` |

语义与 `scripts/disable-uac/u.ps1` 一致。写完即落盘；`EnableLUA=0` 在下次重启（或等价重新登录会话）后全面生效。当前会话仍可能出现 UAC 提示，属预期。

与现有 `SoftwareSASGeneration=1` 同键共存，互不覆盖。

## MSI 安装

仿现有 `AddRegSoftwareSASGeneration`：

| 项 | 约定 |
| --- | --- |
| CustomAction | 新建 `DisableUacPolicy`（或等价名） |
| 实现位置 | `res/msi/CustomActions/CustomActions.cpp` + `.def` 导出 |
| 执行 | `Execute="deferred"`、`Impersonate="no"`（SYSTEM） |
| 返回 | 写注册表失败记日志；主安装不因此失败（与 SAS/防火墙同类 best-effort；正常 SYSTEM 安装几乎不会失败） |
| 调度 | `Before="InstallFinalize"`，条件对齐 SAS：安装/升级执行，纯卸载不执行 |
| 重启 | 不请求 reboot；调用方继续 `/qn /norestart`、`REBOOT=ReallySuppress` |

实现方式二选一，优先与 SAS 一致的多次 `reg add`，或原生 `RegSetKeyValueW`；四个值一次 CA 内全部写完并可选回读校验。

不新增 MSI 公共属性开关：本私人定制默认永远关闭 UAC，静默/交互/Agent 发起的 MSI 一律写入。

## EXE / after-install 兼容

`src/platform/windows.rs` 的 `get_after_install()` 已在同路径写 `SoftwareSASGeneration`。在其后追加四条 `reg add`（或抽公共命令片段供 install_me / `--after-install` 共用），保证：

- 首装 after-install
- 覆盖安装 / 修复走 after-install 的路径

与 MSI 目标值完全一致。不调用 `Restart-Computer`。

## Agent 能力

### 能力名

```text
system.query_uac@1
system.disable_uac@1
```

- 仅 Windows x86_64 声明（与 MSI 包能力同一平台门）
- 写入 `status` 的 `capabilities` 列表
- 心跳 `status.uac` 携带四键整型与 `fully_disabled`（只读快照，供后台面板展示）
- 命令 kind：`system.query_uac`（观察权限，实时回读）、`system.disable_uac`（服务权限，写入）
- 权限：查询纳入 observe；关闭纳入 API 绑定默认授予的 agent 运维权限（与现有 service 同类）
- 作用对象：仅本机固定 `Policies\System` 四键；禁止任意路径、任意脚本
- 后台：设备详情「概览 → 本机 UAC」专用面板（查询状态 / 一键关闭）；不进通用操作列表，避免与面板重复

### 执行（query）

1. 以 LocalSystem 打开 `HKLM\...\Policies\System`，只读四键
2. 结果上报：`uac` 对象（四键 + `fully_disabled`）；读失败返回 `uac_policy_query_failed`
3. 不写注册表、不重启、不检查本地维护门禁（只读）

### 执行（disable）

1. 校验本地门禁：`disable-recovery`（purge 标记）、`disable-agent`、`maintenance.json`、用户明确停止意图（RustDesk 配置 `stop-service=Y`；配置读不出或值无法识别时同样拒绝）任一命中即拒绝，命令以 `local_gate_*` 失败，上报的 `error_code` 为 `local_gate_refused`；写注册表失败上报 `uac_policy_apply_failed`，后台据此区分「本地拒绝」与「写入失败」
2. 以 LocalSystem（agent 服务身份）打开 `HKLM\...\Policies\System`，写入四键
3. 回读校验；全为 `0` → 成功
4. 结果上报：`applied`（已是目标值也算成功）、`reboot_required`（`EnableLUA` 本次从非 0 改为 0，或开机以来尚未生效时为 true；已是 0 且无需重启标记可 false）、错误码（写失败时）
5. **不重启**、不弹 UI、不跑外部 `u.ps1`

幂等：已是目标值再执行一次仍成功，不改其它键。

### 审计

命令、事务、事件按现有 operations 审计链保留：谁下发、何时写、写前/写后快照（仅四键整型，无敏感数据）、是否需要用户重启。

## 卸载

卸载 / purge **不**删除或还原上述四键。机器安全策略一旦关闭 UAC，保持关闭，避免卸载后突然恢复弹窗或误开。

`SoftwareSASGeneration` 现有行为不在本文扩展范围内，本文不改其卸载语义。

## 失败与日志

写注册表在 elevated/SYSTEM 下常态成功。若失败（键权限异常、极少数第三方锁定）：

- MSI：记 MSI 日志，安装继续
- after-install：记应用日志，安装流程按现有 reg 命令习惯继续
- Agent：命令结果失败 + 错误信息，可重试；不升级为整机 maintenance

不做 GPO 对抗；若域策略随后覆盖回 `EnableLUA=1`，以系统策略为准，agent 可再次下发补写，仍不保证对抗持续 GPO。

## 代码落点（实现时）

| 层 | 文件/位置 |
| --- | --- |
| MSI CA | `res/msi/CustomActions/CustomActions.cpp`、`.def` |
| MSI 调度 | `res/msi/Package/Fragments/CustomActions.wxs`、`Components/RustDesk.wxs` |
| EXE after-install | `src/platform/windows.rs` → `get_after_install` |
| Agent 能力声明 | `src/management_agent/status.rs` → `capabilities` / `capabilities_for` |
| Agent 命令路由 | `src/management_agent/operations.rs`、`operations_runtime.rs` |
| 写键实现（建议） | `src/management_agent/` 或 `src/platform/windows*.rs` 内单一函数，CA 侧可独立 C++ 同语义，避免 MSI 链依赖 Rust |
| 运维脚本 | `scripts/disable-uac/` 保留人工/应急；安装与 agent **不**调用它 |
| 发版说明 | `release-notes.zh.md` 记：安装默认关 UAC，需自行重启后完全生效；卸载不恢复 |

## 测试要点

1. **单元**：目标四键常量；「已是 0 / 部分非 0 / 全缺」→ 写入集合与 `reboot_required` 判定
2. **MSI 字符串/CA**：调度条件、导出符号、不请求 reboot
3. **after-install 命令串**：含四条目标 `reg add`（或等价）
4. **Agent**：capability 出现在 status；命令成功幂等；门禁拒绝路径；审计字段
5. **win10-gs 真机**：静默 MSI `/qn /norestart` 装后四键为 0、进程未重启；手动重启后 UAC 关闭；再跑 agent 命令仍成功；卸载后四键仍为 0

## 决策记录

| 项 | 决定 |
| --- | --- |
| 范围 | Windows MSI + Agent 能力（EXE after-install 同步） |
| 失败 | 写注册表常态成功；异常只记日志/命令失败，不阻断主安装 |
| 卸载 | 不恢复 UAC |
| 重启 | 从不自动重启，等用户下次自行重启 |
| 开关 | 无 MSI 属性；默认始终写入 |

## 关联：桌面管理员终端 / SYSTEM 会话

与关闭 UAC 同属私人机群运维能力，但代码路径独立。

完整产品说明、实现落点、跨机验收（2026-10-28 win10-gs → GepShare）见：

**`docs/desktop-terminal-admin-system.zh.md`**

摘要：已安装 Windows 服务上，桌面「管理员终端 / SYSTEM 终端」使用服务进程 LocalSystem 身份承载会话；连接侧只做 RustDesk 连接认证（固定/一次性密码与控屏共用记住逻辑），不再收集本机 Windows 交互登录口令。只换 Management Agent 不会生效，须主程序（`librustdesk.dll` 等）。
