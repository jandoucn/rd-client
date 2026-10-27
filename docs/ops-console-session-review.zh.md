# 运维控制台「交互会话」v1 · 复核说明

请按下列材料复核。范围仅限 **rd-api + rd-client** 本功能 v1，**不要**和仓库里其它未提交改动混在一起审。

最后整理：2026-10-28。

---

## 一句话

管理后台网页里的交互会话页，经 stream-broker 的 WebSocket 中转到设备上 Management Agent 承载的伪终端会话；与既有「命令下发」通道分开。生产 API 镜像 **0.1.56**。台账上 v1 必做项已闭环；审计时收紧了两处原先写得偏满的表述。

---

## 仓库与版本

| 项 | 值 |
|---|---|
| API 仓 | `/Users/olly/github/rd-api`，分支 `main` |
| 客户端仓 | `/Users/olly/github/rd-client`，分支 `master` |
| 生产镜像 | `ccr.ccs.tencentyun.com/olly/rd-api:0.1.56` |
| 生产绑定 | 宿主机 `127.0.0.1:7000` → 容器 `80` |
| 管理前缀 | `/yanolly` |
| 会话页 | `https://rdapi.yan.life/yanolly/terminal` |
| 发版 | `v0.1.55` 功能；`v0.1.56` 修 nginx 路由正则（含 `{32}` 的 location 须整体加引号，否则进程起不来） |

---

## 必读文档（按顺序）

1. **台账（唯一进度源）**  
   `docs/terminal-tdd-ledger.zh.md`  
   状态：完成 · 延期下一版 · 用户否决 · 已知限制  

2. **证据**  
   - `docs/terminal-test-evidence.zh.md`（含「审计核对」）  
   - `docs/terminal-test-record.zh.md`  
   - `docs/evidence/terminal-v{2,4,5}-out.txt`  

3. **协议（字段三方不得单方改）**  
   `docs/terminal-protocol.zh.md`  

4. **使用说明与安全审查**  
   - `docs/terminal-usage.zh.md`  
   - `/Users/olly/github/rd-api/docs/terminal-usage.zh.md`  
   - `/Users/olly/github/rd-api/docs/terminal-security-review.zh.md`  

5. **API 进度 / 协作板**  
   - `/Users/olly/github/rd-api/docs/terminal-rd-api-progress.zh.md`  
   - `/Users/olly/github/rd-api/docs/terminal-cross-session-board.zh.md`  

6. **索引**  
   `docs/terminal-docs-index.zh.md`  

本文件路径：`docs/ops-console-session-review.zh.md`（给人转发的复核入口；路径与协议字段以台账/协议为准）。

---

## 台账结论（请对照）

| 范围 | 结论 |
|---|---|
| 阶段 T1–T4、联调 V1–V6、收尾 G1–G6 | 台账为完成；有点名测试或摘录 |
| T5 列表轮询改间隔 | 用户否决，仍为 5s / 3s |
| T1.6 / T1.7 / V3（进程重启后恢复同一会话） | 明确延期下一产品版 |
| G7 仓库认知索引 | 工具握手失败，未 maintain |
| G8 未完成的作业式脚本模块 | 本轮保留源码，不进入能力声明 |

### 审计收紧（避免假绿）

- **G3**：终端相关子集 + CI；**不是**本机重跑全仓 Python 全量套件。  
- **T4.2**：有基础输出与页面 E2E、以及尺寸变化联调；**无**颜色 / 光标 / 清屏专项断言。  
- **V5**：断线后再附着可继续输入已验证；序号已近顶端时，历史帧回放可能无可补内容（台账已写限制）。  

---

## 建议复核命令

```bash
# API
cd /Users/olly/github/rd-api
(cd stream-broker && go test ./... -count=1)
cd tests && python3 -m unittest terminal_http_test terminal_nginx_config_test terminal_e2e_test
# 页面 E2E（需本机 Playwright 环境，例如 /tmp/rd-api-e2e-venv）
# /tmp/rd-api-e2e-venv/bin/python e2e/terminal.spec.py

# 客户端（开发机）
cd /Users/olly/github/rd-client
cargo test --locked --bin rustdesk-management-agent -- terminal::

# 客户端（win10-gs 构建机，termark 资产与脚本见 scripts/win10-gs/README.zh.md）
# bash scripts/win10-gs/run-build.sh debug 'terminal::'
```

审计整理时曾现跑：Go 通过；Python 会话相关 19 项通过；开发机 `terminal::` 过滤器 66 项通过；生产仍为 0.1.56。

---

## 关键实现位置

### rd-api

- `sqlite/terminal.php` — 短鉴权与审计元数据写入  
- `stream-broker/terminal.go`、`ws.go` — 会话中转、输出环、限额  
- `config/nginx.conf` — 会话相关 HTTP/WS 路由（正则加引号）  
- `web/src/views/terminal/`、`web/src/modules/terminal/`  
- `tests/terminal_*.py`、`tests/e2e/terminal.spec.py`  

### rd-client

- `src/management_agent/terminal/` — 会话、协议、传输、本地门禁  
- `src/management_agent/status.rs` — 能力标识 `system.terminal@1`  
- `src/management_agent.rs` — 注册运行线程  

---

## 生产抽查要点

- 示例设备 ID：`1338209581`，状态能力列表含 `system.terminal@1`  
- 审计曾有会话打开记录：只含 shell、行列、会话号、来源等**元数据**，不含键入/回显正文  
- 浏览器侧：全局管理员会话、CSRF、同源校验  
- 设备侧：设备签名、安装身份绑定、能力声明  
- 设备本地维护/停用类门禁优先于远程「打开 / 输入」  

---

## 明确不在本轮 / 勿混审

- 工作区 UAC、安装包、日志刷新等其它未完成改动  
- 下一版：进程重启后恢复同一伪终端会话  
- 仓库认知索引（工具不可用）  
- 删除或产品化未完成的作业式脚本模块  

---

## 请复核人回答

1. 台账每条「完成」是否都能在源码测到对应测试名，或在 `docs/evidence/` / 生产审计里找到摘录？  
2. G3 / T4.2 / V5 的限制表述是否可接受？  
3. 独立通道、审计不落会话正文、本地门禁优先、单管理员/单设备会话上限，实现与文档是否一致？  
4. 生产 0.1.56 与会话页是否仍健康、可打开？  
5. 延期项是否同意放到下一产品版？  

有问题请改台账或开 issue，并在 `docs/terminal-test-evidence.zh.md` 的「审计核对」节补一行。
