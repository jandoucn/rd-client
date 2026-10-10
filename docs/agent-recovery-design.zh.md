# Management Agent 兜底方案

日期：2026-10-10

这份文档只整理需求和方案，不代表已经改代码。

本文保留为原始需求与历史故障记录。后续开发以 [完整升级方案](agent-recovery-upgrade-plan.zh.md)、[全量测试手册](agent-recovery-test-runbook.zh.md) 和 [Mac 开发交接](agent-recovery-macos-handoff.zh.md) 为准。本文的 401 路径根因、policy/stream 成功判据和初版命令/事件建议已在完整方案中复核修正，不应直接作为实现契约。

## 背景

RustDesk 主程序和 management agent 是两个服务。

- 主程序服务名是 `RustDesk`，负责上线、被控和在线更新。
- agent 服务名是 `RustDeskManagementAgent`，以 LocalSystem 运行，负责主程序丢失后的恢复。
- 主程序路径：`C:\Program Files\RustDesk\RustDesk.exe`
- agent 实际运行路径：`C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe`
- 保底客户端路径：`C:\ProgramData\RustDesk\Client\<edition>\RustDesk\RustDesk.exe`
- agent 策略接口：`GET /rd/update/v1/agent/policy`
- 主程序在线流：`GET /rd/update/v1/policy/stream`
- 下载地址限制：`https://download.yan.life/`

目前 agent 每 30 秒检查一次。它只在两个 `RustDesk.exe` 都不存在时才会下载并重装。只要其中一个文件还在，它就认为不需要恢复。它不会检查主服务是否运行，也不能接收远程命令。

## 已观察到的问题

YXJ，ID `528916907`：

- 主程序从 2026-10-08 11:49 后离线。
- agent 一直在发 `/rd/update/v1/agent/policy`，但全部返回 401。
- 机器重启后 agent 仍在，主程序没有上线。
- 401 不是主程序离线造成的。agent 签名原文写死为 `/rd/update/v1/agent/policy`，反代后的实际验签路径不一致。

YJ，ID `497732374`：

- agent 请求返回 200，验签正常。
- 2026-10-09 17:20 后主程序离线。
- 2026-10-10 主程序没有任何 `policy/stream` 请求。
- agent 正常不能代表主程序正常。文件还在但服务没有拉起时，现有 agent 不会处理。

Windows 托盘退出不会停止主服务。因此“托盘退出后第二天不上线”不能用托盘退出解释，应该检查服务状态、启动类型和 binpath。

## 用户目标

机器可能在异地。主程序因为意外没有上线时，不能只能靠人工到现场处理。

希望 agent 成为主程序的兜底层：

- 主程序服务异常停止时，agent 能拉起它。
- 服务损坏时，agent 能重建服务。
- 主程序文件丢失时，agent 能自动重装。
- API 可以远程下发明确的恢复、重装和更新指令。
- 协议一次设计完整，后续尽量只改 rustdesk-api 就能扩展。
- 允许这次彻底改造 agent。已经安装的旧 agent 可以等待下一次成功的在线更新替换。
- 用户最初希望预留执行 cmd 或 PowerShell 的能力，以便远程处理意外情况。

## 建议方案

把 agent 改成“本地状态机优先，API 命令补充”的运维代理，不把它改成远程 shell。

### 本地状态机

agent 每 30 秒执行一次，顺序固定：

1. 存在 `disable-recovery` 标记时，停止自愈和远程安装。
2. 读取主程序配置中的 `stop-service`。值为 `Y` 时表示用户主动停用，不拉起服务。配置无法读取时标记为 `intent_unknown`，不猜测用户意图。
3. 检查主 exe、保底 exe、服务是否存在、服务状态，以及服务 binpath 是否指向可信 exe。
4. 主 exe 存在但服务停止或 binpath 错误时，修正并启动服务。
5. 主 exe 不存在但保底 exe 存在时，把服务指向保底 exe 并启动。
6. 两个 exe 都不存在时，下载已签名安装包并重装。
7. 启动后等待主程序重新出现 `policy/stream` 成功请求。没有成功不能立刻重复启动。

同一类失败使用 5 分钟、15 分钟、30 分钟、1 小时退避。系统启动后的前 3 分钟不判定为失败，避免服务尚未完成启动时被误重装。

### 三层兜底

- 第一层：启动已有服务，处理开机未拉起或服务被停止。
- 第二层：exe 还在但服务不存在或 binpath 损坏时，重建服务。
- 第三层：两个 exe 都不存在时，下载并安装已签名包。

安装必须满足：

- 下载地址以 `https://download.yan.life/` 开头。
- manifest 签名有效。
- 包大小和 SHA-256 匹配。
- 安装过程使用独立 helper，避免安装包停止 agent 后没有程序负责恢复 agent。
- 安装完成后确认主服务运行，并且 agent 服务也恢复运行。

不能再用“exe 文件存在”代表主程序在线。YJ 的情况就是文件存在，但服务没有运行。

### API 扩展

保留现有策略接口，不在策略响应里混入命令。

新增两个使用现有设备签名的接口：

- `POST /rd/update/v1/agent/status`
- `GET /rd/update/v1/agent/commands`

状态上报至少包含：

- schema 版本
- agent 版本
- 主服务状态
- 服务 binpath
- 主 exe 是否存在
- 保底 exe 是否存在
- 是否用户主动停用
- 最近错误
- 最近一次自愈时间

管理页面应能区分：

- 主服务没有拉起
- 主程序文件被删除
- 用户主动停用
- agent 失联
- agent 请求仍在但签名失败

命令使用固定信封：

```json
{
  "schema": 1,
  "id": "命令号",
  "type": "service.start",
  "expires_at": 1790000000,
  "args": {}
}
```

agent 只执行白名单中的 `type`。未知类型返回 `unsupported`，不能执行。这样后续 API 可以增加类型，而旧 agent 不会误执行。

建议的首批类型：

- `service.status`
- `service.start`
- `service.restart`
- `service.recreate`
- `file.exists`
- `package.reinstall`
- `package.update`
- `recovery.disable`
- `recovery.enable`

命令要求：

- 绑定设备 ID 和 UUID。
- 有过期时间。
- 每个命令最多执行一次。
- 执行结果写入现有事件表。
- 状态沿用 `accepted`、`started`、`installed`、`failed`、`expired`，并增加 `unsupported`。

## 不建议的能力

不建议提供通用 cmd、PowerShell、任意文件上传或任意程序执行。

原因：

- agent 使用 LocalSystem，通用命令执行等于把 API 权限扩大成整机控制。
- API 账号、签名密钥或服务端漏洞会直接影响所有在线 agent。
- 服务程序远程下载并执行任意命令，容易被 360 等安全软件判定为远控行为。
- 新运维动作可以通过新增明确的 `type` 扩展，不需要万能执行口。

如果后续确有必要增加高风险动作，应该单独设计类型、参数白名单、审计记录和二次确认，而不是复用一个任意命令接口。

## 发布顺序

1. 先发布 API。旧 agent 不调用新接口，因此不受影响。
2. 再发布包含新 agent 的完整安装包。
3. 主程序仍在线的机器通过在线更新替换 agent。
4. YJ 和 YXJ 当前主程序不在线，收不到这次更新。需要先在现场启动或重装主程序，新 agent 才能接管以后的故障。

## 需要外部复核的问题

- 本地状态机的判断顺序是否有遗漏。
- 用保底 exe 临时拉起服务是否会带来版本或权限问题。
- 命令信封是否足以支持以后只改 API 的安全扩展。
- 是否存在必须提供任意命令执行才能解决、且不能通过明确命令类型解决的场景。
- 现有在线更新是否一定能替换正在运行的 agent，安装事务是否会留下旧 agent。
