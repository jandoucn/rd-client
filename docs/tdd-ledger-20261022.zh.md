# 本轮交付 TDD 计划（2026-10-22 重排）

> 本文是本轮唯一执行台账。旧计划中的「已做/待做」不再引用，全部在此重新排列。
> 状态：⬜ 未开始 · 🟨 进行中 · ✅ 完成 · ⛔ 阻塞 · ⚠️ 已知限制（有意不修）
> 更新：每完成一项立即改状态并写证据，不写「应该通过」。

## 0. 当前基线（不再重复验证）

| 项 | 结果 | 证据 |
|---|---|---|
| client agent 全量测试 | 111 通过 / 70 失败 | 70 项全部为实验室门禁（COMPUTERNAME 必须 RDLAB-WIN10/11），与 HEAD 基线逐条一致 |
| client clippy（改动文件） | 0 命中 | `--bin rustdesk-management-agent --all-targets --no-deps -D warnings` |
| 后端 PHP 四套 | 25/30/76/86 断言 | 本机 php 8.5 |
| 后端 Python HTTP | 75 项 | 不含浏览器 |
| 后端 integration | 68 项（跳过 1） | 需 `FRANKENPHP` 环境变量 |
| 后端容器门禁 | 通过 | 构建/80 端口/管理路径/公开回退/重启持久化 |
| 后端前端套件 | 130 项 | 本机复跑，非采信子代理报告 |
| 四包矩阵 | std-msi / sos-msi / std-exe / sos-exe 各 10 PASS 0 FAIL | `/tmp/wgs/mx_*.log`：清理→安装→两服务失败动作→新 agent 哈希→后端新实例+generation→service.inspect 事件链→杀进程自愈 |

## 1. 待办（按优先级）

### T1 重装后重新登记（用户已确认必做）✅
后端在拒绝新实例时记录候选，管理页提供授权入口。

| # | 步骤 | 层 | 状态 |
|---|---|---|---|
| T1.1 | 写失败测试：status 409 时 `device_agent_candidates` 落一行（设备/新 installation_id/first_seen/last_seen），且 status 表仍保留旧行 | PHP 单测 | ✅ 先红（表不存在）后绿 |
| T1.2 | 写失败测试：enroll 403 时同样记录候选（first_seen 不变、last_seen 刷新） | HTTP 集成 | ✅ 并入 T1.5 |
| T1.3 | 写失败测试：已授权实例不再显示候选；旧实例回来只成为候选、不覆盖已授权状态 | PHP 单测 | ✅ |
| T1.4 | 写失败测试：候选不被当作授权，命令仍 403；撤销+授权后 generation 递增、审计 +2 | PHP 单测 | ✅ `agent_candidate_test` 20 断言 |
| T1.5 | 写失败测试：HTTP 层——真签名上报新实例→409 查库有候选；enroll→403；overview 暴露 candidate；授权后恢复。另加原子接管：无 CSRF 拒绝、并发 409 不改库、成功后 enroll 拿到新 generation | Python HTTP 集成 | ✅ 先红（overview 无 candidate 字段 KeyError）后绿；接管用例做变异验证（去掉并发比对→变红） |
| T1.6 | 实现：`agent_status.php` / `agent_operations.php` 记录候选（独立提交，不随 409 回滚） | PHP | ✅ |
| T1.7 | 实现：原子接管 `agent_binding_adopt` + `POST binding/adopt`；`agent_console.php` 列表 LEFT JOIN 候选；`agents.html` 列表徽标「待接管新实例」、详情「接管此新实例」按钮（确认框写明旧实例失权、代次+1、写审计） | PHP+前端 | ✅ 单测 33 断言（含接管 5 组：候选不符/并发/非法权限/成功/无候选） |
| T1.8 | 浏览器 E2E：桌面+390px；取消不写库、并发被拦截、确认后实例切换 generation+1 候选清空审计一条、新实例上报被接受、列表徽标消失 | Playwright | ✅ 2/2 |
| T1.9 | 回归：PHP 五套 + Python HTTP 77 + integration 68 + 前端套件 140 | 全量 | ✅ 全绿 |

### T2 远程发起的安装/升级/卸载完全静默，本地手动保持原样 🟨
规则（用户 2026-10-22 确认）：
- 本地用户手动（主界面检查更新、在线更新、手动安装/卸载）→ 成功与失败都照常通知（即现有行为不变）。
- 远程后端发起（后台下发更新命令/策略自动安装、Agent 命令、救援、首装、purge）→ **不论 EXE 还是 MSI、不论安装/升级/修复/重装/卸载，一律完全静默、不通知**（无 Toast/窗口/msiexec UI/托盘），成功和失败都一样。
- 静默不等于丢失：保留日志，并可经 Agent 远程调取。

| # | 步骤 | 状态 |
|---|---|---|
| T2.1 | 定位通知来源：`src/core_main.rs` 两处 Toast（`--update`、`--silent-install`）；MSI `LaunchApp` 仅在 `UILevel≠2` 时启动主窗口 | ✅ |
| T2.1b | 前一版实现（成功一律静默）与新规则冲突，已整体还原 `src/core_main.rs`（HEAD 无其他改动） | ✅ |
| T2.2 | 全量梳理入口 | ✅ 结论：Agent 的 MSI 安装/更新/修复/卸载与 EXE 事务已全静默且多数有 `/L*v`；违反项只有 4 处（见下） |
| T2.2a | 违规项：后台推送/策略触发的 EXE 升级结束时弹 Toast（成功与失败） | ✅ 已改 |
| T2.2b | 违规项：`--silent-install` 结束时弹 Toast | ✅ 已改 |
| T2.2c | 违规项：远程命令/策略在用户 Flutter 进程执行 → UAC/浏览器/主窗口/密码框/卡片 | ✅（Mac）安装版用户进程不跑后台 updater；V3 不拉起主窗口；V2 仅本地失败开浏览器；V5 判本地保留；无服务时非提权进程拒绝远程提权，上报 `failed/elevation_required_remote`、写日志、不重试。base 62/62 先红后绿；Windows 编译、Flutter 测试、真机冒烟待 win10-gs |
| T2.2d | 违规项：远程日志无法调取；两处 MSI 调用无日志 | ✅ 见 T2.4c |
| T2.3 | 来源标志 `--remote-silent`（`libs/base/src/update.rs`）；`update_is_remote(manually, has_command)`、`update_launch_args`、`is_remote_silent` 纯函数；标志插在 `--update` 之后、`--update-event` 之前（后者按位置解析，空 command_id 是末位，追加会被误读） | ✅ 3 条测试先红后绿，base 全量 56 通过 |
| T2.4 | 实现：`updater.rs` 仅 remote 时传标志；`core_main.rs` 两处 Toast 前判断 `is_remote_silent`，远程只写日志、本地路径零改动 | ✅ 代码完成，待 Windows 编译 |
| T2.4b | 远程日志命令 `diagnostics.logs`：后端注册表/参数校验/权限/控制台/页面/测试 | ✅ 后端 25 断言先红后绿；E2E 桌面+390px 2/2；注册表回归 92 |
| T2.4c | 远程日志命令客户端实现 + 两处 MSI 补 `/L*v` | ✅ win10-gs：模块 21/21、operations 集成 11/11、MSI lib 3/3，均先红后绿；变异 8 组被抓住；agent 全量 137 通过/66 失败（失败为基线 70 项子集，全是实验室门禁）；本地手动 MSI 命令行逐字不变 |
| T2.5 | 真机静默验证 | 🟨 取证结论：win10-gs 的“安装成功”通知**不是静默漏网**——由矩阵脚本直接执行裸 `--silent-install`（无 `--remote-silent`）触发，属本地路径，按规则应通知；两次 EXE 各弹一条、两次 MSI 零条，与两条代码路径一致。**验证方法必须改**：矩阵脚本与手工拼标志都不能证明远程链路，须由后端下发 install 命令走 Agent。另发现 session 0 的 Toast **能**送达会话 1，原脚本假设被证伪 |

### T3 F1 真机验收 ⛔
需要带修复的 CI 正式构建；当前生产只有 `2026101805`（旧代码）。前置完成后执行：下发同构建 agent.update → 命令达终态 → 后续 service.inspect 正常。

### T4 文档 🔄
- `docs/management-agent-delivery-plan.zh.md` Phase 4/5b 已补实测记录 ✅
- 两仓库交付文档（delivery-plan Phase 4/5b/5c、windows-exe §23、agent-console-plan 增量）已同步 ✅
- `release-notes.zh.md` 需在拿到正式构建号后再写（不编造版本）⬜

### T7 安全审查（后台本轮 diff）✅
结论：无严重/高。签名校验先于候选写入、候选表每设备一行、接管需全局管理员+CSRF、无注入/XSS、日志参数无法绕过。
| # | 项 | 状态 |
|---|---|---|
| M1 | 接管时 `expected=null` 被当作“不比对”，与授权接口语义相反，并发下可覆盖他人刚建的绑定 | ✅ HTTP 先红（200 覆盖成 gen 6）后绿 409；路由缺键 422；变异 3 组均变红 |
| M2 | `diagnostics.logs` 属 observe，后台关闭管理代理策略时仍可调取 | ✅ 用户 2026-10-22 决定：不禁止，保持与 `diagnostics.collect` 一致；本地 `disable-agent` 仍由客户端优先拦截 |
| L1 | 删除设备不清理候选行 | ✅ 单删/批删各加一行，变异 2 组均变红 |
| L2 | 候选写入失败被静默吞掉 | ✅ 只记设备 ID+异常类名，仍 409；变异变红 |
| L3–L6 | 接管不判断旧实例在线、省略权限时扩为全部、日志事件无保留期、候选无设备侧交叉证据 | ⚠️ 记为已知限制（需新状态/新规则） |
| L7 | 设备端/管理端无速率限制 | ⚠️ 既有问题，另行处理 |

## 2. 已知限制（有意不修，需维护者决定）
| 项 | 原因 |
|---|---|
| `package.reinstall/update` 工作区被外部删除时永久 `reconciliation_required` | 需要为包命令新增终态判定依据（新状态） |
| `devices.html` 948 行超 800 行上限 | 拆 JS 需改 PHP 静态资源路由 |
| devices/agents 用原生 `confirm()` | 48 处既有测试依赖 |
| 客户端页一次性加载全部设备 | 既有测试要求一次显示 206 行 |
| `setup.html` / `home.html` 未重构 | 未纳入本轮范围 |

### T5 本地构建与 CI 对齐 🟨
| # | 步骤 | 状态 |
|---|---|---|
| T5.1 | 发现：本地 SOS 构建漏设 CI 的 `RUSTDESK_SOS=1`（`flutter-build.yml:80`），本地 sos dll 不是真 SOS 构建（`windows.rs:1674` 据此写 agent `--edition`，sos-exe 装后自报 standard）。产品/CI 不受影响 | ✅ |
| T5.2 | 按 CI 环境变量重编 sos dll（`LIB_SOS_EXIT=0` / `LIB_STD_EXIT=0`），重打 SOS 包（`PACK_sos=0`），矩阵增加 edition 校验后重跑 | ✅ sos-msi / sos-exe / std-msi 各 13/13 PASS；dep-info 记录 `env-dep:RUSTDESK_SOS=1`，sos-exe 服务路径 `--edition sos` |
| T5.3 | 现有本地 standard 包编译时漏了 `vram` feature（CI 为 `hwcodec,vram,flutter`），体积少约 900KB | ⚠️ 只影响本地实验包，产品/CI 不受影响；是否重打待定 |

### T6 审查 rustdesk-api 最终 diff 的中优先级问题 ✅
6 项全部处理，完整前端套件 **152 项通过**；其中「取消确认」时序漏洞用 CDP 400ms 延迟做了变异证明（旧 helper 通过、新 helper 失败）；「全部」按钮作用范围用变异法验证。admin.spec.js 本机无法运行，只做 `node --check` 与静态核对，已如实标注。
**admin.spec.js 补跑（2026-10-22）**：容器补齐 `RUSTDESK_UPDATE_KEYS_JSON`（与 `integration_test.py` 同一测试 key）后，与 HEAD 基线容器对照：
- 基线 HEAD 页面跑 HEAD spec 也失败 2 项（用户删除改为对话框、通讯录备注移出列表行），属 spec 落后于页面，已按现页面改 spec（删除走 `#delete-dialog`，备注改在详情「通讯录身份」里断言）。
- 新发现 1 项真实缺陷：初始化时客户端库分页加载较慢，期间保存的「通讯录已保存」会被随后的首次加载覆盖成「已加载 N 个」。设备越多越容易出现（1240 台时 6/10 失败）。先写 E2E 用路由延迟复现（红），再把首次 `load()` 改为成功提示锁定时安静加载（绿），基线页面同样失败，说明不是本轮重构引入。
- 结果：`admin.spec.js` 57/57（串行），`installer-generator.spec.js` 2/2（需本地起 18080 静态服务）。
- `ui_functional_e2e_test.py` 中我写的日志用例按 `created_at,command_id` 断言先后，同秒创建时顺序随机，改为按集合比较；24 项通过。
- 后端 Python 套件全部通过（需 `AGENT_TEST_PHP`/`AGENT_TEST_PHP_INI`；`integration_test.py` 需 `FRANKENPHP=$(command -v php)`，68 项跳过 1；`provisioner_test` 需 `python3 -m unittest`）。

审查结论：无严重/高。中优先级待处理：403 前缀误导排障；取消确认断言时序漏洞；「全部」按钮作用范围未查库；通讯录两个新确认缺取消路径；缺 CSRF/授权失败场景；撤销按钮嵌在 `<summary>` 内。

### T8 客户端代码审查（本轮 diff）✅
结论：无严重。路径穿越/重解析点/大小上限/错误外泄/Tokio/unwrap/日志节流均通过。
| # | 项 | 状态 |
|---|---|---|
| H1 | 本轮删掉了 HEAD `586beafc7` 的“agent.update 已到目标构建即成功”兜底，工作区仍在但证据损坏时会永久卡住 | ✅ 恢复兜底（与 HEAD 逐字一致），PHP 往返测试先红后绿；未达目标仍 reconciliation_required |
| M1 | MSI 日志与 `--update` 日志同目录，`update` 源会读到 `msi-update.log` | ✅ 改为 `log/msi-update/msi-update.log`，断言 update 源不返回 MSI 日志 |
| M2 | 本地手动 MSI 更新也被加上 `/L*v`，违反“手动零改动” | ✅ 远程走 `update_me_msi_remote`，手动 `update_me_msi` 与 HEAD 逐字一致，测试断言 |
| M3 | `--silent-install --remote-silent` 当前无生成方，静默分支未接通 | ✅ 核实：远程链路（Agent EXE 只解压不安装、MSI 均 `/qn`、救援/首装/修复只登记服务）都不走 `--silent-install`，无需追加；便携包会原样转发标志，静默分支保留为防御入口。补回归测试 2 条（portable 11/11、defender_options 3/3），变异验证变红 |
| M4 | 无命令的自动更新（`manually=false`）也被静默 | ✅ 符合用户“远程/策略一律静默”规则，记入回归面 |
| L1 | 失败动作线程超时后要等 agent 重启才重试 | ⚠️ 干净失败，已知限制 |
| L2 | 线程 spawn 失败使服务启动失败 | ⚠️ 回归面说明 |
| L3 | SOS 服务名需确认为 `RustDesk` | ✅ sos-msi/sos-exe 矩阵两服务失败动作 PASS |
| L4 | 脱敏关键词缺 bearer/api_key/access_key/otp | ✅ 先红后绿 |
| L7 | `recovery-msi.log` 远程无源 | ✅ 客户端已加 `recovery-msi` 源；后端注册表与 agents.html 下拉同步，测试先红（422 日志源无效）后绿，26 断言 |

Mac 门禁：`base` 新测试 fmt/`non_fmt_panics` 两处本轮问题已修，56/56；其余 fmt/clippy 失败与 HEAD 一致，主 crate 与 Windows 文件需在 win10-gs 补。

### T9 后台 §10.3 全量门禁 ✅（含 T7 修复后的工作树）
- 静态：`php -l` 6 文件、`compileall`、`git diff --check`、`docker compose config`（注入临时 secret）、`bash -n` 6 脚本，全部 0。
- PHP 6 套：54/92/25/76/25/30 断言；Python 333 项（integration 68 跳过 1）；stream-broker `go test` 11。
- 新镜像容器：内部 80、`/ops-x9` 200、公开 fallback 正确、重启后用户持久化（API 与 SQLite 一致）；`admin.spec.js` 57/57，`installer-generator.spec.js` 2/2。
- 未覆盖：POSIX/Shell suite 只做了语法检查（本机无 Linux）；MySQL 冻结未跑。

### T10 runbook §10.5 剩余总门禁（客户端代码冻结后执行）⬜
| # | 步骤 | 状态 |
|---|---|---|
| T10.1 | win10-gs：主 crate 测试 / clippy / fmt（只认本轮新增命中） | ✅ 第三轮（含 T13-A1）：22 文件哈希一致；base 76、updater 25、ui_interface 2、activate_os 3、operations 11、portable 14 全过；agent 失败集合与 r2 一致；check 告警 106 无新增；fmt/clippy 改动行 0；Python 新文件 GBK 读取问题已改 `encoding="utf-8"` |
| T10.2 | win10-gs：`flutter analyze` / `flutter test` | ✅ 第二轮：analyze 问题集合与 HEAD 相同；全量 370 通过 |
| T10.3 | 用最终代码按 CI 全流程重打四包 | ✅ `D:\t10\pkg-2026102301\`：std EXE `889C777A…3BC7` / std MSI `EC796775…4010` / sos EXE `6FA46525…2006` / sos MSI `47C545A7…8CA9`；两 edition dll 哈希不同且 MSI 解包一致；`ServiceInstall`/`bootstrap.json` edition 正确；manifest 无 dpiAware/PerMonitorV2；`verify4.sh` 32/32；耗时 2h45m。**偏差**：冒烟阶段一度按进程名误杀用户正在运行的 RustDesk（SCM 5s 内自愈，未损安装目录），已改为仅按 stage 路径匹配 |
| T10.4 | 四包全新安装矩阵（含 edition 校验） | ✅ 四包各 15 项全 PASS（60/60）；sos 包实测 `--edition sos`，`agent.json` target key 四个包各自正确；失败动作、agent 哈希、后端 build/edition/kind、事件链、双服务 6s 自愈均通过。偏差：恢复脚本两次失误（备份文件 Hidden+System 属性导致静默失败；`robocopy /MIR` 覆盖 ProgramData DACL）已修复并回到矩阵前状态（installation_id 不变、generation 11→16）。备份脚本扫描目录需改递归（RustDesk*.toml 在 `config\` 子目录，本次备份数为 0 但身份未变） |
| T10.5 | 四包旧版本升级路径 | 🟨 暂停于 15:45（让位开启 Defender），续跑步骤见 `/tmp/wgs/t10/t10.5-result.zh.md`「暂停点」。std-msi 58 PASS / 4 FAIL，4 FAIL 为脚本只认双引号的解析错误（配置用单引号；enc_id/salt 实际存在，password 为空串是固定密码预置的设计），身份用非空判据离线重算全 PASS，另有 5 项空对空 PASS 不计数。std-exe 第 4 次尝试中断时 57 PASS / 10 FAIL：身份/配置首次真实比较全 PASS、远程升级窗口 Toast 0；**待定性 4 组**：EXE 远程升级后 ProgramData agent 仍为旧构建 2026101805、两服务无 SCM 失败动作、杀进程 45s 不自愈、后端 build_seq 仍为旧值，另主程序载荷为新包但文件版本资源显示 2026101805，可能为产品缺陷，续跑先只读取证。sos-msi / sos-exe 未开始。右下角「安装失败」通知 5 条（ID 668–672）全部来自旧包本地 `--silent-install`，规则内；连续失败原因为清理脚本遗留公共桌面 `RustDesk.lnk`，旧安装器按设计拒装且退出码仍为 0，已修清理脚本。续跑前须修 `task_ps` 按 ANSI 读 UTF-8 结果（已修，见 T23 X0） |
| T10.5a | std-exe 第 4 次尝试的 4 组 FAIL **已定性为产品缺陷**（2026-10-23 16:10，只读取证）：不是脚本误判，也不是「agent 自升级未完成」。根因见 T23 X1。sos-msi / sos-exe 仍按原计划续跑 | ✅ 定性完成 |
| T10.6 | R0–R6、agent 自身与事务中断故障注入 | 🟨 守卫已替换并在 win10-gs 验证：82 文件哈希一致；全部测试目标、两个独立 crate、`.cpp` 编译通过，16 个 `.ps1` 语法通过，本轮改动行告警 0；不设 `RUSTDESK_LAB_HOST` 时 66 项仍被拒绝（62 项提示已换成新守卫）；设 `YAN` 后低风险项通过（lab_guard 8、purge_marker 6、purge_preflight 7（需串行）、package_download 22、installer_busy 12、契约 5）；exe_assets/msi_metadata 缺 `C:\RustDeskAgentLab` fixture。前后两服务 Running、主程序哈希不变。会停服务/重装的高危项未执行，待备份后逐项安排。**备份/恢复/看门狗脚本（2026-10-23，静态完成，未上机）**：`/tmp/wgs/t10/inject_{backup,restore,watchdog}.ps1`，状态见 `inject-scripts-status.zh.md`。修掉 T10.4 两次失误根因（`robocopy /MIR` 覆盖 DACL、Hidden+System 写不进），改为逐文件哈希/属性 + `icacls /save`/`/restore`、toml 备份数为 0 即失败；修 `sc failure`/`binPath` 引号参数错误；恢复时只把 `agent.json` 启动代次推到现场值，避免后端 409；看门狗会话 ≤180 分钟、默认每会话最多恢复 1 次、失败即升级人工。未解决：未在 VM 跑过；本地命令/事件/事务回滚可能重放；双服务停 120s 即恢复短于产品自愈窗口，相关用例须先 `pause.json` 或调大阈值；`inject_restore.ps1` 891 行。上机验证 7 步（语法→基线备份→空跑恢复→5 项定向演练→observe→guard→截止）全过后才执行高危用例 |
| T10.7 | T2.5 真机静默验证 + 远程调回日志 | ⬜ |
| T10.8 | 后台 SQLite 与 API/页面/agent state/事件/outbox 核对 | 🟨 脚本 `/tmp/wgs/t10/t10_8_check.sh`（后端 B01–B22、VM V01–V14），自测 55 例全过，变异（V08/B15 恒 PASS）5 例变红。后端只读核对 30 PASS / 0 FAIL / 4 WARN / 1 SKIP：binding gen17 授权且 7 项权限、status build 2026102301 standard/msi、overview 与页面数据与 SQLite 一致、34 条命令全终态、91 条事件无孤儿、`integrity_check=ok`。WARN：生产库缺 `agent_command_acknowledgements` 表（生产代码早于 rd-api 当前仓库，非数据损坏）；候选表残留一行与已授权实例相同（页面已过滤）；第二次采集时 T10.5 正在重装，需结束后重跑。VM 侧（V01–V14，ps1 语法未经 VM 验证）待 T10.5 结束后执行 |
| T10.9 | 最终代码审查、`git diff --check`、证据清点、临时资源清理 | ⬜ |

### T11 本地构建与 GitHub CI 一致性 🟨
| # | 项 | 状态 |
|---|---|---|
| C1 | 阻断：preflight 契约测试 `tests/test_release_and_remote_ui_contract.py` 仍断言旧字面量 `"{} --update {}"` | ✅ 先红（1 失败）后绿；改为断言 `update_launch_args` 与 `--update` 首位；仓库 Python 契约 128 项通过（跳过 2） |
| C2 | 阻断（提交时）：`diagnostic_logs.rs` / `diagnostic_logs_tests.rs` / `tests/test_check_on_startup_global_option.py` 未纳入 git | ⬜ 提交时必须一起 add；`.mcp.json`、`opencode.json`、`.grok/`、`aoci*` 等本机文件不得 add |
| C3 | CI Windows 用 Rust 1.75，本地只验证 1.90；静态扫描未见 1.75 后才稳定的 API | ⚠️ 用户 2026-10-22 决定：不在 win10-gs 安装 1.75；以静态扫描为准，首次 CI 构建即为 1.75 实测，失败则按构建修复约束最小修复 |
| C4 | `signed_rust_command_roundtrip_*` 缺 PHP 时 panic 而非跳过（既有，本轮扩大） | ⚠️ CI 不在 Windows 跑 cargo test，不影响发布；记为已知限制 |
| C5 | Flutter 改动需 `flutter analyze` + `verified_update_entry_test.dart` | 🟨 win10-gs 门禁中 |
| C6 | 本地打包复用旧 Flutter 目录，dart 改动不会进本地包；`pack4.bat` 等未设 `RUSTDESK_EDITION/RUSTDESK_SOS` | ⬜ T10.3 重打包时按 CI 全流程（含 `flutter build windows`）与环境变量执行 |
未改动：无新依赖、`Cargo.lock` 不变、代码不读任何本地路径/新环境变量、各平台 cfg 完整（Mac `cargo check` lib/agent/tests 通过）。

### T12 后台发布 0.1.41 ✅
| # | 步骤 | 状态 |
|---|---|---|
| R1 | 发布前门禁（精确工作树）：PHP 6 套、`php -l`、`git diff --check`、新镜像容器 + setup、`admin.spec.js` 57/57、`agents_page_e2e` OK、integration 68 跳过 1 | ✅ |
| R2 | `VERSION` 0.1.41、`release-notes.zh.md`；只暂存 18 个业务/测试/文档文件（本机 `.mcp.json`、`aoci*` 等未加入）；提交 `1dea15a` 通过 commit-msg 校验 | ✅ |
| R3 | 推送 `origin/main`、tag `v0.1.41` | ✅ |
| R4 | ACR 工作流：`rustdesk-api:0.1.41`、`rustdesk-api-provisioner:0.1.41` 均有 amd64/arm64 清单 | ✅ |
| R5 | 生产 `NTServer-SH` 升级：第一次 `upgrade-api.sh` 卡在 `compose pull`（香港仓库拉取极慢）约 45 分钟，我中止拉取后单独 `docker pull` 完成，再重跑 `./upgrade-api.sh`；19:36:57 升级成功，旧容器期间持续服务 | ✅ |
| R6 | 核验：容器镜像 revision `1dea15a`（= 0.1.41 提交）、running、重启 0 次、`127.0.0.1:7000->80`；本机 `/` 与 manifest 200，公网 `https://rdapi.yan.life/` 与 manifest 200；近 10 分钟日志无 fatal/PDO/SQLSTATE；新表 `device_agent_candidates` 已建、`PRAGMA integrity_check=ok` | ✅ |

### T13 用户进程更新门禁二次审查 ✅
| # | 项 | 状态 |
|---|---|---|
| A1 | 高：本地“启动时检查/自动安装”开关存用户 LocalConfig，服务进程读不到；策略变更事件到不了界面 | ✅（Mac）方案 1：`enable-check-update` 改存全局 Config 经 IPC 同步；界面读写在 Rust 侧路由，Flutter 不改；旧用户值首次同步后迁移并清除。base 75、updater 28、Python 契约 131 通过（新增 `test_check_on_startup_global_option.py` 先红后绿）。回归面：开关由每用户一份变为整机一份。已知：升级后界面未打开前，服务仍按关闭处理；既有问题——非定制客户端 Flutter 把“开”存为空串而后台只认 `Y`（未改动，另行处理） |
| A2 | 高：界面修改定时更新，服务端调度不重算 | ✅（Mac）服务侧 IPC 调度值变化时通知循环；纯函数测试先红后绿 |
| A3 | 中：已安装但服务被停用时无人执行远程/策略/定时 | ✅（Mac）按“服务已注册”判定，避免服务重启间隙重复执行；`is_self_service_installed` 待 Windows 编译 |
| A4 | 中：macOS daemon 不初始化 TX_MSG | ✅（Mac）初始化 + 进程内只消费一次结果；真机待验 |
| A5 | `D:\github\rustdesk`（win10-gs 残留空壳，仅 `.spec-workflow`） | ⬜ 待清理；本轮不改 |
| A6 | 用户新增项目约束：每次修改内容都要在收尾做一次 AOCI 对齐 | ✅ 已写入 `AGENTS.md` 第 6 条（含 `aoci verify` 相关项清零、`aoci check` 非 blocked、MCP 不可用时如实报告） |
| A7 | 336 项作者化 | 🟨 已交专项 agent，按批次 `aoci_maintain`→`aoci_update_entry` 写入；先重写 `src/` 14 条模板条目；`README.md` 为 0 字节，条目按现状更新 |
| A6 | 中：并发手动检查误清本地安装标志 | ✅ 本地安装检查 `request_id` 为空、设置页检查带 `manual-<时间>`，按此区分；本机 Flutter 3.24.5 测试 13/13 |
| A7 | 低：拒绝提权推送本地 UI failed 事件 / 用 AwaitingInstallResult 表达拒绝 / 同义反复测试 / 契约测试补启动行 / 全局可变标志 | ✅ 新增 `Refused`；拒绝不推本地 UI，非命令来源每进程只报一次（重启后再报一次，已知限制）；base 66、updater 28、契约 22 通过 |

### T14 安卓控制 Windows 连接瞬间左上角弹出右键菜单 ✅
| # | 步骤 | 状态 |
|---|---|---|
| T14.1 | win10-gs 今晚连接日志 + 源码取证，区分控制端发出还是被控端误处理 | ✅ 控制端发出：连上 1.5 秒无首帧时 `model.dart:1310` 调 `sessionInputOsPassword('')` → `client.rs` `activate_os`，密码为空时移到 (3,3) 并右键点击。今晚 7 次连接首帧均晚于 1.5 秒（1.8–2.8 秒），每次都触发 |
| T14.2 | 先写失败测试复现（连接初始化不得产生右键/0,0 事件） | ✅ 新纯函数 `activate_os_mouse_events`，照搬原逻辑时测试红（序列含右键 17/18） |
| T14.3 | 最小修复 + 回归 | ✅（Mac）无密码只移动不点击，有密码左键序列与节奏不变；`activate_os_` 3/3，`cargo check --lib` 通过。已知局限：锁屏且无自动密码时可能需手动点一下才出密码框；Windows 编译待 win10-gs |
| T14.4 | 真机：安卓连接 win10-gs 不再出现右键菜单 | ✅ 用户 2026-10-22 实测：右键菜单不再出现 |
| T14.5 | 残留：唤醒逻辑把被控端光标移到左上角；排查各端同类隐患 | ✅（Mac）无密码时一个消息都不发（不移动、不点击），有密码仅左键+密码+回车的点击落在被控端当前光标处；唤醒由被控端自身机制承担（Windows 相对微动+ES_DISPLAY_REQUIRED、macOS caffeinate、Linux keepawake/DRM），未改。被控端隐患：Windows 光标被拖走、macOS 触发左上热角、Linux 副屏在左时跨屏跳。Flutter 首帧/恢复画布/视图刷新三处也不再迁就被控端光标（显式切显示器保留）。验证：activate_os 6/6、client:: 53、flutter 新测试 4/4、全量 flutter test 374 通过。残留：`remote_toolbar.dart:3549` 直接调 refreshMousePos 的小概率路径未改（超出本轮授权） |
| T14.6 | 重打安卓测试包（含光标修复） | ✅ 新包 `rd-localtest-1.5.3-cursor-fix-arm64.apk`，SHA-256 `ca04510b…85f6`；用户实测：右键菜单问题已解决、功能正常。用户观察到的左上角鼠标是安卓端自己绘制的本地光标，不影响被控端，用户决定不改、就此关闭 |

### T15 故障注入清单暴露的 runbook 与代码不一致（只记录，待维护者决定）⚠️

**文档已按现状对齐（2026-10-23 18:30）**：`agent-recovery-test-runbook.zh.md` 10 处（F1–F8、N2/N3/N7）、`agent-recovery-upgrade-plan.zh.md` 6 处内容 + 6 处 VM 术语、`agent-recovery-macos-handoff.zh.md` 2 处术语。只改文档，未动 `.rs`/`.ps1`/`.sh`。新增 N7 警告：`cfg(test)` 互斥名 `Global\RustDeskAgentTestInstallGuard` 与生产不同，真机测试前须先停生产自愈或置 `maintenance.json`。

**逐条核实（2026-10-23，`/tmp/wgs/t10/t15-verify.zh.md`，只读）**：F1–F7 属实，F8 部分属实（无进程锁，但 `--service` 仅 SCM 可启、计划任务 `IgnoreNew`，双实例现有入口不可达）。F2 比原记录更严重：有 `rescue\current` 时自动 R5 整体跳过（N1）。处理类别：F1、F8 只改文档；F2–F7 需新增状态/规则，按 Corner cases 须维护者批准，批准前在 runbook 标为已知限制。新增发现 N1–N7：N1 有 bundle 时自动 R5 不跑；N2 R1/R2 只认 SCM Running 不认主程序 ready；N3 agent 服务被禁用时 bootstrap 静默成功；N4 旧 R5 的 `last_error` 不上报；N5 自动 R4 退避预算重启清零；N6 implementation-status 说明过时；N7 `cfg(test)` 安装互斥名与生产不同，VM 真机测试与生产 R1–R4 不互斥，可能污染故障注入结论（T10.6 执行前必须处理或规避）。
| # | 项 |
|---|---|
| F1 | R1 门禁：agent 服务每次启动自动写 `service-recovery-enabled`（`src/management_agent.rs:1673`），旧文档“默认关闭需手建”已不成立 |
| F2 | R5 自动包恢复只用 `target.primary`，404 直接退避，无镜像回退（镜像回退仅在远程命令下载路径） |
| F3 | R6 无独立熔断状态与告警事件，只是固定 3600s 重试 |
| F4 | R3/R4 认不出“文件在但内容被篡改”；R4 与 agent 备份恢复只支持 MSI，EXE 包无此路径 |
| F5 | agent 文件损坏（非缺失）时 bootstrap 拒绝启动且不替换 |
| F6 | 未实现：检查循环停滞进度超时；禁用 Task Scheduler 后上报 degraded（`recovery_entry` 恒 `not_assessed`） |
| F7 | 本地恢复事件只有 R1/R3/R4；R0/R2/自动 R5/R6/bootstrap 不发事件 |
| F8 | 无 agent 进程级单实例锁，只有安装互斥 |

### T16 后端仓库改名 rd-api ✅
- GitHub `jandoucn/rustdesk-api` → `jandoucn/rd-api`（旧地址自动跳转）；Mac `/Users/olly/github/rd-api`、win10-gs `D:\github\rd-api`，两边 remote 均已改新地址。
- 同步更新：Mac 后端 `.mcp.json`/`opencode.json` 路径、win10-gs `gate.bat`/`gate2.bat`/`gate3.bat`、客户端 5 份文档链接、`operations_runtime.rs` PHP 往返测试默认后端目录 `rd-api`；临时符号链接已删除。
- 未改：生产 `/opt/1panel/apps/rustdesk-api`、容器 `rustdesk-api`、ACR 镜像名。

### T17 AOCI 对齐 🟨（专项 agent 执行中）
| # | 项 | 状态 |
|---|---|---|
| A1 | `.mcp.json` 的 aoci 服务原指向 `/Users/olly/github/rustdesk`（该目录已不存在） | ✅ 改为本仓库；MCP 需重开会话生效 |
| A2 | `aoci.code.txt` 165 个分节标题仍写旧绝对路径 | ✅ 已改为 `/Users/olly/github/rd-client`；结构仍有效 |
| A3 | 漂移：missing 215（159 非归档新文件）、stale 131、unbaselined 215、observed_new 151、skipped 72，共 635 finding | 🟨 需经 `aoci_rules`/`aoci_overview`/`aoci_maintain` 逐批作者化；不用 `scan --force` 洗白 |
| A4 | 文档/配置里残余旧路径（`docs/*.md`、`opencode.json`） | ✅ 已改 |
| A5 | `D:\github\rustdesk`（win10-gs 残留空壳，仅 `.spec-workflow`） | ⬜ 待清理；本轮不改 |

### T19 静默验证方法修正（取证结论）🟨
| # | 项 | 状态 |
|---|---|---|
| B1 | 本次通知来源：`src/core_main.rs:373-410` `--silent-install` 分支，`is_remote_silent` 为假 → `Toast`（文案 `Installation Successful!` → `安装成功!`，AUMID 为 PowerShell，与 `Toast::POWERSHELL_APP_ID` 一致） | ✅ 证据闭合（通知库残余字节 + WPN 3052/3055 事件 + 逐毫秒时间对齐） |
| B2 | MSI 路径不弹：`LaunchApp` 被 `NOT UILevel=2` 跳过，`LaunchAppTray` 只起托盘 | ✅ `mx_msi.log` 实证 |
| B3 | `silent_check.ps1` 的“session 0 的 Toast 会被丢弃”假设错误 | ✅（静态）已重写为只采集证据：通知库连同 `-wal`/`-shm` 复制，`CurrentNotificationId` 不增长且无 PowerShell/RustDesk 新通知；每 0.5s 采样新增可见窗口（msiexec/RustDesk 及其子进程、`consent.exe`）；托盘新出现判 FAIL。判定移到 `silent_judge.py`，4 组构造数据通过。真机语法与运行待 VM |
| B4 | `silent_one.sh` 的 EXE 用例手工拼 `--silent-install --remote-silent`、watch 用 schtasks 本地跑 → 未验证后端→Agent→安装真实链路 | ✅（静态）手工拼标志与 schtasks 用例已删；全部经 `ops_t10.php` → `agent_command_create` 下发（restart/repair/MSI reinstall/可选 update、agent.update），每例用 `diagnostics.logs` 取回对应日志并校验时间窗；本地 `--update` 为对照组（期望有通知，未出现则把静默 PASS 降为 SKIP）；EXE 只能测服务类命令加 reinstall 负例（期望 409）。未覆盖：新构建未发布时包类命令只能 SKIP（发布到生产需用户批准）；后端无远程卸载命令；EXE 远程升级走客户端策略 `--update --remote-silent` 不经命令表；救援/首装/purge 不在注册表。待 VM 执行，四包约 2–2.5h |
| B5 | 潜在缺口（非本轮泄漏）：远程 `--silent-install` 时 `run_after_run_cmds` 仍会启动 `--tray`（`src/platform/windows.rs:4171-4184`，无对应开关；MSI 用 `LAUNCH_TRAY_APP=N`） | ⚠️ 记录：当前无生产路径构造 `--silent-install`，仅远程 EXE 场景才需处理 |
| B6 | 通知库读取必须同时复制 `-wal`/`-shm` 并比对 `Metadata.CurrentNotificationId` | ✅ 已写入修正后的判据 |

### T20 测试机约束：VM 停用，全部改在 win10-gs 🟨
用户 2026-10-23 告知：win10-gs 上已没有 VM，所有原 VM 测试都在 win10-gs 执行。
| # | 项 | 状态 |
|---|---|---|
| V1 | `AGENTS.md`：并行协作节「每台 VM」改为 win10-gs 单执行负责人；测试与验收约束新增「Windows 真机测试统一在 win10-gs」（无快照→完整备份+空跑恢复+看门狗、仅 termark SSH 带外、保护 `D:\build-env`/`D:\github`、测试构建与生产 agent 互斥名不同须先规避、无恢复手段记 BLOCKED） | ✅ |
| V2 | `docs/agent-recovery-test-runbook.zh.md` 现行条款中隔离 VM/快照全部改为 win10-gs/备份标签 | ✅ |
| V3 | Defender 真实写入：用户 2026-10-23 授权在 win10-gs 开启 Defender 并实机测试。只读探测：WinDefend/WdNisSvc/SecurityHealthService/Sense 均 Stopped+Disabled，策略 `DisableAntiSpyware=1`，`DisableRealtimeMonitoring=true`，`Get-MpComputerStatus` 报「无法执行外部方法」，LTSC 10.0.17763，COMPUTERNAME=YAN。开启需删策略、恢复服务启动类型并可能重启，排在 T10.5 之后；开不起来如实报告 | 🟨 |
| V3b | 开启进度：已备份到 `D:\build-env\defender-backup-20261023\`（9 项服务 .reg、策略、`before.json`），恢复脚本 `restore-defender.bat` 同目录；预配置完成：删除 `DisableAntiSpyware` 策略，WdBoot/WdFilter=0、WinDefend/wscsvc=2、其余=3，逐项读回一致。未见本地组策略或关闭工具会写回；机器装有火绒（Huorong sysdiag），可能使 Defender 进入被动模式。待用户重启 | 🟨 |
| V4 | `docs/agent-vm-system-driver.zh.md` 加停用说明，历史内容不改 | ✅ |
| V5 | `scripts/guest-*.ps1` 守卫迁到 `RUSTDESK_LAB_HOST` | ✅ 契约测试扩到 `scripts/`（新增「COMPUTERNAME 判断须伴随 RUSTDESK_LAB_HOST」与「scripts 至少扫到 30 个文件」）先红（4 用例 47 条）后绿（7/7）；15 个脚本各改 1–2 行，原锁 Win10 的 6 个加 `RUSTDESK_LAB_OS=win10` 可选；SYSTEM SID 校验保留；变异（改回 `-notlike 'RDLAB-*'`）3 用例变红；Python 契约全量 136→138，唯一失败 `test_msi_script_embedding_is_single_source_and_roundtrips_unicode` 改动前即存在。`guest-agent-acceptance-boot.ps1` 读取 SMBIOS UUID 仅作同机证据比对，不是准入守卫，保留。两份 `scripts/*.zh.md` 现行规则已改。未在 win10-gs 实测（未设变量拒绝、设置后放行） |
| V6 | 本轮 VM 字样的历史报告/交接文档不改写 | ⚠️ 保留为历史 |

### T21 Defender 排除项补全（T10 完成后执行）⬜
执行安排（用户 2026-10-23）：当前验收照常进行；全部验收完成后再修改代码、做代码审查，然后只做简单测试（mock 单测 + 一次实际安装读回排除项），不重跑全套矩阵。
用户 2026-10-23 决定：范围扩为整个 `C:\ProgramData\RustDesk` + 3 个 exe 路径 + 3 个进程排除；在 T10 全部完成后再改，不打断当前验收。
| # | 项 | 状态 |
|---|---|---|
| D1 | 现状：`res/defender-exclusions.ps1` 只加 2 个 exe（主程序、`rustdesk-management-agent.exe`）+ 3 个目录（安装目录、`ProgramData\RustDesk\ManagementAgent`、`ProgramData\RustDesk\Client`）；CFA 开启时 2 个 exe 加允许应用 | ✅ 核实 |
| D2 | 缺口：`ProgramData\RustDesk\RecoveryBootstrap\rustdesk-recovery-bootstrap.exe` 及目录不在范围（被隔离会断开 agent 自救链）；更新包下载到 `%TEMP%`/`C:\Windows\Temp` 随机子目录不在范围 | ✅ 核实 |
| D3 | 现状默认：EXE 安装页 `defender=false`（`install_page.dart:70`），MSI `DEFENDER_EXCLUSIONS=inherit`，无保存选择时不勾、不写；静默/远程安装无参数沿用保存选择，全新机器不写 | ✅ 核实 |
| D4 | 实现（TDD）：目录改为整个 `ProgramData\RustDesk`（替代 ManagementAgent/Client 两项）+ 安装目录；exe 路径加 bootstrap；新增 `ExclusionProcess` 3 项；CFA 允许应用加 bootstrap；更新文档、mock 测试、MSI CA 参数 | ⬜ |
| D5 | 用户 2026-10-23 决定：不再显示配置项，默认启用。EXE 安装页（`install_page.dart` / `defender_install_option.dart`）与 MSI 目录页（`Defender.wxs` 的 `DEFENDER_CHECKBOX`）移除该选项；首装、升级、修复、静默与远程安装一律配置排除项，不再依赖 `Consent`；已保存 `Consent=0` 的旧机器升级后同样配置。`defender=`/`DEFENDER_EXCLUSIONS=` 命令行参数与 `Consent` 注册表的去留、文档与 mock 测试同步改写，实现时一并按 TDD 处理 | ⬜ T10 后执行 |

### T23 EXE 覆盖安装不升级管理代理（T10.5 定性结论）🟨
2026-10-23 只读取证（win10-gs 现场文件时间戳 + SCM 事件日志 15:20–15:40）定性：std-exe 升级后 4 组 FAIL 是**同一个产品缺陷**，不是脚本误判。
| # | 项 | 状态 |
|---|---|---|
| X1 | 根因：`src/platform/windows.rs` `get_management_agent_install_commands`（HEAD `82deb46c9` 引入，非本轮改动）里 `copy /Y 源 目标` 排在 `sc stop` **之前**。EXE 覆盖安装时旧 agent 服务正以该目标文件运行，Windows 上 `copy /Y` 覆盖正在执行的文件返回共享冲突，但脚本非 `cmd /c` 上下文执行、又不检查 errorlevel，失败被静默吞掉；随后 `sc config`/`sc start` 只是把**旧二进制**重新拉起。证据：升级后 `C:\ProgramData\RustDesk\ManagementAgent\rustdesk-management-agent.exe` 仍是旧包哈希 `6AE671E7…` 且 mtime 15:21:34（升级前一小时），而同目录新包载荷 `C:\Program Files\RustDesk\rustdesk-management-agent.exe` 哈希 `8FA8AC58…` mtime 15:26:01；SCM 7045 在 15:21:37 只重建了 `RustDesk Service`，**没有**重建 `RustDesk Management Agent`。全新安装不复用旧 agent，因此 T10.4 四包矩阵 60/60 未暴露 | 🟨 待 TDD 修复 |
| X1a | 连带解释另外 3 组 FAIL：后端 build 仍 `2026101805`（旧 agent 上报）、两服务失败动作 `none`（旧包 2026101805 早于 `586beafc7` 的 `ensure_service_failure_actions`）、杀进程不自愈（无失败动作）。**不需要**额外修失败动作逻辑 | ✅ 分析闭合 |
| X1b | 待查：15:35/15:38 服务是谁拉起的（旧 agent 的 R1 自愈或计划任务） | ⬜ 定性非必需 |
| X2 | `RustDesk.exe` 文件版本资源显示 `1.5.3+2026101805`：本地打包未传 `--build-number`，继承 `flutter/pubspec.yaml` 的 `version: 1.5.3+2026101805`；CI 用 `--build-number "$BUILD_NUMBER"` 覆盖。属本地实验包偏差（同 T5.1/T5.3），产品不受影响 | ✅ 定性；不改代码 |
| X3 | 顺带缺陷：`get_management_agent_install_commands` 不检查 `copy` 是否成功、`sc start` 后不校验实际运行的是否为新包；修复须同时补上可验证的失败路径 | ⬜ 并入 X1 |
| X0 | 测试脚本两处隐患已修（2026-10-23 16:05，未在真机复跑验证）：`t10_lib.sh:72` 读结果 JSON 改 `Get-Content -Raw -Encoding UTF8`；`t10_lib.sh`/`mx_t10.sh`/`mx_one.sh` 的 `schtasks /create` 由 `/st 23:59` 改 `/sd 2000/01/01 /st 00:00`，消除当晚 23:59 重跑（对应 T22 S3 的这部分） | ✅ 已改 |
| X4 | 用户 2026-10-23 16:20 决定：X1 修复后**只重跑 std-exe 升级路径 + 一次全新安装冒烟**，不重跑 60 项四包矩阵（矩阵走全新安装，本来测不出该缺陷） | ✅ 范围确定 |
| X5 | 用户 2026-10-23 16:20 决定：X1 改动须在 win10-gs 验证"测试先红后绿"之后才提交 master；未验证前不落库 | ✅ 提交门禁 |
| X6 | 重跑前置已就位：新包 `/tmp/wgs/t10/newpkg/rustdesk-1.5.3-2026102301-*-windows-x86_64.{exe,msi}`（std-exe SHA-256 `889c777a…3bc7`，与 T10.3 记录一致）；旧包 `/tmp/wgs/dl_pkgs/*-2026101805-*` 四个齐全；驱动 `/tmp/wgs/t10/run_all.sh` 现写死 `AGENT_SHA=8FA8AC58…B5A0`（= 2026102301 包的 agent），重跑 std-exe 时该值需按新构建重新取。**新包在 X1 修复前打的，不包含修复，必须重打才能验证升级路径** | 🟨 2026-10-23 19:20 已派代理在 win10-gs 重打 standard EXE（构建号 `2026102302`，产物目录 `D:\t10\pkg-2026102302\`）；流程含：源码同步 SHA 对比 → `cargo check` → 4 个测试全绿 → **变异验证**（临时还原缺陷版顺序须让 X1 测试变红）→ 打包 → 版本资源核对 → 还原 version.json/pubspec。**只需 standard EXE，不需要 MSI 与 sos**（X1/X7 只影响 EXE 安装脚本；MSI 走 WiX Component 且已有 `RdBootstrapUpgradeReady` 预检） |
| X6a | **打包结果（2026-10-23 21:05 完成）**：产物 `D:\t10\pkg-2026102302\rustdesk-1.5.3-2026102302-standard-windows-x86_64.exe`，29,099,008 字节，SHA-256 `dff62fc78f51a3fc48231a6b13f1f1809c192adf07c21c1d56084fadbecf8469`。**包内 agent SHA-256 = `51d26e2e498ef36e6a537bb6cebb6dbe910fe70fa8492e58926bd595476ce4c9`（8,200,704 字节），这就是 `EXPECT_AGENT`**。版本资源：包内 `rustdesk.exe` 与 `rustdesk-management-agent.exe` 均为 `1.5.3+2026102302`；外层打包器自身仍是 `1.5.0`（packer crate 版本，与旧包一致）。payload 107 个文件，与旧包完全一致。dll 内嵌脚本字符串顺序 `sc query` → `sc stop` → `copy /Y` 已确认（`BLOCK_ORDER_OK=True`）。耗时：stage 6m59s + agent 5m03s + packer 约 2min。 | ✅ 完成 |
| X6b | **该包不含 X7 两处健壮性修复（已确认，非缺陷）**：打包链 20:45:55 启动，代理在约 20:40 上传源码（当时本地确为 `cecc71d4…`）；我 20:50 才补 X7 边界修复，故包内是 `cecc71d4…`，本地已是 `e8410097…`，差 756 字节。**对本次重跑无影响**：A 用例升级时目录与 manifest 都已存在，走正常路径；B 用例全新机 `RecoveryBootstrap` 目录不存在，`Test-Path` 为假直接 `exit 0`。误判窗口只在「目录已建、manifest 未写」的几百毫秒内（升级后 agent 重启那一刻），重跑脚本等两服务 Running 后才快照，撞不上 | ⚠️ 已知偏差，不影响本轮 |
| X6c | **两个必须记住的坑**：① **测试过滤器**：`cargo test --bin rustdesk -- platform::windows::tests::management_agent` 匹配 **0 个测试**（这 4 个测试属于 lib target），会静默「通过」；正确命令是 `--lib`。代理改用 `--lib` 后 4 passed。② **打包脚本**：复制 pack 脚本时**不得**多写 `cd /d %SRC%`，否则 `rustdesk-portable-packer.exe` 只留在 `deps\`、不会提升到 `target\release\`，导致交付产物缺失。原 2026102301 脚本此处无该行 | ✅ 已记录 |
| X7 | **X1 修复的连带缺陷，已定性为真（2026-10-23 18:50，代码核对 + 只读定性）**：bootstrap 与 agent 是同一二进制的两份拷贝，`bootstrap.json.agent_sha256` 把三者绑死（真机实测三者同为 `6ae671e7…`）。`bootstrap.rs:754-757` 的 `run_once()` **同时校验 bootstrap 自身与 agent 文件的哈希**，任一不符即 `agent integrity unavailable; no start attempted`，既不启动也不恢复；`bootstrap.rs:226-245` 的 `verify_existing()` 在 `install()` 里对已存在目录只校验不重建，漂移后 `initialize_if_allowed` 每轮重试、永远失败（`management_agent.rs:1880-1883`，300s 节流 warn）。`install_me` 的安装脚本**不同步 RecoveryBootstrap**（`bootstrap_update::synchronize` 仅被 `native_install`/`exe_preparation` 调用），故 X1 修好后 EXE 覆盖升级会留下 agent 新、bootstrap 旧的漂移。**修正代理结论一处**：`recreate_missing_agent` 在服务缺失时仍可触发（`bootstrap.rs:793`），因此「服务在但文件漂移」才卡死，「服务被删」可重建。**兜底仍有 3 条**：`install_me` 的 `sc start`（仅当次）、`sc failure` 重启动作（24h 内 3 次、只兜进程崩溃）、服务 `start= auto`（系统重启）。MSI 不漂移：`RecoveryBootstrap.wxs` 受 `RdBootstrapUpgradeReady` 门禁，`bootstrap-preflight.ps1:44-46` 预检三者自洽后才由 MSI 同时替换 | 🟨 真缺陷，待定方案 |
| X7a | 修复方向：用户 2026-10-23 19:00 批准**方向一**（停/禁用 `RustDeskManagementAgentRecovery` 计划任务 → 同步 bootstrap → 重新注册任务）。**实际实现有偏离**：没有停任务，改用 `Copy` 重试 30×500ms（`windows.rs:1689`）。偏离理由：① 停任务会引入新的任务生命周期风险（已注册任务被禁用后若安装中断，恢复链彻底断掉）；② `bootstrap.rs:403-405` 的 `schtasks /Create` 无 `/F`，任务已存在时重注册会失败，处理幂等需要额外状态。重试方案更简单且失败是安全降级。**代价**：极端情况下（恢复任务跑满 60s 执行上限）同步会失败、安装以 `0x5253000D` 中止，状态为「agent 已换新、bootstrap 仍旧」，需人工重装一次。该窗口已记为 X15/X16 并在后续 TDD 中加锁消除 | ✅ 已实现（与批准方案有偏离，已说明） |
| X18 | **工作树内既有的未提交改动，不属本轮四个任务**（2026-10-23 21:20 回归面审查发现，已核实）：`src/management_agent.rs` 的 SCM 失败动作配置从「启动时一次性调用」改为「后台重试线程 + `failures_stopped` 原子量」，且线程 spawn 失败现在会让服务启动失败（`map_err(...)?`）。对比 HEAD 第 1575 行确认为工作树新增（`git diff HEAD --stat` 显示该文件 +115/−16）。该改动逻辑合理（原地升级不会重跑安装脚本，需要每次启动重配失败动作；门禁每次重读以避免 `maintenance.json` 期间跳过）但**应单独说明**，不要与 X1/X7 混在同一次提交说明里 | ⚠️ 提交时单独说明 |
| X19 | **mac 已知缺陷（既有代码，本轮不修）**：`src/updater.rs:2715` `consume_mac_update_result()` 用 `MAC_UPDATE_RESULT_CONSUMER`（`first_mac_update_result_consumer` = `!flag.swap(true)`）实现「先到者独占」。两个调用点：`updater.rs:1134`（`start_auto_update_check_` 开头）与 `2652`（scheduler 线程）。若先到者是 1134 而结果文件尚未落盘，`consume_update_result()` 返回 `None` 立即 `return`，之后 scheduler 线程被永久抑制 → **该次更新结果丢失**。修法需要新增状态（例如失败时回退标志、或改成「结果文件存在才认领」），属 Corner cases 需维护者批准的新生命周期规则，**本轮不修**。mac 路径无法在 Mac 本机完整验证 | ⚠️ 已知缺陷，待立项 |
| X20 | 回归面审查提出的两条**建议**（低优先级，本轮不改）：① `src/management_agent/operations.rs:280` `diagnostics_logs_args_valid` 与 `diagnostic_logs::parse_args` 重复实现同一安全契约、无交叉断言，易漂移；② `src/platform/windows.rs:5310` 的 `LOCAL_QUIET_MSI` 是 HEAD 命令的字面拷贝，`update_me_msi` 未来改动不会被该测试发现 | ⚠️ 记录 |
| X21 | 回归面审查建议**回退**：`src/management_agent/status.rs:15-28` 把 `capabilities()` 数组从多行压缩改成一串一行一项。**核实结论：不是纯格式化**——同一改动实际新增了 `diagnostics.logs@1`（T2.4c 要的功能）。重排只是顺带的格式噪声，但删掉重排会与新增项混在一起；**保留**，不做无谓返工 | ✅ 保留（附理由） |
| X22 | **X1 修复首次真机验证失败（2026-10-23 21:49，`retest_x1.sh` 用例 A）**：升级后 ProgramData agent 哈希仍为旧包 `6ae671e7…`、mtime 21:44:16（升级前一小时），bootstrap 同样未变；但 `RustDesk.exe`、`librustdesk.dll`、`rustdesk.exe` 版本资源全部换成新包 `2026102302`，两服务 Running，身份与配置全保留。15 条断言 3 FAIL（`ProgramData agent sha256 == EXPECT_AGENT` / `== payload` / `changed from old build`），其余 12 PASS。X7 的 9 条断言与用例 B 因失败即停未执行 | ✅ 取证完成（根因见 X23） |
| X23 | **根因（2026-10-23 22:05 win10-gs 复现确认）**：安装脚本执行环境 `trusted_install_environment()` 把 `PATH` 收窄为 `C:\Windows\System32`（`installer_shell.rs:168-173`），而 `powershell.exe` 位于 `System32\WindowsPowerShell\v1.0\`，**不在该 PATH 内**。脚本里那句等服务的 PowerShell 是裸调用 `powershell.exe`（`windows.rs:1769`），实测返回 `PS_EXIT=9009`（`'powershell.exe' 不是内部或外部命令`），**等待根本没发生**。于是：`sc stop` 异步发出 → 立刻 `copy /Y` → 服务仍占文件 → 复制失败 → 进入失败分支 → `sc start` 把旧 agent 拉回 → `exit /b 0x5253000C`。这完整解释现场：agent 服务在 21:47:02 被重启（正是 `sc start`）、文件零变化、bootstrap 同步段未执行（`exit /b` 提前退出）。同文件其它 PowerShell 调用（`windows.rs:1740`、`exe_desktop_shortcut.rs:243/255`）均已用绝对路径，只有这一处漏了 | ✅ 复现确认 |
| X23a | **`--silent-install` 的退出码不可信（重要）**：`retest_x1.sh:102` 早已注明「旧版 `--silent-install` 失败时外壳与子进程都以 0 退出」。EXE 便携包外壳立即返回 0，真正安装在脱离的子进程里。本次实测「installer exit 0」与「目标文件未变」并存即是此因。**任何验证脚本都不能用该退出码判定成功**，必须以文件哈希 + 服务状态为准 | ✅ 已确认，与既有记录一致 |
| X24 | 其它裸命令核对：`sc.exe`、`reg.exe`、`netsh.exe`、`taskkill.exe`、`certutil.exe`、`findstr.exe` 均在 `C:\Windows\System32` 内，受影响的只有 `powershell.exe` 这一处。已改为 `"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"` | ✅ 已修 |
| X25 | **修复已真机验证（2026-10-23 23:20，不经打包）**：在 win10-gs 用 SYSTEM 计划任务、完全复刻 `trusted_install_environment()` 的收窄环境（`PATH=C:\Windows\System32`、`ComSpec`/`SystemRoot`/`WINDIR`/`ProgramData`/`PUBLIC`/`PATHEXT`/`NoDefaultCurrentDirectoryInExePath`）直接跑修复后的命令段。结果：`SRC_OK` → `QUERY_EXIT=0` → `STOP_ISSUED` → **`WAIT_EXIT=0`（旧版此处是 9009）** → `COPY_EXIT=0`，且 `C:\ProgramData\...\rustdesk-management-agent.exe` 哈希由旧包 `6ae671e7…` 变为新包 `51d26e2e…`、mtime 前进。**根因与修复双向确认** | ✅ 真机验证通过 |
| X26 | 对照实验（同一次，同一受限环境）：裸 `powershell.exe` → `OLD_EXIT=9009`（`'powershell.exe' 不是内部或外部命令`）；绝对路径 → `NEW_EXIT=0` 且 `NEW_OK` 正常输出。排除「PowerShell 被策略禁用」等其它解释 | ✅ 对照成立 |
| X27 | 验证副作用与复原：验证时把 ProgramData agent 直接换成了新包，导致 `bootstrap.json`/`bootstrap.exe` 停在旧哈希（即 X7 描述的漂移态）。已手工同步三者至一致（agent = bootstrap = manifest = `51d26e2e…`），两服务 Running。机器当前为「standard EXE 新包 + agent 新包 + bootstrap 已同步」的一致状态 | ✅ 已复原 |
| X28 | **待办：含本修复的包尚未重打**。当前 `D:\t10\pkg-2026102302\` 的包（`dff62fc7…`）**不含** X23 修复，也不含 X7 的两处健壮性修复。`retest_x1.sh` 需用重打后的包再跑一遍全量（用例 A + B，含 X7 的 9 条断言） | ✅ **已重打（2026-10-24 02:13）**：`D:\t10\pkg-2026102401\rustdesk-1.5.3-2026102401-standard-windows-x86_64.exe`，29,105,664 字节，SHA-256 `467fcc20b64b0cf7f06f95fc9869bd9f8ab4f4e87fbd65bac2cc553da450c25d`。**包内 agent SHA-256 = `167defa1464eae68e68bb3a66ab097bb50b56b6977ccc53555b72a41c27c4eea`（`EXPECT_AGENT`）**。版本资源 `rustdesk.exe`/`rustdesk-management-agent.exe` 均为 `1.5.3+2026102401` | 
| X28a | **打包门禁（2026-10-24）**：5 个测试全绿；**变异 A**（`sc stop` 移到 `copy /Y` 之后）→ `management_agent_service_is_stopped_before_its_binary_is_overwritten` FAILED；**变异 B**（绝对路径改裸 `powershell.exe`）→ `management_agent_stop_wait_uses_an_absolute_powershell_path` FAILED；还原后 5/5 绿。**dll 字节校验**：单反斜杠 `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe` 出现 5 次、**双反斜杠 0 次**；偏移 `sc stop`(28343283) < 绝对路径(28343334) < `copy /Y`(28343621)，顺序正确 | ✅ 门禁通过 |
| X28b | **打包脚本的缓存坑（必记）**：`copy /Y` 会保留目标旧 mtime，cargo 因此判定产物最新、跳过编译，直接跑变异 B 的**陈旧二进制**导致误报。回传还原后必须 `LastWriteTime = Get-Date` 刷 mtime 才能真正重编译。另：`pack` 阶段报 `PACKER_MISSING` 是既有行为（二进制落在 `deps\`，由第 5 步 `packer` 的 uplift 补上），不是缺产物 | ✅ 已记录 |
| X37 | **定向重跑第一轮（2026-10-24 02:21–02:28）：35 PASS / 1 FAIL，唯一 FAIL 是判据问题**。用例 A 全流程跑完：agent 哈希 `6AE671E7…` → `167DEFA1…`（X1 目标达成）、X7 的 9 条 bootstrap 断言全 PASS、`installation_id` 与设备 ID `1338209581` 保留、两服务 Running。FAIL 落在 A11「升级窗口内的 SCM 事件 7045/7040」。**定性为判据不可满足，不是产品缺陷**：① 该机 System 日志（2026-09-15 起 8108 条）中 **7036 为 0 条**，7 天内亦为 0；② 我独立对照查询 7040 正常返回 2 条（BITS/Windows Modules Installer），`wevtutil qe System "/q:*[System[(EventID=7036)]]"` 返回空，证明**查询链路正常、是这台 Win10 LTSC 17763 不写 7036**；③ 升级路径走 `sc config` 只改 binPath、服务未重装，本就不产生 7045/7040。三因叠加使原判据在该机不可能通过 | ✅ 已定性 |
| X38 | **A11 判据修正（2026-10-24，我直接改）**：不再依赖事件日志，改用两条更难伪造的直接证据——① agent 服务 **PID 必须变化**；② 新进程 **`CreationDate` 必须 ≥ 升级起点**（UTC ticks，取 VM 自身时钟，时区不参与）。SCM 事件仍照常采集并写入日志，降级为辅助材料。**两个新信号均先在真机单验通过**：PID 提取在真实前后快照上得 `5296 → 17484`；`Get-CimInstance Win32_Process` 探针返回 `17484|639283767043752770`。`bash -n` 通过 | ✅ 已修 |
| X39 | **定向重跑第二轮（2026-10-24 02:47 起）：57 PASS / 0 FAIL / 0 SKIP，用例 A + B 完整跑完**。A11 新判据通过：PID `10964 → 24492`，新进程创建 `639283783881359350` ≥ 升级起点 `639283783825596195`。**用例 B（全新安装）首次完整执行并全 PASS**：agent/hash/RustDesk.exe 三者等于新包载荷、两服务 Running、`--get-id` 非空、bootstrap 三方一致。**X1 与 X7 至此在 win10-gs 端到端验证通过，满足 X5 提交门禁** | ✅ 验收通过 |

| X40 | **X16 前置分析（2026-10-24，未落代码）**：动手加锁前先查死锁面。持 `Global\RustDeskManagementAgentInstall` 期间会执行外部批处理的只有 `windows_exe_agent_update.rs` 的 `Invoke-LegacyAgentUpdate`（持锁后运行 `main.cmd`）。`main.cmd` 的 body 来自 `update_me` 的 `cmds`（`update_transaction_commands` + 注册表/打印机），**不含** `get_management_agent_install_commands`，该路径自己用 `Sync-Bootstrap` 同步 bootstrap，所以 `BOOTSTRAP_SYNC_SCRIPT` 加锁**不会**在这条路径上自锁。`install_me` 本身不持锁。其余持锁点（`bootstrap.rs`、`rescue_first_install.rs`、`local_repair_total_loss_runtime.rs`、`windows_agent_registration.rs`）都在 agent 进程内，不调用安装脚本。**另一个约束**：`platform::windows::tests` 在 macOS 上根本不编译（`--list` 匹配 0 条），X16 的先红后绿只能在 win10-gs 上做。曾临时插入断言 `bootstrap_sync_takes_the_install_mutex`，因用户已定「第一轮先不加」且加锁会让刚验收的包作废，**已撤回**，源码回到 X39 验收时的状态 | 🟨 分析完成，留待后续 |
| X41 | **本轮提交前检查（2026-10-24）**：`flutter analyze` 无 error、`flutter test` 381 通过；`cargo test -p rustdesk --lib` 383 通过（3 ignored）、`cargo test -p base` 83 通过；`git diff --check` 干净。**`cargo fmt --check` 不过，但都是既有问题**：HEAD 自身就有 1717 处差异（如 `tests/windows_update_disabled.rs`，未修改）；`windows.rs` 的 8 处差异中 7 处（`windows_exe_migration` 模块声明、`install_me` 桌面快捷方式、`try_lock_tray_single_instance` 等）**不在任何改动块里，属 HEAD 既有**，1 处（`update_me_msi_receipt`）和 `updater.rs` 的 2 处（`ordinary_update_receipts`）在之前会话的未提交改动里；本轮改动段格式干净。（更正：先前把 HEAD 拷到 `/tmp` 做 fmt 对照得出「HEAD 干净」，但副本处 rustfmt 解析不到 `#[path]` 子模块，该对照无效。）按「不做无关格式化」不处理。**未跑**：`cargo clippy --workspace --all-features`、Windows 目标上的 `platform::windows::tests`（真机验收 X39 已覆盖其行为） | 🟨 记录在案 |

| X42 | **提交（2026-10-24，用户批准方案 A：只提交本轮可独立的改动）**：`5ae601528`「fix: EXE 覆盖安装时先停服务再替换管理代理并同步恢复引导」，仅含 `src/platform/windows.rs` 的 X1/X7/X23 改动块与 5 个测试、`installer_handoff.rs` 的退出码映射（+282/-2）。做法：以 HEAD 为底用脚本只叠加本轮 6 个块后 `git update-index --cacheinfo` 暂存，工作树保持不动；核对暂存版与工作树只差被排除的 5 块（`update_me` 远程窗口、`update_me_msi` 远程日志、`is_self_service_installed`、MSI 日志测试）。**未单独编译验证暂存版本身**（Windows 模块在 Mac 不编译；验收包 2026102401 是用含全部改动的工作树打的），依赖已静态核对均在 HEAD 中。**T2 远程静默（X29/X30/X33/X34/X35）未提交**：`updater.rs` 本轮的来源槽与事件载荷依赖之前会话未提交的 `push_local_update_failed_event`/`start_manual_update_check`，后者又与 `ordinary_update_receipts`、mac 结果消费者等未完成工作交织，单挑会编不过；Flutter 侧单独提交会让本机手动更新失败永远不弹浏览器。需等这批 updater 改动收尾后一起提交。未推送 | ✅ 已提交（未推送） |

| X29 | **T2 远程静默的 3 个关键缺陷（2026-10-24 审查，已核实，属 T2 不属 X1/X7）**：<br>① `software_update_event` 载荷**不带 `request_origin`**（`updater.rs:2152` 只有 `name/status/version/build_seq/source/url`），Flutter 只能靠「本进程 30 分钟内点过更新」猜来源；而 `update_event_reaches_local_ui()` 只过滤 `elevation_required_remote`。→ 用户点「立即更新」后 30 分钟内，任何远程 `package.update` 失败都会开浏览器，违反 T2。`report_update_event_with_origin` 已有 origin 参数但没写进 json。<br>② Android 路径绕过门禁：`common.dart:4277` catch 分支直接 `openUpdateFailureFallback`，未经 `applyLocalUpdateEvent`；而 `startAndroidVerifiedUpdate` 的入口之一是 Rust 侧 `kAndroidUpdateReady`（远程发起）。同文件 `kSoftwareUpdateEvent` 已接门禁，此处遗漏。<br>③ 三处**接线无测试**：`grep -c "handleUpdate\|registerEventHandler" flutter/test/verified_update_entry_test.dart` = **0**；12 个测试全是纯函数。删掉 `update_progress.dart:31` 的时间戳赋值、`common.dart:4305` 的 `applyLocalUpdateCheckEvent`、`common.dart:4317` 的 `if (!applyLocalUpdateEvent(evt)) return;` 任何一处，测试仍全绿 | ⬜ 待修（T2 收尾时） |
| X30 | T2 附属问题（同次审查，已核实）：`deferred` 被当成终态清空本地更新请求。Rust 侧 `deferred` 明确是非终态（`updater.rs:1495/1604` `schedule_update_command_retry`），但 Flutter `_localUpdateActiveStatuses`（`update_progress.dart`）只含 `{started, downloaded, installing}`。→ 有远程会话时点更新，时间戳被清空，会话结束重试失败时不再开浏览器，用户点了更新什么都没发生也拿不到回退入口 | ✅ **已修（2026-10-24）**：加入 `deferred`；测试改为断言 `deferred` 保持请求；变异（移除 `deferred`）确认变红，还原后 13/13 通过 |
| X31 | **raw string 转义陷阱（2026-10-24，打包门禁抓到）**：`get_management_agent_install_commands` 返回 `format!(r#"..."#)`，**raw string 不处理转义**。我照搬 `concat!` 普通字符串里的 `\\` 写法，生成物变成双反斜杠 `"%SystemRoot%\\System32\\..."`。Windows 路径 API 多数情况能容忍重复分隔符，但生成物与预期不符、测试门禁变红。**已改为 raw string 内单反斜杠**。教训：本文件两种字符串混用，改 `windows.rs` 的批处理模板时必须先看该行在 raw string 还是普通字符串里 | ✅ 已修（`cf6afa9a…`） |
| X32 | **Flutter 接线测试的真相（修正审查结论）**：审查代理说「0 个接线测试」**不准确**——`verified_update_entry_test.dart:144` 有 `group('checkUpdate event handler wiring')` 含 5 个用例。但这些用例**直接调用 `applyLocalUpdateEvent`/`applyLocalUpdateCheckEvent` 纯函数，没有经过 `checkUpdate()` 里注册的 event handler**，所以「真实接线未覆盖」的实质结论成立。已在 X35 补齐 | ✅ 已补齐，见 X35 |
| X35 | **接线测试补齐（2026-10-24，TDD 先红后绿）**：<br>① `handleUpdate` 主体抽成 `handleUpdateWith(now, command)`（`update_progress.dart`），使「时间戳在发出请求前写入」可被断言。<br>② `checkUpdate()` 里两个 handler 的内联体抽成具名函数 `handleCheckSoftwareUpdateFinish()` / `handleSoftwareUpdateEvent()`（`common.dart`），并给后者加可注入的 `openFallback` 参数（默认仍是 `openUpdateFailureFallback`），使注册点真正可被测试驱动。<br>**新增测试** `flutter/test/update_event_wiring_test.dart`（5 条，直接调用 `handleSoftwareUpdateEvent`，即 `checkUpdate()` 实际注册的那个函数）：远程 `command_install` 失败不开浏览器、无来源失败不开、本地 `manual` 失败恰好开一次且不重复、超时的本地请求不开、远程事件仍更新状态行。<br>**变异验证**：把 `if (!applyLocalUpdateEvent(evt)) return;` 改成无条件 `applyLocalUpdateEvent(evt);` 后，5 条全部变红（远程失败实际打开了 `['failed','failed']`），还原后全绿——证明测试钉的是行为不是结构。<br>**仍未覆盖**：`registerEventHandler` 到 FFI 的那一跳（依赖 `PlatformFFI._ffiBind`，VM 测试环境无法驱动）；`kCheckSoftwareUpdateFinish` 分支仅抽出未加断言（`updateUiState` 依赖 native 侧结果）。<br>**回归**：Flutter 全量 381 通过（原 376，+5）、`flutter analyze` 无 error | ✅ 已补，仍有未覆盖项 |
| X36 | **一次自己造成的破坏（2026-10-24，已修）**：编辑 `common.dart` 时误删 `if (isAndroid) {` 一行，导致 `kAndroidUpdateReady` 的注册块失去条件包裹、花括号不匹配。`flutter analyze` 立刻报出并在下一次编辑中修正。教训：该文件 handler 块嵌套较深，Edit 的 `old_string` 必须包含足够上下文（至少包含上一行的闭合 `});`） | ✅ 已修 |
| X33 | **X29① 已实现（2026-10-24，TDD 先红后绿）**：<br>Rust 侧：新增 `local_update_event_payload()` 纯函数（`updater.rs:1372` 附近），两处推送点统一走它；新增 `CURRENT_UPDATE_ORIGIN` 全局槽 + `set_update_request_origin()`/`update_request_origin()`，在 `check_update_request` 的两个入口写入当前来源（command→`command`/`command_install`，其余透传 `manual`/`system`/`startup`）。<br>Flutter 侧：`localUpdateFailureMayOpenBrowser()` 增加 `event['request_origin'] == 'manual'` 条件。<br>**测试**：`local_update_event_payload_carries_the_request_origin`、`the_request_origin_is_stored_and_read_back` 通过；Flutter 新增 `a remote failure is silent even while a local request is pending`——**先红后绿**（修前 `localUpdateFailureMayOpenBrowser` 不看来源，远程失败在本地请求窗口内会开浏览器）。<br>**回归**：Flutter 全量 375 通过、`base` 83 通过、`updater::` 30 通过 | ✅ 已修 |
| X34 | **X29② 已修（Android 路径）**：`common.dart` 的 `startAndroidVerifiedUpdate` catch 分支改为 `if (applyLocalUpdateEvent(result))` 才 `openUpdateFailureFallback`。**未补专门测试**（该函数依赖 `gFFI` 与 `bind`，Android 平台路径在 Mac 上无法真实驱动）——按 AGENTS.md「未覆盖必须如实列出」，记为未覆盖 | 🟨 已修，测试未覆盖 |
| X8 | win10-gs 只读取证补充（2026-10-23）：15:35:00 agent 由计划任务 `\RustDeskManagementAgentRecovery`（SYSTEM，PT5M）经 `rustdesk-recovery-bootstrap.exe bootstrap` 拉起（证据：`budget.json` mtime 精确 15:35:00，`next_at` = +300s，与 `bootstrap.rs` 退避公式自洽）；15:38:01 RustDesk 服务由 agent 的 R1 恢复拉起（`events.json` code=r1 `start_service` `terminal_at` 同一秒 + `agent.json last_service_recovery_at` + 服务日志首行）。差 3 分钟是 `recovery::decide()` 的 180 秒启动宽限所致，误差 43ms。**对重跑的含义：靠停服务验证自愈时，超时必须 ≥ 180 秒，否则会误判"不自愈"** | ✅ 取证闭合（X1b 结案） |
| X9 | 注入脚本静态校验（2026-10-23 17:05）：`inject_backup/restore/watchdog.ps1` 三个文件 here-string 与括号配平全部 OK；按进程名杀进程 0 处（5 处 `Stop-Process` 全部 PID 定向且有来源约束，`inject_restore.ps1:472` 还前置校验 `ExecutablePath -ieq $binary`）；`build-env`/`D:\github` 零命中，`robocopy /MIR` 仅注释中的禁止说明；看门狗 180 分钟上限与每会话恢复次数上限（默认 1、上限 2）均实现，失败写 `escalate.json` 交人工；备份递归扫描用手写栈（`:151-152`），`RustDesk*.toml` 计数为 0 即 `NO_TOML_BACKED_UP` 失败；`icacls /save` 与 `/restore` 成对。**未做**：本机无 PowerShell，配平结论来自启发式脚本不是真实 AST 解析 | ✅ 静态通过；AST 与真机仍待 |
| X10 | 测试脚本两处编码陷阱的边界（2026-10-23 实测）：`t10_lib.sh:42` `waitlog`、`mx_t10.sh:17`、`mx_one.sh:12` **继续用旧式 `Get-Content -Raw` 是正确的**，因为 `mx_clean.log`/`mx_install.log` 是 cmd 重定向产出的 GBK，加 `-Encoding UTF8` 反而乱码。仅 `t10_lib.sh:72` 读的 JSON 是 UTF-8，需要 `-Encoding UTF8`。另记两处未修同类隐患：`silent_one.sh:156` 读 `$WATCH_OUT`（UTF-8 JSON）、`upgrade_snapshot.ps1:278` 读 `agent.json`（`last_error` 含中文时会解析失败） | ⚠️ 记录，暂不修 |
| X12 | 定向重跑脚本已就绪（2026-10-23 18:20）：`/tmp/wgs/t10/retest_x1.sh`（430 行，`bash -n` 通过，带桩干跑 42 PASS / 0 FAIL / 0 SKIP）。用例 A：清理 → 装旧包 → 快照 → 远程静默升级 → 断言 ProgramData agent 哈希三方一致（`==EXPECT_AGENT`、`==payload`、`!=旧构建`）+ mtime 推进 + binPath + 身份保留 + 两服务 Running + SCM 7040/7045。用例 B：全新安装冒烟。判据相互独立、失败即停（`exit 1`）。复用 `t10_lib.sh` 的 `task_ps`/`clean_machine`/`probe` 等，计划任务名 `rd-x1`，退出时 `trap` 删除。**未实测**：远端探针（`agent_ticks`/`scm_events`）未在 Windows 跑过；`EXPECT_AGENT` 需重打包后由调用方提供。**SCM 事件判为 SKIP 而非 FAIL**（`sc config` 在启动类型未变时可能不写事件），哈希与 mtime 才是主证据 | ✅ 就绪，待包 |
| X13 | X7 边界审查（2026-10-23 19:40，定向读全部相关路径）发现 2 个真问题，**均已修**：<br>① **判定条件错**：原用 `Test-Path $root`（目录）判定是否需要同步，但 agent 的 `initialize_if_allowed` 会先 `create_dir_all` 建目录（`bootstrap.rs:369`）再写 manifest（`:399`），中间隔着 icacls/copy/schtasks。脚本在这几百毫秒窗口内看到目录而无文件，会把一次健康的首次安装判失败，且失败前已覆盖了 bootstrap.exe。**已改为 `Test-Path $manifest`**。<br>② **manifest 损坏会永久阻断安装**：解析失败即 `exit 1`，而 agent 侧 `install()` 对已存在目录只 `verify_existing` 不重写（`bootstrap.rs:347-349`、`226-246`），`run_once` 同样只读不修（`:729-731`）→ 所有后续 EXE 安装同一处失败，恢复链一直死，唯一出路是人工删目录。且 `WriteAllText` 先截断再写非原子，本项目有断电验收，写到一半断电正好造出该状态。**已改为：解析失败先把原件改名 `.corrupt` 保留现场再失败；写入改为先写 `.new` 再 `Move-Item` 原子替换**。<br>③ 附带修：`0x5253_000D` 不在 `elevated_install_failure_reason` 映射里，只报 "installer command failed" → 已在 `installer_handoff.rs` 补映射。`cargo check --lib` 通过 | ✅ 已修 |
| X14 | X7 审查判定为**可接受**的项：① 全新安装目录不存在直接 `exit 0` 正确（目录由 agent 自建）；② 脚本经 `ShellExecuteEx runas` 提权，`%~f0` 指向 System32 里校验后的 runner，`.b64`/`.ps1` 落在 System32，低权用户不可篡改，与 Defender 的 `embedded_commands` 保护等价（唯一差别是 Defender 有 60s 超时守护，本脚本自身最长约 15s）；③ `BootstrapManifest` 无 `deny_unknown_fields`，字段顺序/格式变化不影响反序列化，`ConvertTo-Json` 把非 ASCII 转 `\uXXXX` 后 serde 正常解码；④ `sc start` 在同步之前不影响正确性——agent 启动时 bootstrap 初始化失败只记 throttled warn 后继续运行（`management_agent.rs:1881-1883`），拒绝启动的是恢复任务，而它本来就已经在失败 | ⚠️ 记录 |
| X15 | **已知限制（需维护者决定）**：同步脚本没拿安装互斥锁 `Global\RustDeskManagementAgentInstall`（Rust 侧所有写 bootstrap 的路径都持它，`bootstrap.rs:348`、`windows_agent_guard.rs:4`）。`Copy` 的 30×500ms 重试只兜文件占用，兜不住逻辑互斥，总上限 15s 而恢复任务可跑满 60s 执行上限（`bootstrap.rs:199`）。最坏情况同步失败 → agent 已换新、bootstrap 仍旧 → `run_once` 报 `agent integrity unavailable`。属"安全但降级"的失败（安装报错退出并保留旧态），不是静默损坏。**加锁需引入 PowerShell 侧互斥逻辑，属新状态，按 Corner cases 规则需批准** | ✅ 用户 2026-10-23 21:00 决定：**第一轮不加**，排入后续 TDD（见 X16）。当时的判断依据：失败是安全降级不是静默损坏；等 X7 真机验证通过后再加，是干净的增量 |
| X16 | **后续 TDD 条目（用户 2026-10-23 21:00 批准排期）**：给 `bootstrap_sync_commands()` 生成的 PowerShell 加 `System.Threading.Mutex` 打开同名 `Global\RustDeskManagementAgentInstall`，与 Rust 侧 `recovery_windows::install_guard::InstallGuard` 互斥。<br>**实施前置**：X7 真机验证通过（`retest_x1.sh` 全绿）。<br>**TDD 要求**：① 先写失败测试——断言生成的脚本里包含同名 mutex 的获取与释放，且获取失败时的行为明确（等待而非静默跳过）；② 实现；③ 变异验证（去掉互斥语句须让测试变红）。<br>**注意**：测试名 `Global\RustDeskAgentTestInstallGuard`（`cfg(test)`）与生产不同（`windows_agent_guard.rs:4-6`），脚本里必须用生产名，不能用测试名。<br>**不涉及后端**：该互斥是本机内核对象（`CreateMutexW`），后端命令表无此概念，无需新增接口 | ⬜ 后续 TDD，待 X7 验证通过后启动 |
| X17 | 用户 2026-10-23 追问「加锁是否要后端加接口」，核实结论：**不需要**。安装互斥是 `CreateMutexW` 创建的本机命名内核对象，仅在本机进程间生效，后端不可见也无从介入。**但退出码语义确实变了**：新增 `0x5253_000C`（copy 失败）与 `0x5253_000D`（bootstrap 同步失败），远程 `package.reinstall`/`package.update` 会把它作为失败上报。后端不解析具体退出码（只当失败处理），故**后端零改动**；`elevated_install_failure_reason` 已补映射让失败原因可读 | ✅ 已核实，后端无待办 |

### T22 win10-gs 测试计划任务隐患 🟨
| # | 项 | 状态 |
|---|---|---|
| S1 | 测试脚本以 `schtasks /create /sc once /st 23:59 /f` 建任务后立即 `/run`，任务在**创建当天 23:59 会再自动执行一次**。2026-10-23 发现 `rd-mx`（`mx_clean.bat`，卸载并清理 RustDesk）与 `rd-t10`（安装截断包）下次运行均为当晚 23:59:59 | ✅ 已 `Disable-ScheduledTask`，两任务 Disabled |
| S2 | 之前各天的同类任务可能已在当晚 23:59 自动重跑过，次日测试前状态异常时应先查任务历史 | ⚠️ 记录 |
| S3 | 修脚本：建任务改为过去日期的一次性触发（如 `/sd 2000/01/01`）或运行结束后 `schtasks /delete`；`t10_lib.sh` 的 `runtask`、矩阵脚本同步修改 | 🟨 2026-10-23 16:05 已改 `t10_lib.sh`/`mx_t10.sh`/`mx_one.sh` 三处为 `/sd 2000/01/01 /st 00:00`；`mx_one.sh.bak-pre-edition` 是备份不改；真机尚未验证（见 T23 X0） |
| S4 | win10-gs 上残留 50+ 个 `RD*`/`rd-*` 历史测试任务（均为过去时间的一次性触发，当前不会再触发）；11:30 起残留 `RDMatrix` 运行态与 `msiexec /?` 帮助窗口，重启后清除 | ⚠️ 待验收结束统一清理 |
