# 夜间 / 联调记录（2026-10-27 晚 → 2026-10-28 晨）

> 用户授权：按终端文档 TDD、台账、后端联调、真机 win10-gs、生产升级；不停不问；特殊情况记此文件。  
> 进度主文档：`docs/terminal-tdd-ledger.zh.md`。证据：`docs/terminal-test-evidence.zh.md`。时间线：`docs/terminal-test-record.zh.md`。

## 特殊情况 / 已知限制

1. 【文档】台账 §1 曾写「命令通道」，协议 §9 为独立 WS；实现按协议。
2. 【生产 0.1.55】nginx 终端 location 正则未加引号，`{32}` 当配置块，升级失败回滚；**0.1.56** 已修并上生产。
3. 【路径】win10-gs Management Agent 服务二进制在 `C:\ProgramData\RustDesk\ManagementAgent\`，不是 Program Files。
4. 【门禁】`state/disable-recovery` 会跳过 agent **整圈**主循环（含 status），勿当「只关修复」。
5. 【协议】broker 可能改写 open 的 `session_id`，必须以 `opened.session_id` 为准（前端 channel 已处理）。
6. 【V5】attach 后续输入已验证；历史 marker 回放在 last_seq 近顶端时未观测到。
7. 【G7】本会话 aoci MCP handshake 失败；业务不阻塞，待 MCP 恢复后 maintain。
8. 【G8】`script_run.rs` 本轮保留不删、不声明能力。
9. 【V3/T1.6】Agent 重启续会话明确 v2。

## 关键结论（2026-10-28 06:30）

- 生产 **0.1.56** 健康；设备 **1338209581** caps 含 `system.terminal@1`；终端 WS ESTABLISHED。
- **V1–V2、V4–V6** 台账已 ✅；**V3** ⏭️ v2。
- 收尾门禁 G1–G6 ✅；G7 ⚠️；G8 ⚠️ 保留。
