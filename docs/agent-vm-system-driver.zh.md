# VM SYSTEM 测试驱动

2026-10-11 已在两台客体实际执行独立身份探针，返回 `NT AUTHORITY\SYSTEM`、SID `S-1-5-18`。`RustDeskAgentLabDriver` 注册为 SYSTEM、Highest，未通过身份验证任务执行产品安装或故障注入。

| 虚拟机 | 驱动就绪快照 | UUID |
| --- | --- | --- |
| RustDesk-Agent-Win10 | SystemDriverVerified-20261011-v2 | 9dc875ec-2f3c-41d3-aad1-a7745e413d90 |
| RustDesk-Agent-Win11 | SystemDriverVerified-20261011-v2 | a832dec3-eec0-492c-b898-b05103c25870 |

这两个快照保留既有产品现场，不是干净 OS 快照，也不是产品功能通过证明。原干净 OS 快照继续保留。

客体证据：`C:\RustDeskAgentLab\system-access-20261011-v2.result.json`，含机器名、执行身份、任务主体和 `product_tests_started=false`。

## 可重复控制方式

- VirtualBox：`C:\Program Files\Oracle\VirtualBox\VBoxManage.exe`。
- 客体账号：`rdlab`；使用现有 passwordfile，不在命令行输出密码。
- `guestcontrol ... run --exe <程序> -- <参数>` 的参数中不重复 exe 名称。
- 普通 `rdlab` 令牌仍受 UAC 限制。SYSTEM 任务注册成功不代表普通令牌有启动该任务的权限。
- 提权启动使用 `guestcontrol ... start`，客体 PowerShell 内使用 `Start-Process -Verb RunAs -Wait`，保持启动进程存活，正常确认 UAC 后等待子进程完成。
- 已确认 UAC 为预期 Windows PowerShell 时，客体键盘 `38 15 95 b8` 表示 Alt+Y。读取本轮唯一结果文件验证操作，不通过弹窗消失判断成功。
- 不关闭 UAC、不修改安全桌面设置、不挂载或修改 VM 系统盘。
- 运行产品测试前必须核对任务动作、请求文件、包哈希与 agent 哈希；保留失败现场，避免重复运行遗留请求。

之前把 SYSTEM 测试能力判为不可用的结论已经被本次实测纠正。包哈希不一致、恢复功能未完成等产品问题仍须独立验证，不能用权限通道通过代替验收。

## 本轮续做证据

执行均在 VM 内；宿主只编译和传输测试产物。

- Win10 红测确认两项执行器回归：运行中变更命令可能被重放、服务动作返回 false 可能被当作成功；修复后 operations 测试两系统各 8 项通过。
- 本地 disable-agent 门禁红测后修复；SYSTEM 直接调用 start/restart/repair 的拒绝测试核对实际服务 PID、状态、配置不变，两系统通过。
- agent 测试二进制两系统各 38 项通过，含 Rust→PHP→SQLite 签名命令、claim、事件、本地日志往返。
- package gate 两系统各 5 项通过，含 API 批准包引用/目标/build 绑定、签名、降级拒绝和 Windows 文件锁。
- Win10 API HTTP 12 项通过。默认绑定全权限与显式空权限拒绝均断言 SQLite 数据；Win11 浏览器授权流程覆盖 1440/390 宽度，另有重启命令取消不落库/确认后精确命令落库的红绿验证。
- MSI 升级测试包 SHA-256：`8D3EA90EAEE757ED76D44529839994FB6519245B502DC2778A1597028A41D011`。两系统安装后的 agent SHA-256 均为 `3999D4DBEC39CD94D94EACC7BA685B5DAFEC0FAEEF3DAE4D324D361AFF344D14`，与本次打包 payload 一致；安装身份保留、generation 增加、marker 存在、实际 `status --json` 输出通过。
- 包使用已有 Flutter Debug 主程序资产和本轮新 agent，仅证明此组合；不等于全平台正式发布包或新主程序完整构建验收。

升级基线快照：Win10 `25cd5450-3532-4ade-90a2-2c816f86cb63`；Win11 `bb143f19-140c-4e13-8dd2-d158b9278e94`，名称均为 `AgentUpgradeVerified-v5-20261011`。

| 实际故障 | Win10 | Win11 |
| --- | --- | --- |
| 正常停止主服务后自动启动 | 18.18 秒通过 | 11.91 秒通过 |
| 终止 SCM 主服务进程后自动启动 | 59.32 秒通过 | 59.70 秒通过 |
| 错误 binpath 自动修复正常路径 | 120.80 秒通过 | 118.98 秒通过 |
| 主 EXE 移走后切换完整 fallback | 27.40 秒通过 | 补测 186.84 秒通过 |

观察期间没有人工启动服务，agent PID 与 installation_id 保持。Win11 原矩阵在进程已自行退出时的 Stop-Process 竞态导致驱动失败，保留 `fault-v5` 原失败记录；修正驱动后 `fallback-v6` 通过。fallback 使用本轮完整安装目录副本作为前置，不证明自动部署或签名资产供应；SCM Running 不证明业务 ready 或真实远控上线。

本轮证据导出根：`C:\Users\Administrator\AppData\Local\Temp\opencode`。文件前缀为 `RustDesk-Agent-Win10-` / `RustDesk-Agent-Win11-`，后缀包含 `system-access-20261011-v2.result.json`、`system-gates-v4.result.json`、`system-gates-v4.stdout.txt`、`upgrade-v5.result.json`、`upgrade-v5.log`、`fault-v5.result.json`；Win11 另有 `fallback-v6.result.json`。测试工具、产物与证据暂留用于后续回归，完成整体验收后按精确路径清理。

尚未完成：真实线上命令升级/自更新、事务崩溃接续与完整回滚、完整 EXE/SOS/新主程序包、真实远控会话就绪、全量覆盖率及最终全仓门禁。不能用上述局部通过替代这些项目。

## 2026-10-11 后续 TDD 与实包检查点

- `integrity`：两系统各 5 项通过。固定对象流式哈希、大小和摘要不符、文件占用、绝对路径、真实目录联接及原文件保留；没有可信文件摘要时明确返回 `unverified`。尚无完整签名文件清单供应链，不能声称在场 EXE 已经得到发布签名证明。
- 命令入口：两系统实际执行 `package.verify`/`diagnostics.collect` 测试，确认固定对象测量、脱敏错误和拒绝任意正常路径。
- 网络探测：两系统各 3 项通过，本地 HTTP/TCP fixture 覆盖 200/302/403/503、跳转不跟随、超时、端口拒绝。生产目标固定为两个 HTTPS 域名及 `rd.yan.life:21116`；TCP 成功不表示 ID 注册成功。
- `agent-v9` SYSTEM 批次两系统各 75 项通过，六个 exe 均退出 0：agent 43、execution 12、transaction 10、native 2、integrity 5、network 3。修复驱动退出码为空问题后重新执行，旧 `agent-v8` 失败记录保留。
- `helper-v10` 两系统各 20 项底座通过：workspace 3、directory backup 5、file restore 2、resume task 4、service control 2、health pipe 4。helper 曾因测试使用启动前时间校验子进程稍后生成的回执而失败，`helper-v11` 修正取时后各 3 项通过。
- `helper-boundary-v13` 两系统维护门禁 2 项、helper 4 项通过。保护目录中的维护标记绑定 installation/transaction/plan SHA-256/nonce；本地停用和其他事务标记在真实双进程接管路径被拒绝，旧记录保留。此模块仍未接入生产事务 CLI。
- `agent-v14` SYSTEM 批次两系统各 44 项通过：修复正常/保底修复绕过退避、保存失败后内存/旧磁盘数据不变、`check-now` 本地禁用在目录/网络/写入前拒绝。删除服务观测内的嵌套包恢复，避免外层旧状态覆盖内层结果，原服务循环仍负责包恢复。
- v16 journal 两系统各 10 项通过，status 投影单项通过：运行中/需要对账显示真实状态，损坏日志显示 unknown，读取不修改记录。v19 effective recovery 单项两系统通过：维护、disable-agent、用户停止/未知意图和 Disabled 启动类型不能显示恢复有效。
- `native-ready-v18` SYSTEM 两系统各 4 项通过：SCM Running 不足以证明客户端 ready，要求健康管道进程路径/版本/nonce 一致；不依赖外部注册。agent 就绪、安装结果接续和真实回滚仍需完善，不能视作完整事务验收。

| 检查点 | Win10 UUID | Win11 UUID |
| --- | --- | --- |
| AgentTdd75Verified-v9-20261011 | 41a2ed00-453e-4ad6-835c-2385ddfa033b | ed5212a6-0e7f-40ba-b635-0d36967e4413 |
| AgentUpgradeVerified-v15-20261011 | e722e7c2-bb1f-4b9c-95e6-acd850c9f10a | d9554d3b-9039-4dce-a195-c3478fd516ae |
| AgentRepairVerified-v17-20261011 | 5151aeee-fd79-4d7c-b8e6-58b08e5d17a0 | 4cfa51b8-de06-476d-8f60-897f96c9c510 |

v15 MSI 的 SHA-256 为 `682365710A96C1C76575B66D8DF0A7AE5209E9272A819432E36AB2D859F9AC66`，安装 agent 为 `A239BE33EF9AC308E99510F54F9A740FB2A7B60B8AE047A02D8F438CE3BC0B2F`。两系统退出 0、身份保留、启动代次递增、双服务 LocalSystem/Auto/Running。MSI 仍使用现有 Flutter Debug 资产。

v17 实包故障测试：保持维护 35 秒，主服务停止且错误 binpath 未被 agent 覆盖；移除本轮维护后 Win10 15.08 秒、Win11 15.40 秒自动修复，无人工 start。`service_recovery_failures` 递增、未来退避与恢复时间写入，安装身份和 agent PID 不变。当前 VM 恢复在正常主路径，未停留在故障现场。

上述 v15/v17 实包不包含后续 v16/v19 状态修改，helper/maintenance/native installer 仍是尚未生产接线的底座。仍需继续完整任务账本，不能在此宣布远程升级/agent 自更新完成。

新增导出证据仍在前述临时根，前缀为虚拟机名，后缀包括 `agent-v9.result.json`、`agent-v14.result.json`、`helper-v10.result.json`、`helper-v11.result.json`、`helper-boundary-v13.result.json`、`maintenance-v12-green.result.json`、`upgrade-v15.result.json`、`upgrade-v15.log`、`repair-v17.result.json`。VM 内的 `RustDeskAgentLab*` 任务及测试运行库供后续持续测试使用；测试建立的唯一 fixture 服务和任务由各测试清理，仍须在最终验收盘点。

## 后续健康与恢复验证（v18–v24）

- `agent-health-v21-green`：两系统 SYSTEM 实际执行 agent 二进制全部 48 项，退出码 0。含健康管道状态投影、完整本地门禁、损坏命令日志保持 unknown、禁用策略在刷新前拒绝且原状态字节不变。
- `native-resume-v22`：两系统各 5 项通过，退出码 0。NativeInstaller 仅在应用包之前要求原 MSI 和原身份仍在位；Applying/Validating/RollingBack 阶段允许打开已验证备份做恢复。实际升级回滚仍未验证。
- `live-status-v23`：两系统真实主程序健康验证均失败，返回 `health_unconfirmed`，未篡改断言。v24 明确错误为固定健康管道不存在（os error 2），当时 `--service` 和 `--server` 进程都在运行。
- 打包用 Flutter 目录的 `librustdesk.dll` SHA-256 为 `3DE31DC46E8A61F532984022893585D2A3C693F3E5D305BA8C25544690F36E10`，与此前健康 DLL 构建 `044AC5F029E05CCE763D3BE922FBFA06B2858209DE9372F7B951A69658374422` 不同。正在重新构建当前源码 Flutter Rust DLL 并按 MSI/agent/DLL 三摘要重新验收；此前实包服务恢复通过不能替代应用健康通过。

## 当前源码主程序健康包（v25）

重新构建 `cargo build --locked --lib --features flutter` 并核对必要 Flutter 导出，将新 DLL 与 agent 放入独立 MSI fixture；2026-10-11 两系统安装后均验证成功。主程序实际健康管道返回 `ready`、版本 `1.5.3`、build `2026100502`、独立 `registered=false`，双服务 Running，安装身份保留，generation 递增。v23 的失败记录保留，不以更换断言消除失败。

| 产物 | SHA-256 |
| --- | --- |
| v25 MSI | d158bbb84ff98b7b1ac84cd373181555f9b97a9a2782c6690aaaa8859f4956d3 |
| v25 agent | 9c08ecb3b3bfc3eb6ebe90734251bfe5ac437196bddeaafa5be467a18465611b |
| v25 Flutter Rust DLL | e678cba1da59a7d70cd64149436c041326f027999cb8047daab49290fe467769 |

健康快照 `AgentHealthyVerified-v25-20261011`：Win10 `20b2a996-5ba2-4aff-bf83-f9ac58d1add3`；Win11 `272d1986-6168-41b6-bda5-e5493369df43`。此前 v5/v15 包不具备该主程序健康证明。当前包仍为 Debug fixture、离线本地验收，没有完成正式四包或在线远控验收。

安装结果持久化 `installer-record-v27`：两系统各 6 项 installer process、5 项 native installer 通过。结果绑定 transaction/step/package SHA-256；启动意图先落盘，结果缺失或写入失败不能再次运行安装器；NativeInstaller 将可能已启动但结果不明的路径视为 Pending，避免与安装器竞争回滚。仍未接入生产命令 CLI。

真实同包 MSI 校验失败→回滚用例已作为显式 ignored VM 专用测试加入 `tests/management_agent_native_install.rs`。它不验证签名远程命令和 helper 编排。首次 v28 在停服前暴露 NativeInstaller 路径以 `/` 拼接、与 SCM `\` 文本比较不一致；已局部修复路径构造并从健康快照重跑。首次失败结果已导出为 `Win10-first-native-v28.*`，不能列为回滚通过。

真实用例继续发现两项问题，尚在迭代：把 MSI 改名为 `package.msi`/`original.msi` 导致 SecureRepair 1316/1603；保留原包名后安装成功，但回滚将安装目标的正常 ACL 错当成事务目录 ACL，返回 `workspace unexpected permission principal`。正常 agent 带 Users 只读执行权限，身份 TOML 带 LocalService 写权限。备份目录继续保持严格 SYSTEM/Administrators；恢复目标允许只读主体，LocalService 写权限仅允许两个固定身份路径。Win11 文件恢复/工作目录各 3 项通过，Win10 正在从健康快照重跑真实 MSI 用例。失败日志分别导出 `Win10-native-v29-*`、`Win10-v30-*`，没有把失败记录改成通过。

## 真实同包回滚与重启最终结果（v31–v35）

上述两项问题修复后，Win10 和 Win11 显式运行 `actual_msi_validation_failure_restores_verified_original_and_identity` 均通过，退出码 0。真实 msiexec 执行后目标清单被测试故意设为不匹配，事务进入 RolledBack；核对原目录全部文件、agent SHA-256、安装身份及实际健康管道。只证明同一版本 MSI 原生执行器回滚，不证明不同版本降级回滚、远程签名命令、helper 编排或断电接续。

随后真实重启两台 VM，`reboot-v32` SYSTEM 只读核验均通过：双服务 LocalSystem/Auto/Running，主程序 ready，隔离注册 false，agent/DLL 哈希保持 v25 值，安装身份保持，session generation 递增（Win10 9、Win11 10），测试维护标记不存在。fixture 服务与 `RustDeskManagementAgentResume-*` 测试任务均无残留。实验室驱动任务继续保留用于后续验收。

| 快照 | Win10 UUID | Win11 UUID |
| --- | --- | --- |
| AgentNativeRollbackVerified-v31-20261011 | bb196b36-4cd7-4129-a607-8acddce24e0c | 403ec8b4-e031-42b7-802a-df68fdefa4cb |
| AgentRollbackRebootVerified-v32-20261011 | ce5ecc4e-e230-4b1a-9f9e-4d1f55421948 | c10e5a24-2047-40c9-bf9a-9b1d695089eb |

`owned-helper-v33-green` 两系统各 5 项通过：真实父/子进程接管后重新检查本地门禁，固定 fixture backend 只执行一次，事务状态和执行结果持久化。`transaction-helper --transaction-id <32位小写hex>` 已作为生产受限入口接入，工作目录固定为 ProgramData/RustDesk/ManagementAgent/transactions，需自身位于对应 runtime/helper.exe，通过真实发布签名、handoff、任务和维护校验。原生产远程命令尚未创建这些资产；`resume-install` 尚未接入，不能宣称完整包命令可用。

`agent-v35` 两系统 SYSTEM 各 50 项通过，退出码 0；helper 参数入口测试各 1 项通过。已安装包仍是 v25，v33–v35 编排修改仅在测试二进制中验证，未重新安装。当前工作包4继续进行。

## 接续与收尾（v36–v41）

- v36：两系统 SYSTEM 的 installer process 6 项、handoff 6 项通过。安装结果只读加载绑定事务/步骤/摘要；原进程存活验证绑定 PID 与创建 FILETIME，读取权限错误拒绝接续。
- v37/v39：真实第二个 helper 进程恢复终态和 Applying 中断各分支。原 helper 活着时拒绝；缺失安装结果时保持原事务且不执行；匹配结果补回后仅验证、不重装。两系统 helper 6 项通过。
- `resume-install --transaction-id <id>` 已接入生产 CLI，固定自身路径、重新验证包/资产/计划/历史handoff/注册任务/维护归属/安装锁，按阶段使用持久安装结果。
- v40：维护标记按句柄核对归属后删除，错误 nonce 拒绝、本地禁用及原状态保留，两系统各 3 项通过。
- v41：生产 helper 终态收尾要求事务、计划、执行结果匹配后才释放自身维护标记并删除已验证的接续任务。损坏结果拒绝，运行中事务拒绝。两系统 SYSTEM 各 helper 6 项＋agent 50 项通过，所有退出码 0。已安装包仍为 v25；未将这批测试程序冒充已安装 helper 新包。

仍缺少完整生产父进程准备/批准包提取与可信文件清单、原包名保留、安装锁交接、命令结果与事务对账、bootstrap 资产更新、EXE/SOS 与不同版本回滚、真正断电后恢复的实包 E2E。已注册接续任务并不自动证明这些链路完整。事务终态释放维护后若删除任务失败，当前会明确报错，留待人工对账；不自动重新获取或改写缺失的维护状态。

回归面：`management_agent.rs`/`management_agent_bin.rs` 只增加受限 helper CLI、模块接线和包恢复门禁；`operations_runtime.rs` 改固定对象报告、脱敏诊断、网络探测和中断命令判定；`status.rs`/`journal.rs` 改真实应用健康、恢复门禁和命令状态投影；`windows.rs` 改固定服务动作前置门禁与等待；事务专用模块改签名绑定、接管/接续、结果持久化和回滚校验。`workspace.rs` 严格备份策略保留，安装目标恢复策略允许已证明的只读主体，LocalService 写权限只用于固定身份路径。需要这些修改才能避免实际测试出现的假成功、重复安装、错误拒绝回滚与禁用意图被绕过。

最新证据已导出至临时根，前缀为虚拟机名，后缀含 `reboot-v32.result.json`、`record-reader-v36-green.result.json`、`resume-applying-v39.result.json`、`release-v40-green.result.json`、`finalize-v41-green.result.json`、`helper_runtime.v41-green.stdout.txt`、`agent_full.v41-green.stdout.txt`。源测试、不同阶段失败日志和快照继续保留供回归使用。

## MSI 资产和父进程准备（v42–v55，进行中）

`checkpoint-v44` 两系统 SYSTEM 统一执行 agent 50、helper 6、native 5 项全部通过，已安装程序 ready、fixture 服务和任务无残留。快照 `AgentCheckpointVerified-v44-20261011`：Win10 `87e578ab-c540-4ffd-a9f1-7fd2ee6b2543`；Win11 `2cac64df-134f-4f8e-9b1f-eb5de03cad82`。

- MSI 元数据只读 API：真实包产品名、版本、ProductCode、UpgradeCode及包字节不变，两个系统各 2 项通过；NativeInstaller 复核原包/目标包产品族和准备记录的 ProductCode。
- v45/v46：测试专用签名验证后的真实 MSI 管理提取、完整目标目录清单和 agent 副本落盘，两系统通过。提取前后服务 PID/配置、agent 安装身份/启动代次和源包字节保持，拒绝已有提取/准备目录覆盖。生产公钥仍拒绝 fixture 签名。
- v47：缓存保留批准 URL 原文件名，拒绝 Windows 保留名和路径穿越；包门禁两系统各 6 项通过。此修复对应真实 MSI SecureRepair 1316 失败，不改 SecureRepair 设置。
- v49：Windows Installer `LastUsedSource` API 返回目录，类型从 `LastUsedType` 单独读取；两系统 SYSTEM 验证源包名、文件摘要和产品身份通过。当前源包仍指向保留的原生回滚测试目录，必须在后续清理时保留或由明确的安装流程迁移源，不能直接删除。
- v51/v52：`preparation.rs` 在停服前验签目标/回滚包、锁定包文件、检查本地门禁和固定服务、准备完整备份/身份/helper、提取签名目标、落盘不可变计划及 PackageVerified 事务。两系统真实 MSI 用例通过，身份/降级/被改签名拒绝不创建事务目录；准备成功仍保持服务 PID、安装身份和启动代次不变。Win11 固定身份路径大小写差异已标准化，不扩大路径白名单。
- v53：维护标记独占创建使用创建时 SYSTEM/Administrators ACL，两系统各 4 项通过，已有标记不覆盖。
- v54：父进程持安装锁，重查准备资产/门禁，创建维护标记、启动固定 helper、核验 ready、注册并读回 resume 任务，然后依次持久化 Prepared/StoppingServices 和 release；真实父子进程回归两系统各 9 项通过。失败保留记录，不发布未验证 release。
- v55：准备→父进程→helper→真实同包 MSI→目标健康→终态清理的整链用例正在 Win10 运行，尚无结果，不列为通过；使用测试签名，不能替代生产 API 在线验收。

`preparation` 和 `msi_assets` 已加入生产编译，但远程 `package.reinstall/update/agent.update` 尚未准入，也未声明能力。生产 rollback manifest 获取、远程命令到准备输入绑定、父进程 launch 调用与事件对账仍缺接线。EXE/SOS、不同版本升级与回滚、真实断电、活动会话延期、bootstrap 升级和完整全仓门禁继续保留为未完成。

## 最新检查点（v56–v79）

本节更新前面历史阶段的结论。尚未完成完整项目验收。

- v56：两系统测试签名下真实 MSI 准备→父进程交接→独立 helper 安装→健康验证→维护标记/接续任务清理全部通过，退出码 0。Win10 `C:\RustDeskAgentLab\preparation-5144`、Win11 `C:\RustDeskAgentLab\preparation-4544` 保留安装源和事务证据。首次成功测试曾误删被 MSI 登记为源的目录，已恢复快照并修正清理后重跑，不能再次删除已登记源。
- v59/v60/v62：v2 活动状态管道两系统各 5 项通过，v1 格式保持兼容；延期日志各 11 项通过；Rust→PHP→SQLite 活动状态未知导致 deferred、重复轮询不重复事件、服务 PID 不变，agent 50＋operations 9 项通过。尚无真实活动远控会话覆盖。
- v63 实包升级失败：主程序 DLL 替换而 agent 没替换；MSI 日志为 `Existing file is unversioned but modified`。新增 agent 专用版本资源后，v65 从同一失败现场升级成功，安装 agent/DLL 哈希均匹配；实际 v1/v2 管道相同进程身份、空闲 true、隔离注册 false，安装身份保持。
- v65 agent 资源为 `1.5.3+2026100502`；数值资源为 major/minor/build高16位/build低16位，保留完整 build 序号。资源仅通过 bin 专用 link 参数链接，Windows SDK `rc.exe` 优先由构建环境绝对路径定位。没有修改 `version.json` 或正式发布版本。
- v66/v67/v68：事务结果绑定命令摘要、授权代次、安装实例、目标、计划摘要和事务终态；两系统底层 3 项、agent 50 项通过。真实 SQLite 中先拒绝旧代次结果，修正匹配的 fixture 后补报一次成功，不修改服务和 helper 证据。
- API 包解析增加当前已安装 build 对应的有效回滚清单，并再次检查操作权限；两系统 PHP 76 断言、HTTP 13 项通过。当前 build 仍取 agent build，与主程序独立升级后的版本差异需继续验收。测试服务器启动超时补入原有 10 秒等待范围，不改变业务超时。
- v70：MSI 包命令已接入参数验证、目标/回滚包签名与下载、准备记录、交接前权限/批准重查及 helper launch；agent.update 当前走完整 MSI 包，尚未验证独立更新语义。新包能力仍未加入声明，完整远程创建→下载→安装→重启补报尚未验收，不能称已可用。
- v72：真实本地 HTTP 下载矩阵发现 Windows rename 可覆盖原缓存，改用不覆盖的 MoveFileExW。大小、哈希、截断、重定向、缓存锁和中断残留保护通过；两系统 agent 各 51 项通过。
- v73：两系统 SYSTEM 检查当前 v65 实包健康、哈希、身份及无 fixture 服务/任务残留通过。快照命令返回进程异常，但随后 `snapshot list --machinereadable` 确认两快照已保存且为 CurrentSnapshot，未重复创建。
- v75：生产 helper 的 30 秒 release 等待在 31 秒父进程延迟测试中失败；独立等待增至 240 秒，takeover 继续验证原 handoff 300 秒有效期。两系统 helper 10＋agent 51 项通过。该修复尚未打入已安装 v65。
- v76/v78：新增 v25→v65 不同 ProductCode 升级后故意校验失败回滚，先暴露 MSI 参数混用 `/` 导致 1619，再暴露原 MSI 自动启动服务导致回滚目录文件占用。已分别修复参数 Windows 分隔符和原 MSI 返回后再次停止两个固定服务；v79 两系统实际重跑通过，Win10 约 127 秒、Win11 约 163 秒。断言安装结果为 Installed、终态为 RolledBack、原产品已注册且目标产品不再安装、原目录/agent/身份/健康恢复。两包 Rust build_seq 均沿用 fixture 值，不代表正式不同发布 build 验收。

| 当前实包 | SHA-256 |
| --- | --- |
| v65 MSI | b332083068c9346b6b381e459a63a22f3c2deef73088325c5225b14826467050 |
| v65 agent | 888087d1df26adb61c51511f2805cacc60502c274a225135cc99ce7d1c0d6eb0 |
| v65 DLL | d9e35a7678f78a4e20de85b43422a354a2e6c939543d28e74e5ef350d68b812d |

| 快照 | Win10 | Win11 |
| --- | --- | --- |
| AgentLocalTransactionVerified-v56-20261011 | dc6f071f-0265-4e71-95cf-fd4c7faed82d | 87752ce6-a020-4bb3-bfe0-8ea1240048f0 |
| AgentVersionActivityVerified-v65-20261011 | 47eb844d-f7a0-4c8d-93c7-61505368471c | 32c46f5e-d5b2-43e9-96e0-db50d804d75f |
| AgentCheckpointVerified-v73-20261011 | 03c7aa98-2e83-40a6-84d3-1d311fa4846c | c65b4a92-8632-4b8f-a113-e8e14d88d100 |
| AgentUpgradeRollbackVerified-v79-20261012 | 47479869-8966-45f5-85e1-2e89d99ffd92 | 69bd72f2-d808-487b-bf50-3348fa298b7e |

当前两系统均完成 v79 回滚并恢复 v25 程序，不能用 v73 时的安装哈希代替当前现场核验。失败记录已导出 `Win10-upgrade-rollback-v76-*` 和 `Win10-v78-upgrade-rollback-v76-*`；通过记录为 `Win10-v79-upgrade-rollback-v76-*` 和 `Win11-v79-upgrade-rollback-v76-*`。v79 快照在实际通过后保存，命令返回了表内 UUID；事务目录中的原 MSI 已成为安装源，必须保留。

## 停服前活动复查与远程解析拒绝（v80–v87）

- v80 红测证明 helper 在无法确认活动状态时仍可调用安装执行器；v81 在 `OwnedInstallation::execute` 进入执行器前增加远程事务专用 v2 活动复查。未知、错误版本、错误进程和忙碌均拒绝，不执行停服或回滚安装；本地恢复的原路径不受此检查影响。两系统 helper 12、native 6、agent 52 项通过。
- v82 真实第二个 helper 进程接续 `StoppingServices` 阶段也进行活动复查，拒绝时事务、维护标记和服务 PID/状态/配置保持，不创建安装意图或执行结果；两系统通过。目前拒绝会保留维护和接续记录等待对账，不代表已有自动延期重试或无间隙会话锁。
- v83 PHP→Rust 包引用使用真实 PHP 规范化 JSON 校验，包含中文、换行、斜杠及未知字段；引用匹配仍不能替代发布签名。两系统 agent 52 项通过。
- v84 Rust 请求实际 PHP 路由和 SQLite，验证权限撤销 403、回滚清单缺失 409、无效签名拒绝；缓存目录/事务目录未创建，命令、事件、心跳和实际服务不变。两系统 agent 52 项通过。
- v85 编译 29 个专项测试目标成功。首次驱动误将 PowerShell 5.1 JSON 数组包成嵌套数组，在执行前退出；v86 修正后两系统前 14 目标共 90 项通过，在 MSI 资产目标的零执行/1 ignored 处按规则失败。v87 核对原有已通过目标的二进制摘要后续跑，并明确启用具备现场条件的真实 MSI 资产提取用例；尚待完整结果。
- v87/v88：两系统最终 29 个专项目标各 161 项通过，5 项 ignored 未计为通过。MSI 资产提取实际运行；`native_transaction_child` 独立调用无动作，已从专项计数中跳过。v88 再次确认 v25 agent/DLL 摘要、原安装身份、双服务 Auto/LocalSystem/Running、应用 ready，且无 fixture 服务/接续任务/维护标记。证据导出为 `RustDesk-Agent-Win10-regression-v88-evidence.zip`、`RustDesk-Agent-Win11-regression-v88-evidence.zip` 和对应 checkpoint JSON。

v88 快照 `AgentRegressionVerified-v88-20261012` 已保存：Win10 `dadaf94a-3b16-4c32-b538-c9f412aecc79`，Win11 `1fa88ab8-95db-49ec-ae3d-1e36ef346dce`。这是专项回归检查点，不是全仓测试、覆盖率或正式远程升级验收。

## 回执完整性与更新后交接链（v89 起）

- v90 红测明确复现缺少 `error` 字段的 Committed 回执被判为 `succeeded`。v91 要求终态回执必需字段完整且类型正确，保留原有命令/实例/代次/摘要核验；两系统结果绑定 4 项和 agent 52 项通过，原结果字节保留。
- v92 重新恢复 v73 的 v65 安装基线，使用最新 preparation/helper 测试二进制及真实 v65 MSI 执行本地整链，测试 helper 改用生产 `wait_for_release`，并经过新加入的远程停服前活动复查。两系统通过，Win10 167.05 秒、Win11 183.14 秒，Committed/身份/健康/维护标记和任务清理均验证。使用测试签名，仍不能替代 API 命令创建→下载→安装→补报的完整远程 E2E。

v92 快照 `AgentActivityChainVerified-v92-20261012`：Win10 `55a83053-b326-445c-a099-8d0b4fe78d80`、Win11 `7c453b6d-13d6-4a42-84fd-0321a502483e`。当前安装源及事务证据分别为 `C:\RustDeskAgentLab\preparation-1852`、`C:\RustDeskAgentLab\preparation-1548`，必须保留。`chain-v92.*` 已导出到宿主证据目录。

v91 本轮生产修改仅为 `command_transaction.rs` 的终态回执完整性检查；测试文件扩展缺字段/错误类型断言，未修改 helper 输出格式。v92 只改测试 fixture 选择与等待入口，已安装程序仍是历史 v65，最新源码测试二进制与实际已安装版本分别记录。

- v94 实际子 helper 红测发现 finalize 接受缺失或矛盾的 `error` 字段并删除维护/接续记录；v95 在清理前拒绝缺字段、错误类型和 Committed 携带错误。两系统 helper 12、agent 52 项通过，损坏结果和维护证据保留。
- v96–v98 bootstrap 更新专用模块完成独立红绿测试：准备绑定事务/计划/旧新 agent 摘要，更新固定入口和清单，回滚恢复旧字节，保留预算和任务文件；覆盖文件占用、旧新混合中断、外来字节拒绝、未安装 bootstrap 不创建及准备后目录出现拒绝。两系统 agent 54 项通过。尚未接入生产安装事务，不能称 bootstrap 升级已完成。
- v99 正在运行停服前必须有 bootstrap 准备记录的实包红测，等待准备结束；尚未改动真实 bootstrap 入口。
- v99 实包红测已复现缺少 `bootstrap-update.json`。v100 新准备事务使用 NativeAssets schema 2，要求绑定的 bootstrap 记录存在；schema 1 旧事务继续兼容。原生安装验证成功时同步新入口，回滚时恢复旧入口；两系统 agent 54、helper 14、native 8 项通过。
- v101 两系统通过已安装 v65 agent 的 `install-bootstrap` 正常入口创建固定计划任务，然后真实同包 MSI/helper 整链提交成功。bootstrap 摘要与 agent 一致、任务定义未改变，真实 `bootstrap` 入口退出 0。仍为同包，不证明旧新 agent 字节不同的完整升级回滚。

v101 快照 `AgentBootstrapChainVerified-v101-20261012`：Win10 `ccb1ed20-aabc-4549-ac98-3c092acc5774`、Win11 `f2927208-c9f7-424c-91e5-7cc1fee8c1fe`。bootstrap 固定任务有开机和周期触发，作为下一阶段恢复测试资产保留。v102 将从 v88 的旧 v25 基线验证不同包/agent 字节变更后的 bootstrap 回滚，尚待结果。

- v102 在安装前的 fixture 准备阶段重复复制目标 agent 被拒绝；v103 改为直接引用已封存的目标 artifact，未修改生产复制权限。两系统实际升级并验证新 agent/bootstrap 摘要后注入目标验证失败，恢复旧产品注册、目录、agent、bootstrap 原始清单和身份，随后实际执行旧 bootstrap 退出 0。两份 MSI 的产品代码/文件字节不同，Rust build_seq 仍相同，正式不同 build 迁移仍未覆盖。
- v103 快照 `AgentBootstrapRollbackVerified-v103-20261012`：Win10 `293d10e6-4ef2-40e5-b3ce-829b282c8d9f`，Win11 `d86c4539-f5e4-492a-b759-52cd2bf7cd5c`。必须保留的当前 MSI 源目录为 Win10 `C:\RustDeskAgentLab\native-bootstrap-rollback-4120`、Win11 `C:\RustDeskAgentLab\native-bootstrap-rollback-1564`。结果、事务、bootstrap 记录和两份安装日志均已导出宿主临时证据目录。
- v104 正在验证已注册 bootstrap 任务对双服务停止的恢复：先保留本轮 `disable-agent` 验证不启动，再移除本轮标记并启动真实注册任务，等待 agent 自行恢复主服务。没有手动启动产品服务。该场景手动触发已注册任务，不等于已证明开机触发或等待完整五分钟周期。
- v104 两系统 `disable-agent` 门禁均通过，随后 bootstrap 已拉起 agent；Win10 在任务完成等待处超时，Win11 在 180 秒主服务恢复窗口超时，均保留 FAIL。源码启动宽限期也是 180 秒，尚需覆盖其后轮询间隔；随后独立 SCM 查询两系统双服务 Running 不替代本轮失败。Guest Control 新 PowerShell 进程停滞，已导出 `dtf-v104.result.json` 后正常重启两 VM，准备诊断并重跑。
- v105 两系统重启后只读诊断确认双服务运行、应用 ready、身份保持、本地标记均不存在，bootstrap 任务退出码 0。v106 将主服务恢复观察窗口扩至 420 秒，Win11 实测 181.565 秒通过；Win10 的任务完成观察仍超时。
- v108 Win10 采样查明驱动问题：第二次启动真实任务后观察到 Running(4)/267009→Ready(3)/0，但 LastRunTime 与第一次相同，原驱动仅接受时间戳递增，错误地等待至超时。v109 驱动改为时间戳前进或明确观察到 Running→Ready 后才接受退出码 0，生产任务和恢复程序未修改；等待旧轮结束和退避到期后重跑。
- v107 两系统验证 NativeAssets schema 1/2 序列化读取兼容，0、3 和最大 u32 未知版本拒绝。该测试只读 fixture，不触碰服务。

## 周期恢复与自动初始化（v110 起）

- Win10 v108 最终恢复成功，182.965 秒；期间观察到后续周期任务，不能把结果仅归因于第二次手动触发。Win11 v106 为 181.565 秒。对应双服务恢复快照：Win10 `0bb3c144-9352-4f15-8901-01b1fa8fc0a1`，Win11 `616c809e-f0de-486d-abc8-b8932de2e20e`。
- 新持久验收脚本 `scripts/guest-agent-bootstrap-recovery-test.ps1` 仅允许指定 VM/SYSTEM，使用独占本轮标记，检查原有门禁和退避，不手动启动产品服务。PowerShell 5.1 无 BOM 中文脚本解析失败已复现并补 UTF-8 BOM，两 VM 解析通过。
- v110 解除禁用后不再手动触发任务，等待已有五分钟周期：Win10 292.996 秒、Win11 302.854 秒通过，双服务/应用 ready/安装身份/文件摘要/服务配置/固定任务不变。本地禁用下仍不启动。证据已导出 `periodic-v110.*`。
- 周期恢复快照 `AgentPeriodicRecoveryVerified-v110-20261012`：Win10 `8a43908c-eb21-43a2-9848-72041a30ecb6`、Win11 `11b2c98f-dadc-409f-a8e2-71374e3de3b7`。当前实包为回滚后的 v25，不代表已安装最新源码。
- v111 红测发现 bootstrap 准备记录省略 `manifests` 被当成未安装，v112 要求字段显式存在，允许明确 null；两 VM 各 2 项通过，v113 全 agent 各 54 项通过。
- v114 红测证明正常 agent 启动不自动建立 bootstrap。v115 增加本地门禁和明确 Enabled 意图后的一次初始化，身份落盘后调用，失败保留日志并后续重试。两 VM 全 agent 各 55 项通过。实际新 MSI 自动初始化验证尚未完成，不称正常升级已具备该能力。
- v116 新 agent 与 Flutter Rust DLL 已实际构建并生成测试 MSI。两 VM 恢复到 v88 无 bootstrap 的 v25 健康基线，正在通过普通 MSI 安装验证自动建立入口、SYSTEM/五分钟/开机任务、保护 ACL、摘要和设备身份。本轮不调用 `install-bootstrap`；结果未出前不列为通过。

v116 测试包摘要：MSI `6743bc449663c29f1b9fe4b0d5529b158341cac1e81fc2a5af88f0f2b820966f`，agent `6c1f9f70368a12b87242508ffd69a563e9346cbe1496cbb0855e4131daa2cf5c`，DLL `a94894f955f2d9fd303b8c6f24d268fdf42a167e02dc040cbefc960ca8f0549b`。使用现有 Debug Flutter runner/data 与本轮新 Rust DLL/agent 打包，不是正式 Release 四包。

- v116 普通 MSI 安装两系统均通过：未手动调用 `install-bootstrap`，自动生成保护目录、匹配摘要的入口/清单、SYSTEM 五分钟及开机任务，原身份保持且应用 ready。快照 `AgentAutoBootstrapVerified-v116-20261012`：Win10 `0cfd7e5a-4a1a-4777-9661-9bf7b833ccdc`，Win11 `2395f716-03c4-441f-aa0d-cd704520dde4`。
- v117 在上述最新实包上再次执行纯周期双服务恢复，Win10 284.636 秒、Win11 296.269 秒通过。门禁、原文件摘要、身份、服务配置、任务定义及应用健康均有断言。
- v118 并发红测发现另一线程持安装锁时自动初始化仍先检查文件；v119 将初始化门禁与安装放在同一持锁区间，忙碌返回延期。两系统全 agent 各 56 项通过。v120 正在重新构建锁修复后的真实包，尚未完成该包安装验收。

v120 编译和测试 MSI 打包成功：MSI `bfb3154498ff38b0a07f643799d92da6124267a8d5921456418bb70cce575329`，agent `00866e366c7cc222b39b4a4b34d8ab3938dd1c7dfb23cc00b76982ecdd155c1b`，DLL `f131d0a1857a177e61a82bf978a973fec8e64edbd592311a8f08d180b53d1bb5`。两 VM 再次从 v88 无 bootstrap 基线运行普通安装验收，尚待结果；此前 v117 恢复结果及 v119 测试结果已导出。

v120 两系统安装验收均已通过：安装器/实际 bootstrap 执行退出 0，自动初始化、安装锁、文件摘要、保护 ACL、固定 SYSTEM 任务、原身份、应用 ready 验证通过。快照 `AgentAutoBootstrapLocked-v120-20261012`：Win10 `6dd8202d-e7e3-4d5d-ac05-1a1e1544c2f8`，Win11 `dd968a32-2da6-483c-bd98-eebb3978ba74`。`auto-bootstrap-v120.result.json`、安装日志及状态均已导出。当前两个 VM 安装的是 v120 测试包；不要再以 v25/v65 的摘要作为当前运行事实。

## 普通 MSI 升级已有恢复入口（v121 起）

用户明确 OSS 为正式环境；本轮所有安装包、API fixture 和下载测试仅在本地隔离环境运行，不以发布 GitHub/OSS 作为测试前置。正式下载域名、公钥及在线升级方式保持既有契约。

- v121 Win10 从 v25＋有效 bootstrap 普通升级到 v120：MSI 退出 0，新 agent 替换，但 bootstrap 和清单仍绑定 v25，实际 bootstrap 退出 1。身份和任务未变，已导出失败证据。
- v122 新打包清单生成测试先失败，实现在包内 agent 上流式计算 SHA-256，standard/SOS 分别写清单；缺失、空文件或错误 edition 保留原清单。Win11 本地 Python 两项通过。
- v123 安装前检查 fixture 两系统各 11 断言通过。v124 新组件和自定义检查编译/打包成功，但两系统安装前均 1603：实际旧 ProgramData 父目录创建子项 ACL 被误认为可覆盖现有文件；未继续升级。
- v125 只读 ACL 诊断确认旧父目录允许创建子项，现有 agent/状态文件只读，bootstrap 目录保护。v126 先补 ACL 红测，v127 修复为区分文件/目录权限及 InheritOnly，父目录删除子项权限仍拒绝；两系统各 13 断言通过。
- v128 正在重新生成 MSI 进行已有入口升级验收。新增 RecoveryBootstrap 组件只在检查通过且非 helper 恢复安装时更新两个固定文件，清单伴随 EXE 的安装版本规则；已有任务不修改。尚未证明 MSI 自身失败回滚、新装无入口和 helper 事务兼容，不能列为完成。

- v128 两系统普通升级通过：旧 agent/bootstrap 为 v25，新文件均为 v120 `00866e36…`，清单一致、原安装身份和任务定义保留，实际 bootstrap 退出 0。v129 再确认应用 ready、双服务正常、无接续任务或维护标记。
- v129 快照 `AgentMsiBootstrapUpgrade-v129-20261012`：Win10 `c0f1f0ef-bb62-4c32-87a9-5abb6a2b2626`，Win11 `b55e0256-d402-4430-b00e-50c2c9d0cef5`。v128 MSI 摘要 `682a6f4da21ba2a01863496dceb48e9b6ebcf853fe55d383f6702ae0234dfd90`。
- v130–v134 故障注入驱动在执行安装前分别遇到 COM 返回值混入数组、整数列读取、序号占用、MSI SQL 排序和视图句柄未释放；均记录为测试准备失败。v135 已修复驱动，在 VM 内复制包、插入 InstallExecuteAgain 和最终提交前错误动作，正在验证文件替换后的 MSI 原生回滚。故障动作只存在于客体副本，不写入正式 WiX 配置。
- v135 两系统旧 agent/bootstrap/清单/DLL、产品注册和服务检查通过，恢复入口退出 0；但 v136 追加应用健康检查均为 unknown，v137 真实管道报 `health server executable mismatch`。因此 v135 只证明文件回滚，完整回滚验收失败，原 PASS 结果不能替代更高层检查。
- MSI 日志证明升级时创建的主服务在回滚替换文件时仍运行。v138 增加仅针对已验证 bootstrap 且原主服务 Running 的 MSI 回滚动作：先停止并等待固定服务宿主退出，文件恢复后启动原服务；仅接受固定路径、LocalSystem、自有进程且非 Disabled 配置。新增回滚脚本要求应用 ready，正在构建实包验证。正常执行和 helper 恢复安装不执行这两个 rollback 动作。
- v138 被 MSI 2613 顺序检查拒绝，未替换文件。v139 移除 RemoveExistingProducts 前的动作，仅在回滚开始时停止固定主服务，后续恢复沿用原有安装器/agent 路径。两系统实际故障安装后旧文件、产品注册、服务、身份及真实应用 ready 均恢复，bootstrap 执行 0。快照 `AgentMsiRollbackHealthy-v139-20261012`：Win10 `3becaf6d-6c7d-40f7-a9f1-7e91bcbd1302`，Win11 `1cc7c564-ebcc-45d8-9b88-b00d3552664f`。

## 本地 HTTP 与完整命令链（v140 起）

- 下载逻辑迁入 agent 专用 `package_download.rs`，生产仍由固定批准 URL 下载；测试专用 `download_fixture` 只接受回环地址。现有大小/哈希/不覆盖缓存的实现共用，未增加正式配置开关。
- v140 本地服务 socket 在 Windows 继承非阻塞模式，大包发送 WouldBlock，下载器拒绝截断内容，未安装。v141 显式改测试连接为阻塞并保留超时，Win10 188.54 秒、Win11 222.20 秒通过本地 HTTP 双包下载、验签、prepare/helper、真实 MSI、Committed/身份/健康/清理整链。两份包各 67,343,818 字节；临时下载缓存成功后删除，注册安装源保留。
- v141 快照 `AgentLocalHttpInstall-v141-20261012`：Win10 `d50d0e04-1450-49a2-881a-191310b6c2c1`，Win11 `eca816e9-22fb-4a77-8138-115e993ea91f`。必须保留的源目录为 Win10 `C:\RustDeskAgentLab\preparation-4860`、Win11 `C:\RustDeskAgentLab\preparation-5352`。
- v142 将正式命令轮询实现接入临时 PHP/SQLite/API 和回环 HTTP：测试独立身份/临时发布密钥，只有复制的 API 文件接受 fixture-key；客户端替换密钥/下载服务/prepare/helper 入口均为 `#[cfg(test)]`。正式公钥、API、下载域名不变。Win10 完整链正在执行，尚无终态，不能列为通过。
- v142 两系统完整本地命令链均通过，Win10 约 155 秒、Win11 147.01 秒。实际 PHP/SQLite 创建、授权、签名领取、目标和回滚双包 HTTP 下载、独立 helper 安装、终态补报都在同一用例中执行。SQL 精确断言事件为 receipt_persisted/running/succeeded 三条，序号 3、主心跳保持 123，重复轮询不重装且安装回执不变。尚不覆盖测试进程在补报前被终止或正式不同 build 升级。
- v143 正在两 VM 执行同链的目标验证失败：真实安装和健康验证完成后才注入失败，要求回滚文件/身份/应用并只补报一次 rolled_back。失败注入仅在测试编译和临时事务路径，正式程序没有此入口。
- v143 驱动实际匹配到零测试，按失败记录，未执行安装。检查发现同文件分段补丁未保留全部测试改动，v144 用单文件完整补丁重新编译，执行前先列出且确认唯一目标测试。当前两系统均已实际运行回滚用例，尚待终态。
- v139 日志已核对停止动作在文件回滚前执行且返回成功；不存在重新排入的回滚启动动作，原服务恢复由既有路径完成。包含真实应用 ready 的回滚通过证据已导出，不能用 v135 的仅文件 PASS 替代。
- v144 两系统实际本地命令失败回滚链通过：Win10 195.54 秒、Win11 211.72 秒。先确认目标已真实安装且健康，再注入验证失败，旧 agent/DLL 摘要和安装身份保留、旧包安装回执成功、应用 ready，API 精确三条事件 receipt_persisted/running/rolled_back，重复轮询不重装。快照 `AgentLocalApiRollback-v144-20261012`：Win10 `eee9377b-7e41-4923-993f-2ba8976d386e`、Win11 `2b9755d7-8645-4b72-923d-0c06342ebb6a`。
- v145 两系统正在验证实际进程交接：独立领取进程退出时 helper 尚未完成，新的报告进程及第三个重复报告进程读取持久 journal/事务/回执。测试临时设备私钥只写入 SYSTEM/Admin 保护的 fixture 文件，用后删除；正式客户端没有本地 API/密钥覆盖入口。尚待结果，不等于已验证断电或服务重启所有窗口。
- v145 两系统通过实际三个轮询进程的退出/启动验收：领取进程退出时 helper 尚未最终完成，补报和重复补报进程身份不同、原进程确认死亡，最终一条 succeeded 事件且不重装。Win11 165.25 秒，Win10约169秒。临时设备私钥文件已由测试删除；数据库、SQL断言结果和安装源保留。
- v146 两系统常规 agent 各 56 项通过，排除独立调用无行为的两个子入口，三项需要实包的 ignored 用例不计入。随后应用 ready、双服务正常、无 fixture/接续任务及 PHP/helper 测试进程残留。仍未覆盖真正断电、正式不同发布 build、EXE/SOS 或全仓门禁。
- v146 快照 `AgentLocalApiRestart-v146-20261012`：Win10 `da7d0b40-43b1-4e7b-8d6d-83b01e7006f2`，Win11 `d0ae2a2b-ee89-4c37-9a53-f881eeb99245`。Win11 快照命令超时后通过 CurrentSnapshotUUID 确认已落盘。
- v147 两系统无 bootstrap 基线的普通 v139 MSI 升级和自动初始化通过。v148 本地 HTTP/helper 同包安装两系统失败并回滚，v150 日志确定为 1316：测试基线使用 `agent-auto-bootstrap-v139.msi`，下载测试改名为 `agent-msi-bootstrap-v139.msi`；InspectBootstrapUpgrade 已正确跳过。v151 修复测试使用原包名，重新恢复基线、安装后重跑，尚待结果。生产 Installer 安全设置未变。
- v151 两系统按原包名执行本地 HTTP/helper 同包安装通过，Win10 202.46 秒、Win11 200.43 秒，Committed/身份/应用 ready/维护与任务清理验证。快照 `AgentMsiHelperCompatible-v151-20261012`：Win10 `71620fb2-fdbc-4d43-97b9-0b3492cbcb88`、Win11 `71dd2b70-aab9-4226-92bd-5da4c5e70a8c`。源目录 `preparation-2712`、`preparation-3076` 保留。
- v152 实际 MSI 边界六场景两系统通过：同包接管、maintenance.json、disable-agent、disable-recovery、损坏清单拒绝、重复安装。门禁下恢复文件字节不变；漂移时安装前1603且服务PID/状态/agent状态保持；最终应用ready。快照 Win10 `9f4cca22-136d-4287-9cd9-72eb52c9242f`、Win11 `049cf852-0a2d-46ec-8c49-1f3ce02a8964`。
- v153 正在独立临时源码副本构建 `2026100503` 测试包。正式仓库 `version.json` 原始 SHA-256 `eb3f40862196e9ce294a294f47415870a76a2fc59c183bec5193e2081a564664` 未变，正式 build 仍为 `2026100502`。只在本地测试使用，不发布 GitHub/OSS。
- v154 两系统本地 API/HTTP/真实 MSI 跨 build 升级通过：2026100502→2026100503，计划绑定新目标和旧回滚清单，两份下载内容不同；新 agent/DLL 健康 build 为0503，bootstrap摘要跟随。领取进程先退出，补报和重复补报由新进程执行，SQL只一条succeeded。Win10 240.33 秒、Win11 223.68 秒。
- v154 快照 `AgentCrossBuildUpgrade-v154-20261012`：Win10 `c8d87957-91ff-4bdd-bf48-1c397cb91dbc`、Win11 `02e70fd2-08af-43bd-b062-d53711dc73de`，超时后核对CurrentSnapshotUUID确认。源事务分别在 `local-command-3520`、`local-command-5080` 下，保留作为当前安装源。
- v153 新包 SHA-256 `af62de5a7edf73b8f8a515513b7680f8e48ed1c03bfe73a653efbc9f78b97782`，agent `091831fac3da7c33c27ac81672f5f7e8572fde17e1b7ce300d12217025f0d504`，DLL `8c523baf9937057c6d6260b0698963c544685c40d3bf6de485acad924143a9ef`。临时源码和封存payload在 `agent-cross-build-v153`，正式仓库元数据未修改。
- v155 正准备从 v152 旧build基线验证新版本实际安装成功后注入失败，再恢复旧build/文件/bootstrap/身份并补报rolled_back。尚未执行完成。
- v155 两系统跨build失败回滚通过：新0503真实安装且健康后注入验证失败，恢复旧0502 agent/DLL/bootstrap原清单和身份，SQL仅receipt_persisted/running/rolled_back三条事件。Win10 347.40秒、Win11 353.09秒。v157独立复核旧build健康、摘要、双服务Auto/LocalSystem、无维护/接续任务、PHP/helper残留，通过后导出数据库、精确SQL结果、计划/事务/安装回执与安装日志。
- v157快照 `AgentCrossBuildRollback-v157-20261012`：Win10 `02cf95f8-0893-4595-ba3c-4457b4eb71b9`，Win11 `e9a85518-6fbe-479a-87bd-cce1ae8c9b32`（命令超时后核对CurrentSnapshot确认）。当前已恢复build0502；Win10源为 `C:\RustDeskAgentLab\local-command-3580\RustDesk\ManagementAgent\transactions\9c2e4930ced59d6e9ed39facb7de130a`，Win11源为 `C:\RustDeskAgentLab\local-command-6324\RustDesk\ManagementAgent\transactions\420b51df908a9c67e38419fa2955eba6`，禁止清理这些已登记安装源。

bootstrap 联动回归面：`bootstrap_update.rs` 处理已存在固定恢复入口的准备/升级/回滚；`preparation.rs` 与 `native_install.rs` 在新 schema 2 事务校验并执行它；`bootstrap.rs` 与 `management_agent.rs` 增加首次初始化门禁和薄调用。schema 1 历史事务仍走原路径。普通 MSI 升级旧 bootstrap 已取得 v128/v139 实包证据；自动修复缺失恢复目录/任务和重建 agent 服务仍未完整验收。

本阶段生产回归面包含 `helper_runtime.rs` 的首次/接续停服前门禁与 `native_install.rs` 的受验证 v2 探测。`operations_runtime.rs` 的 v153–v165 改动仅扩展真实 PHP/SQLite 测试；无修改后台业务语义、正式发布能力或安装版本。不同本地测试 build 的完整命令安装链与回滚已在 v154/v155 验证；真实活动远控连接、正式 Release 四包、真正断电接续和生产发布链仍未验收。

## 专项全回归与终态接续（v158 起）

- v158 两系统各执行30个目标、224项通过，11项 ignored 单独列出。实际MSI资产提取用例显式执行，零测试视为失败；子进程测试入口不单独计作能力。v159独立复核双服务、旧0502应用ready、agent/DLL/bootstrap摘要、身份、无PHP/helper及接续任务残留，完整日志已导出。
- v159快照 `AgentRegression224-v159-20261012`：Win10 `a734767f-5c6e-413a-a781-01184d2dee90`、Win11 `b88d0d29-aa4e-4688-a05d-cd8a457df8aa`，快照命令超时后均核对状态确认完成。此快照尚不含v163终态接续修复的已安装agent。
- v159 `cargo fmt --all -- --check` 失败，存在跨平台与agent模块格式差异；没有全仓重排。v159全量Clippy首次缺CMake；v160补齐本机已有CMake/Ninja路径后停在 `libs/virtual_display/dylib/build.rs:1` 的冗余导入。v161定向Clippy仍编译RustDesk库，因637项诊断失败，不能称为agent静态检查通过；未为通过门禁修改无关代码或抑制诊断。
- v161 `cargo test --locked --workspace --no-fail-fast --no-run --message-format=json` 构建成功，共45个测试产物；仅编译，没有在宿主执行测试，也不能代替完整workspace执行验收。
- v162两系统实际红测确认：终态接续接受缺失error字段并重写结果，已回滚结果的原错误从非空变为null。v163仅在helper终态接续路径增加现存回执完整性/事务绑定检查，合法回执直接返回且保持原字节；损坏、缺字段、矛盾结果拒绝并保留证据，结果尚未落盘的中断窗口沿原恢复路径处理。两系统两项红测均转绿，附加缺六字段、七种不匹配/矛盾、坏JSON和缺文件边界断言。
- v164两系统完整helper各15项通过，真实子进程交接/接续、维护门禁、延迟释放与错误保留通过，应用ready，无残留接续任务。v165重新构建agent测试二进制，正在执行真实跨build失败回滚并附加终态接续不调用backend、不覆盖原始错误/回执的断言，尚待结果。
- v165两系统真实跨build失败回滚通过，Win10 375.73秒、Win11 332.19秒；回滚终态额外调用resume，明确断言零backend调用、原错误和回执字节不变。v166两系统新agent测试二进制常规各56项通过、5项ignored另列，复核旧0502健康、原身份、agent/DLL/bootstrap摘要、双服务Auto/LocalSystem、无PHP/helper/维护/接续任务残留。数据库、计划、安装回执、原始错误和日志均导出。
- v166快照 `AgentTerminalResume-v166-20261012`：Win10 `d82ae108-6e81-4184-80d3-b6ae274401eb`、Win11 `2b84876f-75d7-4f63-944c-0cc07090b28d`，命令超时后等待onlinesnapshotting结束并核对确认。已安装agent仍是旧0502基线，修复在新helper/agent测试二进制中完成实测；不得将此快照称为已安装最新生产agent。当前登记源为Win10 `local-command-4076\RustDesk\ManagementAgent\transactions\84f0a1d2620b39d72ead8dbcc1be0fd7`、Win11 `local-command-2844\RustDesk\ManagementAgent\transactions\bd170a9800c0213fc1a27a208f858fb0`（均位于 `C:\RustDeskAgentLab`），必须保留。
- v163生产回归面只改 `helper_runtime.rs::OwnedInstallation::resume` 的终态已有结果分支；原因是恢复时覆盖持久错误会破坏诊断证据。首次执行和非终态安装分支沿原逻辑。新增回归在 `tests/management_agent_helper_runtime.rs` 和 `operations_runtime.rs` 的测试模块。
- v166 `cargo build --locked --bin rustdesk-management-agent` 正式目标构建成功，仍有现存unused警告，不等于严格Clippy通过。v167在两VM列出workspace其余15目标，其中5目标确实无测试，不计PASS；含证书删除、外部设备查询和桌面捕获的测试需要独立验收。
- v168/v170两VM运行base、clipboard、defender_options、naming、portable-packer、windows_agent_guard六目标，各71项通过、1项ignored。最初驱动匹配guard子进程的1项结果少计一项，v170按最后父进程summary重新核对为2项，保留原日志。v169主库更新、policy、installer shell/handoff、目录备份命令、Defender mock、IPC八组各38项通过。不是完整workspace执行通过，未运行真实Defender配置或证书删除。
- v171控制台E2E未执行到页面，因嵌入Python路径未含已有Playwright包而import失败；v172显式只为测试进程添加 `api-tests/playwright-python`，使用当前API源码隔离副本与Edge重跑两条浏览器场景，尚待结果。
- v172两条控制台E2E通过（29.402秒），补齐此前重启取消未写数据库、确认创建精确命令、未声明能力按钮禁用的绿测证据。v173新增1440px重启确认回归，与390px共用同一断言实现，三条浏览器测试33.026秒全部通过，授权/撤销/离线/错误反馈/刷新和命令均直接核对SQLite。API本轮只改测试文件，业务页面未改。v174确认无PHP/Python/Edge测试进程，删除 `C:\RustDeskAgentLab\console-v171` 源码副本，测试自动删除临时DB；保留导出的日志及共享工具。

当前检查点仍未完成：EXE/SOS完整实际包矩阵、真正断电各阶段接续、真实活动远控门禁与会话锁、离线fallback自动资产准备、bootstrap服务重建/备份恢复、独立agent更新、全workspace执行/严格Clippy/格式门禁、覆盖率和完整容器验收。不得以已通过的standard MSI链路替代这些验收。

- v175开始独立开机归因测试：两系统原双服务Auto/Running、应用ready和身份/hash核验后，仅隔离VM将两服务临时设Manual并停止，保留原恢复任务；注册SYSTEM开机只读观察器，执行真实OS重启。要求任务开机事件118、恢复预算增加一次、两Manual服务恢复Running、身份/清单/任务未改，最终恢复Auto。当前已发出重启，尚待结果；不能把客体通道暂不可用或SCM自动启动算作恢复成功。
- v175首次开机恢复失败，v176/v178事件证据确认两系统均有329（超过任务执行时限后终止）。Win10首次为107周期触发，结果0x80070103；Win11首次为118开机触发，结果0x8007050B。现有任务限时PT1M；后续114触发实例约3.5秒退出0且预算各增加一次。首次失败与后续自动重试成功必须区分，Debug包冷启动耗时尚不能外推正式Release。
- v177独立核对并恢复两系统Auto/Running、旧0502应用ready和原安装身份；删除本轮开机观察任务、还原任务日志启用状态。原任务/安装源保留，失败事件已导出。v179仅在Win11测试现场临时将任务限时改PT3M，其余触发器、程序和预算不变，单台OS重启对照正在执行；生产源码未改，结束后必须恢复原PT1M与双服务Auto。
- v179单台Win11对照通过，约384.83秒恢复双Manual服务与应用ready；预算仅增加一次，原PT1M任务与双Auto已恢复。事件118与201绑定同一实例 `{a39bb36f-489b-4c01-8046-a24996bd0e45}`，从动作启动到退出0仅15.18秒。因此本轮不能证明PT3M是必要修复：并行负载也发生了变化。v181正在用原PT1M配置重做单台重启以消除该混杂因素，生产bootstrap源码/时限未改。
- v181原PT1M单台重启的观察任务从未运行（267011），不能计通过。v183系统事件RecordId证实实际重启，但开机时间比之前关机时间回退约三分钟，墙钟时间筛选/耗时会受影响；不把该现象直接归因为产品。双服务后来Running，v183恢复Auto、核对ready与原身份并删除观察任务。v184改用系统/任务RecordId确认新开机、用Stopwatch计观察时长，在Win10正常关机并确认poweroff后由宿主冷启动；原PT1M定义不变，尚待结果。
- v184冷启动已拉起agent，主服务仍在等待启动宽限；观察器已实际执行，使用事件RecordId下界筛选并绑定118→201同一实例，拒绝有329超时的结果。观察计时仅代表开始观察后的耗时，不代替完整冷启动总耗时；成功后恢复双Auto并保持原任务定义。
- v184双Manual服务自行恢复，但开机归因检查FAIL：实际为107周期实例 `{3169fe63-c282-4134-afee-9bef08f100b5}`，动作2.41秒退出0，没有匹配成功118实例，不能报BootTrigger通过。已恢复双Auto，保留原PT1M任务。开机首次超时与时间回退/观察任务未执行仍是验收缺口，未依据含混证据修改生产时限。
- v185两系统独立SYSTEM复核双Auto/Running/LocalSystem、旧0502应用ready、原身份和agent/DLL/bootstrap摘要、原恢复任务与任务日志启用状态；无本地门禁、PHP/helper及接续测试任务残留。精确删除本轮已完成v155–v184测试任务，保留已登记安装源、诊断日志和工具供后续验收。
- v185健康快照 `AgentHealthyEvidence-v185-20261012`：Win10 `d1354b93-985d-4892-a273-fdf1eb2ffb5b`；Win11 `1d6c8ad2-f59c-4190-b8c9-06334ead347d`，两条命令均返回100%成功。快照代表健康可恢复检查点，不代表开机/断电/四包整体验收完成；安装agent仍为旧0502基线，终态修复已在新测试helper和正式构建目标验证。
- v186独立源码副本 `agent-sos-v186` 设置 `RUSTDESK_SOS=1` 编译agent与Flutter Rust DLL成功，version.json仍0502且正式源SHA不变。v187封存payload并用当前MSI源码生成edition=sos包；打包元数据直接来自副本version.json/generated version.rs，不在宿主运行客户端查询版本。包SHA `b912548e12248e8950abae312afba156ac4b9aab7ee07b8c0a72b988855813a9`，agent `d46fcbc3d4554fea8d8bdfa0166cd69c0597f09484210572d6e50280e543f423`，DLL `4d0375fc508f3906836017f53f593a56c8443cdc3e99a0e209c4d7a1ff1ed23b`。
- v188两VM从v88无bootstrap旧standard MSI快照执行SOS安装，目标核对service/state/manifest/task/status的edition及旧身份保留。尚待结果；属于standard→SOS版本切换，不代表SOS全新安装、旧SOS升级或EXE矩阵已完成。当前target/debug已经被SOS副本构建覆盖，后续standard产物必须重新绑定源码/构建及摘要。
- v188两系统通过：SOS服务参数/state/清单/恢复任务一致，新agent/DLL摘要匹配，应用ready且保留原ID/UUID/installation_id，bootstrap执行0。v189同一SOS包重复安装两系统通过，清单字节和任务定义不变，健康与身份保持。v190扩展持久VM脚本的edition参数（默认standard），正在验证SOS双服务停止、本地禁用和周期自动恢复，尚待结果。
- v190 Win10禁用任务完成轮询未捕获瞬时运行而超时，明确记录FAIL并在v191恢复健康；Win11同轮完整周期恢复PASS，用时430.71秒。v192测试驱动使用Task Scheduler COM返回的InstanceGuid绑定201事件确认快速任务完成，临时开启并还原事件日志；Win10重跑禁用门禁/周期恢复PASS，358.79秒，原配置和身份保留。周期等待仍非手动触发，不修改生产恢复逻辑。
- v194两系统SOS健康复核通过：agent/DLL/bootstrap批准摘要、SOS清单/任务/服务参数、ID/UUID/installation_id、双Auto/Running/LocalSystem和应用ready，无本地测试门禁或helper/接续任务残留。本轮v188–v192测试任务已精确移除；当前注册安装源 `C:\RustDeskAgentLab\agent-sos-v187.msi` 必须保留。
- v194 SOS快照 `AgentSosMsiVerified-v194-20261012`：Win10 `18434fac-0932-4a49-9837-2ed966ff7aa5`，Win11 `e9e13ccd-8402-40a3-9bd1-1c6c0b701559`，均100%成功。当前两VM已安装SOS build0502，新agent包括终态接续修复；v185 standard健康快照可用于恢复。SOS首次全新安装、旧SOS升级/回滚、EXE及断电矩阵仍未完成。
- v195 Win11从 `CleanOS-DiskBoot-Offline` 干净断网快照首次安装SOS MSI通过：无原服务/ProgramData目录/任务，安装退出0，自动建立identity、双Auto/LocalSystem服务、SOS恢复清单和SYSTEM任务，批准agent/DLL/bootstrap摘要匹配且应用ready，bootstrap执行0。新身份按全新安装预期生成；已导出结果及MSI日志。快照 `AgentSosFreshInstall-v195-20261012` 为 `2871a6fb-086e-4556-a47e-b537e60eba48`。当前Win11处于这份干净SOS安装，Win10仍为v194升级SOS；两者安装源均为 `C:\RustDeskAgentLab\agent-sos-v187.msi`，禁止删除。
- 本轮新增回归面：`helper_runtime.rs`修复终态接续回执保留；`operations_runtime.rs`/helper测试增加实包和失败断言；`guest-agent-bootstrap-recovery-test.ps1`仅为VM测试新增edition参数和实例事件完成确认；API仅扩展桌面确认测试。正式版本、发布链和bootstrap时限未改。SOS全新安装当前仅Win11通过，旧SOS跨build升级/回滚、Win10全新安装、EXE和真正断电接续仍未完成。
- v196 Win10从 `CleanOS-LTSC2019-Offline` 干净断网快照运行同一v195全新SOS安装用例通过：安装0、双Auto/LocalSystem、默认恢复enabled、新身份、SOS清单/任务/批准摘要、应用ready和bootstrap退出0。前期处于锁屏/UAC未进入脚本的等待不计测试；实际SYSTEM测试从06:18:09Z到06:20:09Z。新installation_id为 `075c943c979685b89ff9f4515560cbda`。快照 `AgentSosFreshInstall-v196-20261012` 为 `a9b877e9-f942-402f-a413-ebbde0eb89be`，100%完成。两系统SOS全新安装现已通过，当前都处于干净SOS安装；旧SOS跨build升级/回滚、EXE及真正断电接续仍未完成。
- v197两系统全新SOS安装收尾健康复核通过并删除 `RustDeskAgentLabSosClean-v195` 和自删除收尾任务：双Auto/Running、应用ready、恢复enabled、SOS版型和新installation身份稳定，恢复任务/agent/DLL/bootstrap摘要不变。Win11新installation_id `dcc73dd2fba22bf04c7df497053ae2df`；两台仍断网，registered=false符合测试环境。上述v195/v196快照保留安装成功现场，当前运行态已额外清理测试任务；原包名与安装源仍保留。
