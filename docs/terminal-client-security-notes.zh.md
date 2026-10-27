# SYSTEM 终端 · rd-client 侧安全核对（2026-10-28）

> 对照 `docs/terminal-protocol.zh.md` 与 AGENTS.md「SYSTEM 终端授权例外」。  
> 范围：`src/management_agent/terminal/**`、`status.rs` 能力声明、`service_main` 挂钩。  
> api 侧正式审查见 `rd-api/docs/terminal-security-review.zh.md`。

## 结论

未发现 CRITICAL/HIGH。下列为已核对事实与低优先级注意。

## 已核对

| 项 | 结论 |
|---|---|
| 能力边界 | 仅 Windows x86_64 声明 `system.terminal@1`；未声明设备后台不提供终端入口（后端按 status caps 拒绝） |
| 通道分离 | 终端走独立 WS runtime；不进入 commands 命令通道的未列入能力清单的执行面 |
| 本地门禁 | open/input 与 UAC 同套：disable-recovery / disable-agent / maintenance / 停止意图；close/attach 清理路径保留 |
| 审计 | `AuditEvent` 仅 session 元数据（id/shell/尺寸/原因/门禁名）；`log_audit` info 级同样无 IO 正文 |
| 会话缓冲 | 输出环在 Agent 内存；会话结束释放；不落盘、不写业务日志正文 |
| 身份 | 握手复用设备 Ed25519 签名与 commands 同源 canonical（空 body） |
| 前端 session | `channel.ts` 在 `opened` 时采纳服务端 `session_id`；open 帧本身不带客户端 id，避免改写冲突 |

## 注意 / 跟进

| 项 | 级别 | 说明 |
|---|---|---|
| `disable-recovery` 语义 | 中 | 文件存在时 **整个** service_main 循环 continue，status 也停。运维文档与脚本注释已写；勿当「仅停修复」 |
| 救援盖回旧 agent | 低 | 真机联调需核对 ImagePath hash；replace 脚本已备份 |
| debug 二进制上生产机 | 低 | 联调使用 debug agent；正式发版须用发布构建替换 |
| G7 AOCI | 低 | 全仓 `aoci check` volumes 治理 blocked（219 findings，不全是本功能）；MCP 曾 handshake 失败 |

## 测试交叉

- 单元：Mac 66 / win10-gs 71 `terminal::`
- 生产：V4 whoami=SYSTEM；审计无 IO；门禁单测 `terminal_local_gate`
