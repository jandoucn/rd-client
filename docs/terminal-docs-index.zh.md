# 终端相关文档索引

## A. 桌面 peer 终端（客户端菜单）

| 文档 | 用途 |
|---|---|
| **`desktop-terminal-admin-system.zh.md`** | **管理员终端 / SYSTEM 会话**：产品行为、连接认证、服务进程身份、跨机验收（2026-10-28） |
| `tdd-ledger-20261027.zh.md` §4 | 桌面终端 T.1–T.5 台账 |
| `windows-disable-uac.zh.md` | 关联运维能力；链到上文 |

## B. Management Agent 网页 SYSTEM 终端（后台浏览器）

| 文档 | 用途 |
|---|---|
| `terminal-protocol.zh.md` | 三方协议（字段/错误码/超时）；不得单方改 |
| `terminal-tdd-ledger.zh.md` | **网页终端唯一执行台账**（T/V/G 状态） |
| `terminal-test-evidence.zh.md` | 证据总册（命令、hash、生产结论） |
| `terminal-test-record.zh.md` | 时间线（何时做了什么） |
| `terminal-usage.zh.md` | 运维使用说明与安全边界 |
| `terminal-client-security-notes.zh.md` | rd-client Agent 侧安全核对 |
| `evidence/terminal-v*.txt` | V2/V4/V5 脱敏输出摘录 |
| `overnight-notes-20261028.zh.md` | 特殊情况与已知限制 |
| `scripts/win10-gs/README.zh.md` | 真机编译/替换脚本 |

rd-api 侧：`docs/terminal-rd-api-progress.zh.md`、`docs/terminal-security-review.zh.md`、`docs/terminal-cross-session-board.zh.md`。

| 复核入口（转发用） | `docs/ops-console-session-review.zh.md` |

A 与 B 通道独立：桌面改主程序 `librustdesk`；网页改 Agent + rd-api + 协议文档。 |
