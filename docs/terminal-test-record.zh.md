# SYSTEM 终端 · 测试时间线记录

> 按时间顺序记「做了什么 → 证据 → 结论」。细节命令与摘录见 `terminal-test-evidence.zh.md`。  
> 时区：CST（UTC+8），除非另行标注。

---

## 2026-10-27

| 时刻 | 动作 | 证据 / 结论 |
|---|---|---|
| 白天–晚 | Claude 做 T1 PTY + 协议/台账；T2 runtime 写到一半被 `[cyber]` 拒 | handoff / overnight-notes |
| 晚 | grok 续 T2：修 `runtime.rs` nonce、挂 `service_main`、能力声明 | Mac 66 测绿 |
| ~20:48 | win10-gs SYSTEM 下 PTY 套件 | V1 **32** 绿 |
| ~23:06 | win10-gs `cargo test … terminal::` | **71** 绿；debug 构建成功 |
| 晚 | 首次替换 agent 路径搞错（Program Files） | 后改为 ProgramData |
| 晚 | rd-api 发 **0.1.55** | nginx 正则未加引号 → 生产 upgrade 失败回滚 0.1.54 |
| 晚 | rd-api 发 **0.1.56**（正则加引号） | CI 绿 |

## 2026-10-28

| 时刻 | 动作 | 证据 / 结论 |
|---|---|---|
| ~01:18 | `upgrade-api.sh 0.1.56` | exit 0；VERSION=0.1.56 |
| 凌晨 | 替换正确路径 agent；误写 `disable-recovery` 导致 status 停更 | 去掉后 status 恢复并出现 `system.terminal@1` |
| 凌晨 | agent-auth 403→200；WS ESTABLISHED 到 47.100.7.221:443 | 设备 1338209581 |
| ~06:21 | V4 WSS open/input/close | whoami=SYSTEM；`__V4_OK__` → **V4 ✅** |
| ~06:22 | V5 断线 attach 后再输入 | `V5_AFTER` 回显；marker 回放未观测 → **V5 ✅**（限制已记） |
| ~06:23 | V2 resize 140×45 | `V2_COLS=140 V2_ROWS=45` → **V2 ✅** |
| ~06:25 | 台账 V2–V6 / G1–G6 对齐；G7 AOCI MCP 不可用记 ⚠️；G8 保留 script_run | |
| ~06:30 | 重写证据总册 + 本时间线 + 脱敏摘录入 `docs/evidence/` | |

---

## 通过标准对照（联调）

| ID | 通过标准 | 状态 |
|---|---|---|
| V1 | SYSTEM 下 PTY 出输出、中文 UTF-8 | ✅ |
| V2 | resize 后 shell 可见新行列 | ✅ 140×45 |
| V3 | Agent 重启续会话 | ⏭️ v2 |
| V4 | 生产后台链路敲命令见 SYSTEM 输出 | ✅ |
| V5 | 断浏览器 WS 后续会话可输入 | ✅ |
| V6 | 生产版本 + 真机 + 上述链路 | ✅ 0.1.56 |

---

## 未关闭 / 跟进

| 项 | 说明 |
|---|---|
| G7 AOCI | MCP handshake 失败；CLI 可用时对改动文件跑 maintain |
| G8 script_run.rs | 保留半成品，不进能力清单 |
| V5 历史回放 | last_seq 近顶端时 attach 未见旧 marker；需专项测大 gap |
| 前端 | `channel.ts` 已在 opened 时覆盖 sessionId，与协议发现一致 |
| 正式发版 client | 大量未提交工作区；终端相关需单独整理提交（中文说明） |
