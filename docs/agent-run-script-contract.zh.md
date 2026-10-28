# 远程脚本执行（system.run）契约

状态：实现中。本文件是 Agent 与后台之间的字段契约，双方以本文件为准。

## 定位

Management Agent 以 **LocalSystem** 身份常驻，是这台机器上权限最高的执行者：不受 UAC 限制，能改服务、驱动、注册表、其他用户目录。`system.run` 让运维在后台直接下发**整段 PowerShell / cmd 脚本**，用于 RustDesk 客户端本身连不上、或需要无人值守执行的场景。

它不是交互式终端：没有 PTY、没有常驻会话。每次下发是一个**作业**，作业有独立的进程、输出文件和生命周期；要看中途进度、要提前终止，用作业查询与终止命令。

## 三条命令

### `system.run` — 启动作业

参数（对象，未知键拒绝）：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `shell` | 字符串 | 否 | `powershell`（默认）/ `pwsh` / `cmd` |
| `command` | 字符串 | 是 | 整段脚本，支持多行、管道、变量、函数；上限 32767 字符 |
| `workdir` | 字符串 | 否 | 工作目录，必须已存在；不传用系统目录 |
| `timeout_seconds` | 整数 | 否 | 1–900，且不超过命令信封的超时；默认取信封值 |

`shell` 语义：

- `powershell`：装了 PowerShell 7 用 `pwsh.exe`，否则回退系统的 `powershell.exe`（`-NoProfile -NonInteractive -ExecutionPolicy Bypass`）；
- `pwsh`：强制 PowerShell 7，不存在则作业直接失败并给出原因，不回退；
- `cmd`：`cmd.exe /c`。

**立即返回**（不等脚本结束）：

```json
{"schema": 1, "job_id": "0123456789abcdef", "state": "running", "shell": "powershell", "started_at": 1793095000}
```

### `system.run.status` — 查状态与增量输出

参数：`job_id`（必填）、`offset`（可选，默认 0）。

返回：

```json
{
  "schema": 1,
  "job_id": "0123456789abcdef",
  "state": "running",
  "exit_code": null,
  "timed_out": false,
  "killed": false,
  "started_at": 1793095000,
  "ended_at": null,
  "duration_ms": 12500,
  "stdout": "已追加的输出",
  "stderr": "",
  "next_offset": 2048,
  "stdout_truncated": false,
  "stderr_truncated": false
}
```

- `state`：`running` / `succeeded` / `failed` / `timed_out` / `killed`
- `stdout` / `stderr` 只返回从 `offset` 起的新增内容；读完用 `next_offset` 作为下次偏移
- 单次返回上限 32 KB，超出部分下次继续取
- 作业总输出上限 1 MB，超出后停止追加并置 `truncated=true`
- 作业不存在或已被清理：返回错误码 `run_job_absent`

### `system.run.kill` — 终止作业

参数：`job_id`。终止整棵进程树（`taskkill /T /F` 语义），作业状态置 `killed`，已有输出保留可读。

## 作业存储

```
C:\ProgramData\RustDesk\ManagementAgent\state\run-jobs\<job_id>\
    meta.json     状态、退出码、起止时间、shell、脚本摘要（不落原文）
    stdout.log    标准输出（追加写）
    stderr.log    标准错误（追加写）
    command.txt   原始脚本（仅本机 SYSTEM 可读，便于事后核对）
```

- 保留最近 20 个作业，或 24 小时内；超出的旧作业在下次启动新作业时清理
- Agent 重启后，仍在运行但已无父进程的作业标记为 `failed`，附 `agent_restarted`
- 作业目录权限沿用 Agent 状态目录的加固（SYSTEM 独占）

## 门禁

`system.run` 与 `system.run.kill` 同 `system.disable_uac` 一样先过本地门禁，任一命中即拒绝，命令以 `local_gate_*` 失败，`error_code` 为 `local_gate_refused`：

- `disable-recovery` 标记（purge）
- `disable-agent` 标记
- `maintenance.json` 维护态
- 用户明确停止意图（`stop-service=Y`，或配置读不出）

`system.run.status` 是只读，不受上述门禁阻挡（否则用户停用后无法查看已跑完的结果）。

## 后台展示要求

- 操作页新增「脚本」区：shell 选择、命令输入（多行）、工作目录、超时，历史命令可一键重发
- 提交后出现作业卡片：状态徽标、已运行时长、输出区（stdout/stderr 分开）、「停止」按钮
- 输出区每 2 秒增量拉取一次，滚动到底部时自动跟随；作业结束后停止轮询
- 作业卡片与操作日志同页，退出码与失败原因就地显示

## 验收

1. 下发 `Get-Date` 能在 3 秒内看到输出与退出码；
2. 下发一个持续 30 秒、每 2 秒打印一行的脚本，能在运行过程中看到逐行增加，而不是结束后一次性出现；
3. 下发 900 秒脚本后点「停止」，进程树消失、状态为 `killed`、已产生的输出仍可读；
4. 超过超时的脚本自动终止，状态 `timed_out`，保留已产生的输出；
5. 多行脚本、管道、中文输出不乱码（PowerShell 输出按 UTF-8 / 系统代码页正确解码）；
6. `disable-agent` 存在时下发 `system.run` 被拒，`system.run.status` 仍可查；
7. 输出超过 1 MB 时截断并标记；
8. 未知参数键、超长命令、不存在的工作目录均被拒绝且不启动进程。
