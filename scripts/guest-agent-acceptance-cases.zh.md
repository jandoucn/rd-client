# Win11 独立验收驱动

仅允许 `RDLAB-WIN11`、SMBIOS UUID `d91a13d9-2bb7-40bf-b5e4-9d57870cd242`、SYSTEM SID `S-1-5-18` 同时匹配。全部 PowerShell 文件使用 ASCII，兼容无 BOM 的 Windows PowerShell 5.1。禁止在宿主执行；宿主只传输文件和读取证据。当前固定基线为 SOS MSI build `2026100502`，installation `dcc73dd2fba22bf04c7df497053ae2df`；更换基线需要先协调、记录新批准摘要并复核驱动期望值。

## 调用与结果

在已核对动作的 SYSTEM 任务中执行，文件布局保持 `scripts/` 与 `tests/` 同级：

```powershell
# 每次命令都换一个唯一值；失败结果也不能覆盖。
$run='acceptance-'+[guid]::NewGuid().ToString('N')
& C:\RustDeskAgentLab\acceptance\scripts\guest-agent-acceptance-readonly.ps1 -RunId $run
& C:\RustDeskAgentLab\acceptance\tests\test_agent_acceptance_driver.ps1 -RunId ('fixture-'+[guid]::NewGuid().ToString('N'))
```

`C:\RustDeskAgentLab\<RunId>.result.json` 包含本轮驱动摘要、10 项实机检查的 pass/fail/unknown 计数、实际 status 输出路径、进程 PID 与创建 FILETIME、服务前后配置、安装摘要和身份、本地标记、任务定义、fixture 运行残留和事件日志状态。缺字段/查询失败/超时为 unknown，断言不符为 fail；超时保留自己创建的探测进程与 stdout/stderr，协调后再按 PID＋创建时间清理。超时不终止产品服务。

`pass` 仅表示本轮只读基线检查通过。`dimensions.scm`、`local_application`、`registration`、`remote_control` 独立；remote_control 恒为 not_tested。已安装文件摘要来自明确基线比较，不宣称完成正式发布签名供应链验证。任务存在及配置通过不代表开机/周期入口实际成功。

`*.tests.json` 是纯驱动 fixture 的实际用例数，`product_tests=0`。不能与 Rust 产品单元测试数相加。Rust summary 工具只接受最后父进程 summary、退出码 0、至少一项实际执行；ignored 不计通过。加载失败是 setup_error、executed=0。

只读脚本列出实验室任务供盘点；历史证据目录及安装源不是运行残留，不删除。需保留 `C:\RustDeskAgentLab\agent-sos-v187.msi`。禁止真实 Defender、停服、替换程序、恢复快照及断电；这些动作需主 agent 协调后单独执行。

## 首次开机 329 归因用例（待协调执行）

1. 记录 source commit、driver/package/agent/DLL/bootstrap 摘要、VM UUID、任务 XML、预算、双服务配置和安装身份。先完成只读基线。
2. 协调获得 Win11 独占窗口。记录 TaskScheduler/Operational 原启用状态，必要时仅为测试启用日志；不能用日志原本关闭后的旧事件推断成功。确认无其他 VM 高负载构建/安装，记录本机及宿主负载。恢复任务保留原 `PT1M`。
3. 为区分 SCM Auto 与 bootstrap，协调后将两个固定服务暂设 Manual 并正常停止；保存原配置，不调用 bootstrap 手动触发。注册一个唯一 SYSTEM 开机观察任务，验证动作与主体。保存下列 baseline，然后由主 agent 执行一次正常重启或冷启动。

```powershell
& C:\RustDeskAgentLab\acceptance\scripts\guest-agent-acceptance-boot.ps1 -RunId boot-baseline-<唯一值> -Mode Baseline
# baseline 必须在破坏性操作前外传；Mode Observe 本身只读，可在开机观察器或恢复后的 SYSTEM 任务运行。
& C:\RustDeskAgentLab\acceptance\scripts\guest-agent-acceptance-boot.ps1 -RunId boot-observe-<唯一值> -Mode Observe -BaselineRunId boot-baseline-<同一值>
```

4. 宿主使用 Stopwatch 记录发起启动、VM running、GuestControl 可用的区间；客体 Stopwatch 只表示观察器启动后的时长。事件只按本日志 RecordId 下界排序，不按墙钟筛选。核实一个新的 Kernel-General/12，日志未清空/绕回/截断，任务 XML 未变。
5. 第一条 118 必须和 200/201 使用同一 InstanceId/TaskInstanceId；同实例任一 329 判首次开机 fail，即使后来 114/107 或第二条 118 成功。没有 118、观察器未执行、事件缺失、日志关闭/清空均 unknown。`first_boot_action=pass` 只证明动作退出 0；还需预算精确增加一次、双 Manual 服务恢复、本地 ready、身份/hash 保持，才证明开机恢复。不得从后来 Running 倒推首次成功。
6. 420 秒恢复窗口覆盖 180 秒启动宽限与轮询；记录实测值，不把观察器延迟排除后声称达到开机 P95。至少重复三轮独立冷启动，分别记录冷缓存/温缓存和负载。PT3M 对照如获批准必须保持其余条件一致，最后恢复 PT1M、双 Auto、原日志设置，导出后精确移除本轮观察任务，再跑只读检查。

## 真正断电矩阵（尚未执行）

下列每项独立 RunId、事务/命令 ID、原包/新包摘要；断电由协调后的宿主 `VBoxManage controlvm <固定UUID> poweroff` 执行，必须观察 VMState=poweroff，再 startvm。正常关机、kill helper、进程重启不能冒充真正断电。恢复快照仅用于经协调重置下一用例，不能成为本用例产品恢复步骤。

| 用例 | 断电触发的持久证据 | 重启后必须断言 |
| --- | --- | --- |
| P01 领取前 | 本地 claim intent 已 flush，API pending | 同 ID 可安全领取；未提前执行，旧程序/身份保留 |
| P02 claim 后本地回执前 | SQLite accepted、本地 receipt 缺失 | 同实例幂等 claim，receipt_persisted 仅一次，不能重复变更 |
| P03 Prepared 交接前 | 计划/备份/任务完整，无 release | 不越过 handoff；旧服务可用或明确对账，禁止凭超时抢活进程 |
| P04 StoppingServices | release 持久、服务确实停止 | 独立接续重新验签/门禁/互斥，活动未知拒绝时保留记录 |
| P05 Applying 意图后安装结果前 | installer intent 含 PID＋创建时间，结果缺失 | unknown/pending，不盲目第二次 msiexec；记录旧可用数据和恢复限制 |
| P06 文件替换中 | 实际旧新混合字节和安装器活动证据 | 真实包语义接续/回滚，不能凭文件存在成功；记录确切终态与错误 |
| P07 Validating | 安装回执已 flush，健康未确认 | 只验证或回滚；不重装；旧身份、MSI 原 ProductCode/源包可恢复 |
| P08 RollingBack | 回滚安装意图/回执界面 | 回滚可接续，旧目录/agent/bootstrap/状态恢复，旧错误保留 |
| P09 终态回执后清理前 | execution-result 完整、维护/任务仍在 | 只收尾，原回执逐字节保持，零 backend 安装调用 |
| P10 清理后补报前 | 终态回执、outbox 已存，API 无终态 | 只补报，event_id 稳定；SQL 唯一终态，不再次安装 |
| P11 API 已提交但 ack 丢失 | SQLite 终态存在，本地未确认 | 重放同事件幂等；原服务/事务不变 |

每个断点必须有实际可观测证据，不能用 sleep 猜测窗口。现有生产程序没有已确认的断电屏障，P03–P08 需功能 agent 提供测试编译限定的暂停/flush 信号或可重复外部观测，再开始破坏性执行。超时未命中断点应标 BLOCKED，不执行随机断电后宣称覆盖。

每项成功、拒绝、unknown 均导出 journal、plan、transaction、handoff、安装 intent/result、原错误、任务事件 XML、SCM/进程创建时间、MSI 日志、SQLite 命令/事件/审计精确查询。对活动事务不能随意删除 marker/任务；完成受控恢复后才清理本轮资源。原安装源、唯一回滚包和失败证据保留。

## 功能回归要求

- 下载镜像：主成功不访问镜像；HTTP/断流/大小/SHA 错误后同域备用路径；零字节/上限/查询/转义/跨域/重定向拒绝；缓存占用与中断残留保留；MSI 原文件名不变；所有镜像失败保留旧可用包。区分本轮新增镜像行为与原先既有缓存失败语义。
- 批量 API：使用真实客户端 wire status（不要 fixture 私自补客户端没上报的字段），授权/撤销/rebind、批量权限和本地门禁、100 台边界、跨批与批内并发/速率、失败阈值、暂停/取消、重复 claim/事件、终态后历史事件展示。全部核对 SQLite 结果。
- 页面：1440/390 宽度、确认取消不落库、实际失败反馈、刷新/关闭/快速切换批次竞态、历史空态和旧客户端能力。通过测试之外的未实现功能应作为矩阵缺口，不能误报本次 bug。
- 未覆盖：Win10 本轮执行、SOS 旧版跨 build 升级/回滚、EXE 两版型、真实远控画面/键鼠/提权、完整容器重启持久化、全仓门禁及覆盖率。
