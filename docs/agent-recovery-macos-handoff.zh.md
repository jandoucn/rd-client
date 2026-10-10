# Management Agent Mac 开发交接

日期：2026-10-10

## 1. 当前真实状态

本次完成的是两仓库源码调查、升级方案和测试执行规范。尚未新增 Rust/PHP/Flutter/Go 功能代码，也没有新能力安装包。不要将方案中的接口、bootstrap、能力清单、事务驱动和测试 runner 当作已经实现。

客户端实施基线为 `5c13e154ae8e2225032c8e6fcd7a10a49b642506`，后台为 `d065b96498008784a1bc46146b3c4f8196350553`。换机后先拉取当前远程主分支，再检查这些基线之后的 diff；如果有新增实现，先核对再继续，避免重复。

本次用户已要求开始实现，但因换 Mac，转为先推送文档交接。后续 Mac 上可以直接开展已授权的本地开发和测试；未来 commit/push/发布/生产部署仍需按届时明确请求和仓库合同执行。

## 2. 必读文件与仓库

按顺序读取：

1. 两仓库 `AGENTS.md`。
2. [完整升级方案](agent-recovery-upgrade-plan.zh.md)。
3. [全量测试手册](agent-recovery-test-runbook.zh.md)。
4. [历史真机测试](windows-agent-live-test.zh.md)、[MSI 交接](windows-management-agent-repair-handoff.zh.md)。
5. 后台 `docs/client-management-contract.md`、`docs/mysql-support-freeze.md`。

仓库地址和主分支：

| 仓库 | 远程 | 必须使用的主分支 |
| --- | --- | --- |
| RustDesk 客户端 | `https://github.com/jandoucn/rustdesk.git` | `master` |
| RustDesk API | `https://github.com/jandoucn/rustdesk-api.git` | `main` |

禁止创建/切换功能分支。已经有 checkout 时先看 status，不覆盖现场未提交内容；源码依赖按 `.gitmodules` 初始化，客户端有子模块。两仓库建议并排放在同一父目录，文档的相对链接据此组织。

## 3. 已核实的关键结论

- 旧 agent 只在正常/保底 exe 都不存在时恢复，不判断主服务健康。
- 保底文件存在会提前跳过轮询后续流程，新实现须持续观测/策略刷新。
- 两端当前签名 canonical path 都是 `/rd/update/v1/agent/policy`，历史 401 根因尚未由生产证据确认。
- 主程序 API 心跳、agent 联络、本地 ready、远控注册是不同事实，不混写主心跳。
- 后台已有 Ed25519、nonce、公钥绑定、policy revision、旧 check/install 命令和 Go stream broker；MySQL 冻结。
- EXE 更新由旧程序启动下载的新包 `--update`，包内新程序能够在替换前运行桥接。曾经“必然旧二进制执行更新”的判断已纠正。
- 当前 EXE 在线更新未证明替换 ProgramData agent；MSI 永久组件也需实测文件版本和服务控制。
- 旧 agent 不能读取新的 migration/transaction 格式，第一次停服前必须部署新 helper/接续入口。
- 普通 agent 升级保留 installation_id；新的授权登记/撤销后授权才改变绑定代次。
- 跨进程 Windows mutex 不能直接转移线程所有权，按持久事务和握手接管。

## 4. 第一批开发顺序

先用 TDD 构建状态机、协议 fixtures 和真实测试入口。不可在缺少测试基础时一次改完安装器、核心协议和控制台。

1. 核对工具、两仓库摘要与现场旧包清单，建立能力—测试目录和失败测试。
2. 进行三个隔离 Windows 可行性试验：`POC-EXE`、`POC-MSI`、`POC-RESUME`，详见方案第 16.1 节。
3. 新增 agent 专属模块：服务/意图观测、纯恢复状态机、持久化队列/事务。共享文件只加必要薄钩子。
4. 后台 SQLite 增量表、精确身份授权、status、commands/claim、events、包解析；复用原验签，旧接口兼容。
5. Windows helper、互斥、接续任务、RecoveryBootstrap、本地保底和离线恢复资产。
6. EXE/MSI 正常安装/旧版迁移接入；不能只改新 agent 让现场旧版自己理解新协议。
7. 管理页展示与命令流程，全量源码门禁、四包安装/升级 E2E、故障注入和灰度证据。

最终必须做到：主程序下线可本地自动救活，R0–R6 fallback 可验证，agent 自身有后备入口，API 断网仍可恢复，旧版一次可用在线升级获得新 agent 全部适用能力。所有指标以真实测试为准，不保证完全失联旧机器能凭新协议远程救回。

## 5. Mac 与 Windows 的分工

Mac 可运行：Rust 平台独立测试、PHP/SQLite 集成、Go broker、Playwright、Docker、本地协议测试，以及适用 Flutter 桌面测试。

Windows 必须运行：SCM/LocalSystem、计划任务、EXE/MSI 安装和升级、UAC/锁屏/登录桌面、helper 退出/断电接续和实际远控。Mac 交叉编译只能证明编译，不能替代 Windows E2E。

Apple Silicon Mac 需要明确 Windows ARM VM 与 x86_64 产品目标差异。Windows ARM 上模拟运行 x64 可提供补充证据，但不能宣称完整替代原生 Windows x64/Windows 10 矩阵；优先准备受控 x64 Windows VM/独立测试机。不得测试时切断唯一远程管理链路。

Windows 未就绪时继续独立低层开发，标记平台验收 BLOCKED；不能为了消除阻塞改 macOS 签名、Android 证书或产品固定配置。

## 6. 已知测试入口与纠错

客户端根目录：

```bash
cargo test --locked --bin rustdesk-management-agent -- --nocapture
cargo fmt --all -- --check
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Flutter 工作目录：

```bash
flutter analyze
flutter test
```

后台 Go 必须以 `stream-broker` 为工作目录：

```bash
go test -count=1 ./...
```

后台 PHP/Python 入口见测试手册第 5/10 节，显式排除 `mysql_integration_test.py`。PHP 要有 sodium/pdo_sqlite，`FRANKENPHP` 可指向源码支持的 PHP CLI 或 FrankenPHP；默认 `/tmp` 路径不是新机器保证存在的运行时。

后台没有 package.json/Playwright lock，测试资产实施时需固化依赖；已有浏览器测试用 Docker exec 直接核对 SQL，URL、管理路径、容器及 fixture 必须一致。当前 Compose 含固定 container_name 和数据卷，不能仅加 `-p` 就认为完全隔离。

## 7. 当前验证和环境阻塞

- 已通过两份新增文档的 `git diff --no-index --check` 空白检查，审查了调用链和对应源码。
- 未运行 Cargo、Flutter、PHP、Go、Docker、Playwright 或 Windows 安装 E2E。本 Windows 环境起初只有 Python/Node/Git，其他工具未就绪。
- 在临时目录尝试准备 Rust GNU 工具链，下载超时，未形成可用构建/测试证据；该临时工具目录不属于源码交付。
- 新接口、测试 fixture、可执行 runner、bootstrap、helper 完整事务尚未实现。
- 线上 YJ/YXJ 仍需核对实际部署版本、SCM/日志和 401 分类；没有访问生产或改变线上数据。

本次 Git 交付只包含设计、测试规范、交接和原始需求关联。安装 MSI、安装日志、`.spec-workflow` 运行资产不是实现代码，不作为本次提交内容。

## 8. 给 Mac OpenCode 的续接请求

可以直接使用下面文字，按本机实际路径替换仓库位置：

> 继续开发 RustDesk Management Agent。客户端与后台两个仓库已经在本机，分别使用 master 和 main，禁止创建功能分支。先读取两仓库 AGENTS.md、客户端 docs/agent-recovery-macos-handoff.zh.md、agent-recovery-upgrade-plan.zh.md 和 agent-recovery-test-runbook.zh.md。此前完成的是方案和测试规范，没有新 agent 功能实现；先核对远程新增提交再按 TDD 开发。目标是主程序离线自动逐级救活，多层本地/在线 fallback，agent 自身 SCM/bootstrap 兜底，EXE/MSI 旧版一次在线升级获得全部适用能力。先验证 POC-EXE、POC-MSI、POC-RESUME，建设固定 fixtures、能力测试清单和执行驱动，再实现状态机、SQLite 后台接口、事务/helper/接续、保底包和控制台。Mac 运行低层/API/浏览器测试，Windows 实机或隔离 VM 验证服务、四包安装/升级、UAC和断电恢复；未执行项记 BLOCKED。保留所有固定密码、服务器、公钥、权限、手动构建、macOS 签名和 Android 证书约束；MySQL 冻结。代码完成执行全量检查与实际包 E2E，报告源码/包摘要、测试数量、错误和回归面。先完成本地开发，不擅自提交、push、发布或改生产。
