# T01 producer：待 Win10 RED 阶段交接

## 最新：2026-10-16 RED 后实现候选，待 Win10 GREEN

已读 `TEMP/opencode/red-lanes-win10-20261016/交回.zh.md`：旧EXE正式输入前置通过，1 failed/raw exit101，命中Err(Unimplemented)；旧数据保全通过。以下新制品取代下方历史RED制品，不能混用摘要。

- 新EXE仍在下方同一绝对路径，SHA-256=`5b92d42749d665db8c267bf4214fcb95c533106e374a81de0fa22a3e6b225290`。
- 模块SHA-256=`a20280c5808a3d24553e6f2708b6790f15d25f5499fabc329e391bed6f0639d1`。
- Rust测试SHA-256=`27894343a0473a60f1c9d90c94604d18860fceac6037c7321a480bc866e382bd`。
- 最终host `--no-run`完成23.82秒；限定两文件rustfmt --check通过。中途一次60秒调用超时、无遗留cargo/rustc且无完成记录，核实后重新启动构建得到本次完成回执。未运行host测试或--list。

Win10 owner复用SOS正式输入及原环境变量，串行执行：

```powershell
$env:RUSTDESK_T01_PRODUCER='red-formal-sos-1501'
& '<新测试EXE>' --ignored t01_producer_ --test-threads=1 --nocapture
```

必须实际执行8项，不是全局所有导入测试。需要先精确复跑旧RED时，原 `--ignored --exact t01_producer_formal_exact_build_publishes_schema1 --test-threads=1 --nocapture` 仍有效。增加：发布前marker/已有批准、低于签名build的current拒绝、真实注册查询对零ProductCode拒绝、同reference漂亮JSON保原bytes、stop-service/缺state/已有rescue保全。测试身份密钥对仅在测试fixture生成，正式发布验签仍只用既有生产公钥；身份配置比较失败只输出固定脱敏消息。

实现使用固定生产root与固定profile，现有root/state必须可信且存在，不创建state。持有目录句柄、state/identity/config只读锁和安装互斥；检查本地marker、恢复启用标记、用户停止、启用身份及事务/rescue占用。事务目录非空即保守Busy，不解释或修改其内容。验签后精确build相等；生产wrapper真实调用installed_source，绑定ProductCode、basename、MSI产品及注册包大小hash，包锁保持至函数返回，发布前再次查询注册源。create_new同目录临时文件、sync、MoveFileExW WRITE_THROUGH且无REPLACE_EXISTING；冲突不覆盖，已有同reference只读返回保留原字节。

测试bound路径注入仅cfg(test)，用于独立文件集成；真实注册负向另走真实installed_source，不能将metadata注入正向当生产注册正向通过。生产wrapper尚未接线；真实已安装ProductCode正向、ACL/reparse故障、竞争落点实际no-replace及磁盘故障、产品入口E2E和分支覆盖率尚未取得。不能标GREEN、闭合或80%。主协调在模块VM通过后再接普通安装完成→实例/注册源建立→首次prepare前入口，须在既有InstallGuard之外调用以免重复互斥；远程事务沿用after_commit。生产wrapper自身获取安装互斥。

本轮只修改独占模块、对应Rust测试和本回执；共享调用点无写入，无VM/CI/提交/子代理。以下为历史RED交付记录。

2026-10-16。仅构建未运行（包括未执行 --list），尚无 RED/GREEN 回执。当前 producer 明确返回 `DeliveryError::Unimplemented`；VM owner 确认业务断言失败后才能替换 stub。不得将环境/输入失败或零匹配作为 RED。

## 制品绑定

- EXE：`C:\Users\Administrator\AppData\Local\Temp\opencode\exe-transaction-v218-target\debug\deps\management_agent_first_install_approval_delivery-77c31a89923d1b9b.exe`
- EXE SHA-256：`284deedecfba62d5df5f33e5135d602b437dc9eadec00ac91fd21ed730104cba`
- 新模块 SHA-256：`4fcb2384dad4d0bbfba35fc10b31a9439f1a1a0214936517b294cbe9abb00650`
- 新 Rust 测试 SHA-256：`9671472b04b33a9d985d1f93493d113d7749de493277fe9caaf5b9382595c419`
- 构建：`cargo test --offline --locked -j1 --test management_agent_first_install_approval_delivery --no-run --message-format=json`，退出0，6.65秒，JSON最终 success=true。
- 日志：TEMP/opencode 下 `t01-producer-red-build.jsonl`、`t01-producer-red-build.stderr.log`；命令封存在 `t01-producer-red-build.cmd`。
- Rustfmt check 报告新测试文件格式差异，尚未处理；为本次冻结基线保留当前摘要，后续格式调整须重编并重绑摘要。

正式输入复用 `TEMP/opencode/t01-delivery-sos-input`，guest 固定为 `C:\RustDeskAgentLab\t01-delivery-input`。resolved SHA-256=`2ca7049c9dabda9899182184184cf4c6eeee6cfeb7604bbf54f1af71df42da45`，original-input SHA-256=`acb6f930b3384d5576d60351f3d18de08716ab614b28162e5253e6b95d1d974c`，本轮已重算匹配。fixture-receipt 原签名有效、SQLite正向及撤销409保全回执已读。测试运行时正式 gate 独立复验原签名及 MSI 字节；没有新增测试密钥。

## owner 执行

主协调串行交给 Win10 实验机 owner（`RUSTDESK_LAB_HOST` 等于本机 `COMPUTERNAME`，可选 `RUSTDESK_LAB_OS=win10`）；本任务不操作 VM。保持原输入目录，测试EXE放入独立文件名。示例中的 EXE 路径由 owner 按实际复制位置替换。

```powershell
$env:RUSTDESK_T01_PRODUCER='red-formal-sos-1501'
& '<producer测试EXE>' --ignored --exact t01_producer_formal_exact_build_publishes_schema1 --test-threads=1 --nocapture
& '<producer测试EXE>' --ignored --exact t01_producer_rejects_binding_failures_preserves_old_bytes --test-threads=1 --nocapture
& '<producer测试EXE>' --ignored --exact t01_producer_local_gates_preserve_old_approval --test-threads=1 --nocapture
```

每条精确选择1项。测试创建唯一 `C:\RustDeskAgentLab\t01-producer-red-<uuid>`，只读正式输入，写隔离fixture；不安装、不改变服务、不联网、不写真实 Agent 目录。保留打印目录及退出码/测试数/失败断言。首项预期在 `BUSINESS RED: legitimate signed input must publish approval` 看到 `Err(Unimplemented)` 对 `Ok(())`；其余两项先断言旧批准、state、identity、config原字节保留，再集中断言精确错误类型。

当前 registered_source 使用 cfg(test) 的隔离注入，MSI元数据与包字节真实；不代表 Windows Installer 注册源查询验收。identity/config 是隔离fixture，不代表实际 DPAPI/健康身份校验。后续必须补注册源、ACL/reparse、现有批准冲突、互斥、用户停止、原子发布故障、上限及完整真实入口 E2E；当前三项不覆盖完整目标，更不声称分支覆盖率达标。

## 主协调接线建议

生产入口 `deliver_installed_msi_approval(Request)` 只收原manifest字节、installation_id、固定MSI target、精确current_build和本地ProductCode。固定protected release-cache、state/identity/config路径及注册源查询由模块内部确定；路径注入仅cfg(test)。先校验本地门禁、固定MSI目标、正式签名与精确build、当前state身份，再核本地ProductCode注册源basename/MSI身份/锁定包大小hash；发布前再次确认保全条件。同目录create_new临时文件、sync后原子发布schema1三字段sidecar；相同绑定幂等，冲突保留旧批准/有效rescue。不要用latest猜批准。

由主协调在普通安装已完成、实例及注册源建立后、首次reader本次prepare前调用。`management_agent.rs` 和普通updater交接由各自owner接线；远程MSI事务继续commit→after_commit。当前没有生产module声明或调用点接线，不改变既有runtime路径、公钥或ready常量。

本次只新增模块及本前缀两个tests文件，现有共享文件均未写入。开工已核并发：无cargo/rustc活动，工作区其他修改保留。无VM/CI/commit/subagents。无task消息工具，阶段交接通过本文件及会话回报提供。AOCI工具、.mcp.json和正式索引不可用，未改写认知资产。
