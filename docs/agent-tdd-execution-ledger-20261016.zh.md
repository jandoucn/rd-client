# TDD 执行总账与接续队列（2026-10-16）

本账依据现有 docs、tests 交接和宿主 TEMP 回执持续整理；本轮只写本文件，未运行测试、构建、VM、CI、提交或子代理。已续纳 MSI present/missing 失败回滚、API 时间 UI 及 T01/T02 候选交接；各项以自己的回执时间和冻结为准，不设混合执行截止时间。**这是滚动检查点，用户要求继续接续，尚非最终交付。**

证据根 `T = C:\Users\Administrator\AppData\Local\Temp\opencode`。下列 `T/` 是证据定位简写；命令中的 `<EXE>` 必须由执行 owner 以对应冻结制品替换，不能直接执行占位符。owner/resource 是后续职责和资源要求，不表示本轮已占有 VM。宿主只整理/构建；客户端行为测试在独占 VM 窗口执行。

## 读账规则

- `status`：RED＝有效业务断言失败且尚无对应 GREEN；GREEN-local＝仅指定层级通过；REUSE＝历史冻结可复用；NOTRUN＝未执行；BLOCKED＝存在明确前置缺口；UNKNOWN＝没有找到足够回执。正确旧行为 baseline 可记 GREEN-local，但 RED 栏必须写“无，不制造 RED”。
- `sourcebinding` 只表示回执绑定的源码/制品。这里未重算整个工作区摘要，任何历史“current”均只指其冻结时点；HEAD、文件名 green、编译成功、设计 case、能力声明均不能代替实际执行。
- 每行独立 ID；RED/GREEN、子进程、重跑、包含关系、不同 OS/edition/freeze 分开。**不计算跨行总通过数，不把 6+1+3+双 token、13+ignored、API 与客户端、编译目标与业务用例相加。**
- E2E 必须注明实际产品、隔离替身或分段上传。快照恢复是场景清理，不是产品 rollback。失败须断言具体错误/状态与旧数据保全；旧错误仍保留。
- 本账中“未找到”只限本次查阅范围，保留 unknown；不推断证据从未存在。新制品必须重绑 source/artifact SHA、环境、测试名、raw exit、非零测试数、旧数据/清理与审查结果。

## 新计划 T01–T12

对应需求与全部待测规格见 [新计划](agent-recovery-tdd-plan-20261016.zh.md)。本表以独立切片表达当前执行事实。

T01 state-dir续接状态以T文件复核：旧 `t01-statedir-vm-stdout.txt` 只报告一个Win11旧stage binary case失败，不绑定C1910。Win10 runner脚本 `T/t01-statedir-rerun.ps1`计划C1910的positive和negative/lock两个exact case；最新handoff报告两次尝试遇parent UAC黑屏、结果缺失，但本次可读T目录没有candidate对应的逐case stdout/stderr和finally checkpoint/result。因此记为**reported attempted / terminal UNKNOWN（Win10）**，不写NOTRUN，也不把candidate报告启动、旧positive RED或consumer Unknown折算pass/fail；Win11也无fresh candidate result。T02 statefile ACL六项与bad-owner两项是不同契约的旧GREEN，不移植至此candidate。

### T01 state-dir candidate 执行态复核

旧RED的唯一实际测试输出仍为 `T/t01-statedir-vm-stdout.txt`，对应旧stage binary；它不绑定C1910。新candidate freeze `T/t01-state-acl-green-freeze-20261017/sha256.json` 只提供 EXE SHA `c1910d99…`、module `1aad8350…`和测试源 `fb190766…`。最新runner来源 `T/t01-statedir-rerun.ps1`含两个exact用例：

| candidate用例 | 本次新鲜VM终态 | 已核实来源/限制 |
| --- | --- | --- |
| `t01_producer_installed_state_inheritance_readonly_candidate` | **未找到对应candidate stdout/stderr/checkpoint/result，terminal UNKNOWN**；parent UAC黑屏/结果缺失的attempt报告不能据此写pass/fail；旧stage同名RED不移植 | 旧 `t01-statedir-vm-*`只绑定旧binary；C1910 hash见candidate sha256.json |
| `t01_installed_state_negative_and_lock_preservation` | **未找到对应candidate stdout/stderr/checkpoint/result，terminal UNKNOWN**；即使runner报告尝试启动，也没有fresh case summary可作RED/GREEN | 与positive case分别判定；不能用同一runner启动状态推导两项结果 |

本次继续保留 Win10/Win11 candidate 状态为待核，不用stage launcher输出、UAC截图、工具调用响应丢失或fixture cleanup回执冒充测试终态。旧stage RED继续留档；T02 state-file ACL回归是另一对象/冻结。

| ID / 范围 | status；RED / GREEN | E2E | sourcebinding / evidence | blocker | nextcommand | owner/resource |
| --- | --- | --- | --- | --- | --- | --- |
| T01-1 producer（T011） | 历史有效RED：0 pass/1 fail、exit101，`Err(Unimplemented)`≠`Ok(())`；对应正式输入发布case已在T01-5新冻结9项中通过，不另计一次GREEN | 隔离fixture，不代表产品入口接线 | `T/red-lanes-win10-20261016/{交回.zh.md,result.json}`；旧EXE `284deedecfba62d5df5f33e5135d602b437dc9eadec00ac91fd21ed730104cba`；新GREEN见T01-5 | 旧缺实现RED不套当前模块；产品安装入口及真实注册正向E2E仍未闭 | C21；进入接线/真实注册源正向验收准备 | producer owner；Win10/正式SOS输入 |
| T01-5 producer模块九项 | **GREEN-local；实际9 passed/0 failed/0 ignored/0 filtered，exit0，9.56s；P2独立review closed（session审查）**。八项候选与新增ACL由本次新冻结统一执行，不算8+9 | Win10管理员隔离fixture，正式发布输入；真实注册case为负向，不是已安装注册源正向E2E；产品仍未接线 | `T/t01-producer-acl-green-freeze-20261016/{vm-result.json,vm.stdout,sha256.json,交回.zh.md}`；EXE `9c825a65ef5710a7ebcc90d84eb46c2cfbf636b79978d6417ac23fa3babf7692`；模块 `a5904090aa3b47d731b506d1e2f9cffd421dd33dbbb4cdb669537657ec83a5f0`；测试 `d7b3dcc49a2425a2c48b3a69d2a8c19015c3f127dc553b367e8975c7e9e74dac` | P2关闭依据用户交接的独立工具session审查，无独立artifact，不伪造文件回执。T01 state-dir另有真实RED及C1910待两VM复测candidate；产品调用点/真实注册正向/完整初始化E2E仍未闭 | 本九项已完成不重跑；T01-STATE-DIR依C42单独验证 | producer/接线owner；Win10独占 |
| T01-ACL 独立单项 | **确证RED→GREEN；RED 0pass/1fail/0ignore/8filtered，worker exit=null（不补101）**；新GREEN中的ACL返回 `Err(UntrustedPath)`且保全通过，已包含T01-5的9项 | RED阶段00–11实际完成：弱ACL接受、严格ACL拒绝、producer错误返回Ok、保全完成后业务断言失败；最小修复只在existing读取前加严格verify_file | RED原件 `T/tdd-acl-v2-actual.stdout`、`T/tdd-acl-v2-actual.stderr`；RED模块 `a20280c5…`→GREEN模块 `a5904090…`，测试 `d7b3dcc4…`保持；GREEN原件同T01-5，窗口12:29:58Z–12:30:09Z、changed=[]/services_match=true | 原start-only场仍UNKNOWN，不用新场补旧退出码；setup AccessDenied不是RED。ACL P2独立review closed为session工具审查（用户确认、无artifact），不扩大成完整T01闭合 | C26仅核证据与绑定；不再把此ACL列为待RED或重复累计单项GREEN | producer/evidence owner；宿主只读取证 |
| T01-STATE-DIR installed inheritance | **旧stage binary Win11真实RED 0pass/1fail/exit101；当前c191正向candidate与negative第二case均NOTRUN/UNKNOWN**。两case的UAC确认后均未产生state result，未取得业务GREEN | 旧RED实际为已安装state目录inheritance导致strict-directory拒绝、producer `Err(UntrustedPath)`并保全旧数据，来源明确绑定旧stage测试binary；不将此结果算c191 candidate执行。当前只读检查确认 guest 无 `tdd-state-dir` receipt/stdout/stderr，services 为 Running；UAC截图仅证明首次可见，确认后没有结果文件，不据此判通过 | 旧RED `T/t01-statedir-vm-{stdout,stderr,result}.txt/json`；候选 `T/t01-state-acl-green-freeze-20261017/sha256.json`：EXE `c1910d99b7c2b41f06f9021fd9ce498d5b3ee899c7db0e90756a4a19312f7bad`、模块 `1aad8350241d8b9dd7c54a3539e2317abad6fb3d8cea782debcba7a5bf814104`、候选测试 `fb190766…`；未找到当前两case VM result。历史 c191 signed earlier VMSessions 的疑似执行，仅在找到对应真实 T 证据原件后登记；当前未找到原件则明确 UNKNOWN | 旧RED成立不等候选修复；不重复UAC。T02 state-file ACL 6项组与bad-owner 2项组是另一reader/文件freeze，不能移植或合并 | Producer/ACL owner先核新测试确切case及candidate摘要，再分Win10、Win11分别保存执行结果；当前回执优先转其他任务 |
| T02-STATEFILE-ACL RED→GREEN | **RED 8项中4fail→GREEN六项组6pass＋bad-owner组2pass；分别是独立真实VM回执**；首次bad-owner setup 1307为setup失败，不算业务RED；随后另行以实际当前SID运行两项均pass | driver/production reader分别验证state file的reparse拒绝、Users readonly接受、Users write拒绝；坏owner为独立组 | `T/t02-client-terminal-20261016/state-acl-red/vm-result.json`；GREEN `state-acl-green/{vm-six.result.json,vm-owner.result.json,vm-six.stdout,vm-owner.stdout,delivery.json,check.json}`；六项EXE `c47db21e…`、runtime `bd236a22…` | 仅agent.json state file契约，不覆盖T01 state directory inheritance；不是Win11新baseline安装后的复测，也不能与T01 candidate合并计数 | 此切片已完成，不重复跑；T01 state-dir由独立owner复测 | state ACL owner；Win10 |
| T01-2 rescue snapshot | GREEN-local baseline；无修复 RED；**13 passed/0 failed/1 ignored**，exit0 | UUID fixture/真实锁与文件信任；非完整初始化 | `T/rescue-window-20261016/交回.zh.md`；EXE `d19f4e11…`、source.json `f6acbf52…` | ignored Z-R06 绑定契约未定；source_unreadable 真 token 拒绝、完整资源准备及覆盖率未闭 | C03；ignored 不随常规过滤器计通过 | rescue owner；Win10 SYSTEM |
| T01-3 正式输入/篡改 | GREEN-local baseline；SOS/standard **各精确1 pass**，exit0；无业务 RED | 正式 key/包字节真实，未调用 installer | `T/t01-tamper-win10-20261016/交回.zh.md`；EXE `21d993bd…`；两份 PHP SQLite 正向/撤销409见 `T/t01-delivery-{sos,standard}-input/fixture-receipt.json` | 不证明 producer/安装/初始化；早期缺 manifest 与编码故障不是 RED | C04；两个 edition 串行、分别核输入 | 输入 owner；Win10 管理员/隔离 SQLite |
| T01-4 普通 MSI 缺批准保全 | NOTRUN；设计＋no-run，不是 RED/GREEN | 普通同版本重装观察尚未执行 | `tests/management_agent_first_install_delivery_contract.zh.md` | 缺批准等待本身合法；需独立安装基线、审查、安装后 worker 新报告 | C05；安装 owner 先满足交接前置 | 安装 owner；Win10 SYSTEM/固定 MSI |
| T01-COORD 协调/producer回归 | **GREEN-local：coord2pass/0fail/exit0；producer9回归pass/0fail/exit0，同09140 freeze分场**，不计2+9总数 | exact manifest调用fetch=1/deliver=1；不可用/非精确继续等待并保留旧字节；隔离协调边界非真实生产worker整链 | `T/t01-coordination-green-freeze-20261016/{vm-coordination.result.json,vm-coordination.stdout,vm-producer.result.json,vm-producer.stdout,sha256.json}`；EXE `09140d2110042a45e0eaf838e049e56941c8c8a868f1ac9c63eedd2d6fcac8c6`；模块 `ab254a4bcecf3438599bb63435bc5172e68cf053ad966dbb9e584cdafd80721a`；测试 `c291456e…` | 两场changed=[]/services_match=true，分别1.70s/9.19s；九项是已有producer回归，不追加到T01-5。HTTP预算修复另freeze，不能外推本freeze全绿 | C27原过滤器；下一缺口C32 gapworker | coordination/worker owner；Win10 |
| T01-HTTP 总预算 | **RED1fail/exit101：2050ms仍Ok(Delivered)、chunks20/delivered1→GREEN两case pass/exit0** | 实际本地HTTP：fast12ms Delivered；silent_headers414ms、oversize_length11ms、oversize_chunked12ms、headers_then_slow_body416ms、slow_chunks416ms均WaitingSource/delivered0；旧字节保全，不把子场景计新父case | `T/t01-http-budget-red-freeze-20261016/{vm-result.json,vm-stdout.txt,sha256.json}`：EXE `8799aac0…`、模块 `394cbe15…`；`T/t01-http-budget-green-freeze-20261016/{vm-result.json,vm-stdout.txt,sha256.json}`：EXE `90c5134197bc1d4564644a0bd09262a4d9c659f2a180c386a548e37ed4825bac`、模块 `b069c2e1ebb5720fa5cf241e0451be47e21a0bf03d674e4946cfb3c8de0949d5`、测试 `ca9279e0…` | GREEN共3.39s，changed=[]/services_match=true；416ms为本fixture预算观测，非生产固定超时承诺；未完成注册源正向/worker产品接线 | C28精确HTTP两case；C32 gapworker | HTTP/worker owner；Win10/loopback |
| T01-GAP-WORKER | **NOTRUN/待接线验证**；协调2及HTTP2不等于生产worker已交付 | 已有worker提取候选用layout/prepare/workspace替身；available-manifest来源边界须重绑固定API协调契约 | `T/t01-worker-wiring-red-20261016/{management_agent_first_install_worker_delivery.zh.md,extraction.json,sha256.json}`；EXE `0c6abf1b…`；源rescue_first_install `1a500f89…`、worker片段 `88a571f8…` | 旧候选handoff“coord未实现”已过时，但没有新worker执行result；不能仅因manifest可用判生产RED。当前接线状态须源码绑定后证明 | C32精确旧候选入口，先核新worker提取/source及边界 | worker owner；Win10独占 |
| T02-1 API | GREEN-local baseline；既有6与补充6分别通过；无有效修复 RED | 本批真实 loopback HTTP/签名/SQLite，未跑UI/installer；后续UI独立见T02-UI | `T/t02-api-host-20261016/T02-result.zh.md`；API `20df0e7c…`；index SHA `a0d049e0…`、lib `8b4b6959…` | 同command多attempt字段未落库，现为GAP，非预定RED；客户端ACK/安装整链未闭，UI绿不补该缺口 | C06（先重建已删除 PHP runtime） | API owner；宿主隔离随机端口/独立 DB |
| T02-2 普通 MSI terminal（T024：4父case） | **RED；0 pass/4 fail/1 ignored，exit101；未 GREEN**。缺 durable intent、退出后收据、丢 ACK 终态、失败历史；child 的两次 pass 不算父契约通过 | 生产函数提取＋installer/reporter 替身，无真实 MSI/HTTP | `T/red-lanes-win10-20261016/{result.json,交回.zh.md}`；EXE `d3fbb59a8ddffa501210662dcde03205104fb69205f66860fc3ae6c5fb3d1a3b`；`T/t02-client-terminal-20261016/build.json`，updater SHA `097ab587…` | 普通 MSI 无可重启补报的安装 outcome；首个缺收据断言后续流程未执行 | C02；实现后重冻生产函数/consumer，再跑四父项 | updater owner；Win10 独占 |
| T02-3 persistence/writer | **GREEN-local；14父项pass/0fail/2ignored，exit0，0.38s**；子进程通过不另计，旧9/10父项不累计 | 实际隔离文件/跨进程writer存活fixture；未接产品writer CLI/真实MSI监督、新server健康和持久ACK整链 | `T/t02-win10-writer.result.json`、`T/t02-win10-writer.stdout`；source见 `T/t02-client-terminal-20261016/writer/delivery.json`；EXE `ccbae932b72f85ca1a048857f291425ba5a213241da2d819518c2bcc06a89bbb`；receipt `6a8ad4bd…`、测试 `06b52169…`、当场updater `0ede2f26…` | result中changed=[]/services_match=true；旧delivery的blocked是运行前状态。原4 durable RED仍未prodclosed；prepare子进程退出不等于真实MSI杀旧worker场景 | C22记录已执行；后续C31 prodreconcile | updater/installer owner；Win10隔离fixture |
| T02-4 wiring原六项RED | **有效RED：2pass/4fail/0ignore，exit101**；后续ACK及runner分别见T02-ACK/T02-3010 | 原生产函数提取＋loopback HTTP/真实batch子进程，非真实MSI | `T/t02-win10-wiring.{result.json,stdout,stderr}`；`T/t02-client-terminal-20261016/wiring/delivery.json`；EXE `0f628d331d3c83bc8a263acb93df8b47b2fe03bc274b2e0f2d4e8a2c51811ec3`；提取 `2310e94e…`、installer_handoff `8d8ca560…` | 不把后续不同freeze拼为本轮全绿；PHP正ACK为201 `{ok:true}`；无command事件幂等契约尚未扩展 | C23仅保留精确复现入口，后续C31 | updater/installer owner；Win10/loopback |
| T02-ACK 七项 | **GREEN-local ACK7；原八项整轮为7pass/1fail、exit101，不是全GREEN** | 生产普通事件report函数＋真实loopback HTTP；runner一项报 `fixture elevated exit code -1073741515`（DLL fixture阻塞），不计1603产品RED | `T/t02green-t01coord-wiring.{result.json,stdout,stderr}`；`T/t02-client-terminal-20261016/green/delivery.json`；EXE `fb21d0e2c75d20aabbbb1dbe6bf36785a8d2c894a0165f3e21a98b573c220698`；updater `af24546c…`、windows `0e4f379d…`、测试 `9f742a08…` | 拒绝非正ACK不等于ordinary outbox已接线；changed=[]/services_match=true仅该场保全。DLL缺依赖与后续3010业务RED分开 | C29精确HTTP过滤；C31继续prodreconcile | updater owner；Win10/loopback |
| T02-3010 runner返回语义 | **RED 3010单项fail＋1603pass，exit101→GREEN2pass/0fail/0ignore、exit0，review closed（session交接）** | 静态CRT harness/stub实际执行生产update_me_msi生成batch；3010映射成功返回，原RD_MSI_EXIT保持；1603错误保留。非真实安装/重启 | `T/t02-client-terminal-20261016/reboot-red/{vm-result.json,vm-stdout.txt,vm-stderr.txt,delivery.json}` EXE `f02a402b…`；`reboot-green/`同名回执，EXE `cc8d80d6c5df11540bd812a0ea9d9b00cfb240806db8ad8c1169dce025b1695e`；windows `0e4f379d…`→`c991032c…`，测试同 `afa07fc2…`，updater同 `af24546c…` | GREEN0.24s、changed=[]/services_match=true；review closed按用户会话确认，无本次独立artifact；未持久化真实3010 outcome，原四durable RED/产品reconcile未闭 | C30登记精确两项；C31新增生产闭环缺口 | installer/reconcile owner；Win10 |
| T02-5 五父项重编候选 | NOTRUN；原4RED仍未接线，新增“无rollback证明应failed”待RED | 旧生产update_new_version函数体未改；不得用持久层通过替代 | `T/t02-client-terminal-20261016/{current-red-build.json,wiring/checkpoint.zh.md}`；EXE `6697f5255bed58adc941e1b5f7ef6b39fdd75591a6f7fe625d46fd884768dfae`；函数 `4b6022bd…` | 预期5父case＋1ignored child不等于5已RED；新SHA不覆盖原d3fb四项实际记录 | C24；保存逐父case/raw exit/child归属 | updater owner；Win10管理员 |
| T02-UI 时间/事件展示 | **GREEN-local；独立review closed（用户最新确认），原件核验单列下文**；原浏览器39检查31pass/8fail，时间新测试旧UI两项失败→新UI三方法通过 | Edge155＋真实PHP/SQLite/login/1440与390/reload；**104次时间矩阵、39项事件检查、6项邻近回归分别green，不合计**；SQL保持 | 已读真实T文件：`T/api-ui-host-20261016/{report.zh.md,actualcases.json}`；`T/api-ui-time-fix-20261016/{report.zh.md,actualcases.json,green.stderr.txt,regression.stderr.txt,source-sha256.json}`及event-regression/actualcases.json；四文件完整绑定见下表 | closed仅适用该四文件冻结；本次未定位独立review/source-final原件，不冒认已读或重做review。received_at/heartbeat不是采样；不证明客户端发送新字段，无JS分支/生产容器验收 | C25；补原件身份/摘要核验，不重跑104/39/6或重开已确认review | API UI/review owner；宿主只读 |
| T03-1 停止意图/身份 | GREEN-local/REUSE；停用标记覆盖修复，70事务＋8身份为各自回执，不合计 | 服务/健康替身 | 新计划 T03；`T/legacy-guard-20261016/`、下方 T03-2/3 | 最新真实成功＋提交前失败仍需绑定；未知映像/PID复用不得放宽 | C07 先读本批 source/结果，再选精确场景 | EXE owner；Win10 串行 |
| T03-2 旧真实 SOS rollback | REUSE；真实新 app/agent1501 ready、bootstrap 同步后固定 throw；内部78/rolled_back，旧六摘要及服务恢复通过 | **真实联合事务 rollback 已执行**；旧0510仅 root 文本，不证明旧 JSON ready/远控 | `T/legacy-realrollback-sync-1501/checkpoint.zh.md`；agent `03884bdf…`、DLL `4693f9b8…`，冻结同步 hook | generic stderr 未持久完整原错误；无 update-event/HTTP 终态；不覆盖当前 wrapper | C07，保留历史证据，不据此关闭 T03-3 | EXE owner；历史 Win10 冻结 |
| T03-3 当前真实 rollback | **BLOCKED/Pending75；无当前 GREEN**，尚未到健康后注入点 | SYSTEM 真实 runner；Stop-Main 身份检查拒绝，backup_complete=false/main_started=false | `T/legacy-realrollback-current-4d4cc3/checkpoint.zh.md`；wrapper `4d4cc3fa…`、agent `8d20eb12…`、DLL `b91dba96…` | 创建时间/映像子条件未知；proof 缺失错误不是注入原错误；快照清理不能补作 rollback | C07；先诊断封存 Pending，禁止直接重复 installer | EXE owner；Win10 独占、保护/失败快照 |
| T04-Z1 模型/投影 | GREEN-local baseline；新增**6 pass**与既有**1 pass**分命令，均exit0；无 RED | 注入 SCM/probe；真实 share_mode(0) 文件锁 | `T/t04z-final-20261016/交回.zh.md`；EXE `bca180de…`，health_observation `d3288bfa…` | 非真实 resident/fallback SCM；身份错误非真实 token 拒绝 | C08；同冻结两条原命令 | health owner；Win10 提升管理员（非 SYSTEM） |
| T04-Z2 blocking 负向 | GREEN-local baseline；**3 pass**，exit0；无 RED | 真实隔离 pipe：missing/malformed/另一存在 EXE 映像不符，原字节保全 | `T/t04z-blocking-20261016/交回.zh.md`；EXE `031d9a59…`，测试源 `2aa77b69…` | nonce重放/PID/create/退出/权限/busy/分片矩阵不足 | C09；独立 freeze，不冒认 Z1 重跑 | health owner；Win10 管理员 |
| T04-Z3 双 token fixture | GREEN-local baseline；server/admin/SYSTEM **各1 pass**，exit0；无 RED | 生产 serve/probe_blocking/application_snapshot，同 fixture owner 两侧 JSON 相等；ready/registered=false | `T/t04z-dual-20261016/交回.zh.md`；EXE `f3ee8bce…`，测试源 `7f363145…`；三批 status SHA 同为 `8eabddff…`但测试/EXE不同 | **不是 SCM resident 或真实客户端 E2E**；后台接收关联未闭 | C10；server/client 协调，不将 server 算新产品能力 | health owner；同 Win10 双 token 独占窗口 |
| T04-Z4 旧 CLI/真实 resident | BLOCKED；两 token CLI raw exit0、JSON解析失败，采集 PASS 非健康 GREEN | 旧0510只 root 文本；一次性 SYSTEM CLI≠驻留 payload | `T/rescue-window-20261016/{admin-live.json,system-live.json}` | 缺新协议真实安装基线、fallback SCM与后台同窗口采样；1501快照恢复失败不算产品恢复 | C07 核基线，再按新计划 T04-Z 取证 | health/部署 owner；Win10＋隔离 API |
| T04-1 R0–R6 恢复阶梯 | GREEN-local/REUSE，分层见 H-R0、H-R1-R2、H-R3、H-R4-*、H-R5-R6 | 有 R3/R4 实际场景；完整恢复后远控仍未闭 | `docs/agent-vm-system-driver.zh.md`；各 H 行证据 | 不把局部 ready/Running 视为整个阶梯验收 | C11，按层补受影响缺口 | recovery owner；单 VM 单 owner |
| T05-1 Bootstrap/agent兜底 | GREEN-local/REUSE；任务/修复/备份/周期有历史红绿 | 正常周期/部分真实启动；完整开机归因及新版缺 agent 链不足 | VM记录 v97–128、v210–213；`T/bootstrap-v213-review.zh.md` | 缺完整boot首段、实际当前备份/双停/门禁矩阵 | C11；先核 bootstrap-v213 绑定/任务与周期结果 | bootstrap owner；Win10/11 分场 |
| T06-1 12能力/命令 | GREEN-local/REUSE；MSI12/EXE9 snapshot 红绿，部分真实命令 | resident service.inspect 真 poll 已证；完整包升级后常驻整链未证 | VM记录最新能力/agent.update/resident条；`T/resident-agent-lab/acceptance.json` | capability/权限开放≠12项均执行；client.enable 缺能力仍拒绝 | C11；逐 action 建命令→SQL→ACK→UI回执 | client/API owner；隔离 API、Win11 |
| T07-1 地址簿模型 | GREEN-local；有效 RED 8pass/2fail→四suite **30pass**；Ab10重跑为子集 | MockClient真实模型/widget，非连接 E2E | `tests/peer_online_reload_handoff.zh.md`；模型 `c5d04479…`，测试 `fd0549fb…` | 真实 session 自然 hash-sync PUT/reload、两online一offline、像素挑战/关闭未闭 | C12；真实 E2E先落实 native 网络出口隔离 | Flutter owner；Win10＋独立控制端/账号fixture |
| T08-1 MSI shortcut升级 | GREEN-local；隔离 MSI 有效 RED→9断言绿；repair两次为增量 | **完整 CA＋官方SOS1501 payload，SYSTEM/per-machine present/missing 两场真实升级 passed**；exit0/State5/四配置摘要保全 | `tests/windows_desktop_shortcuts_handoff.zh.md`；`T/shortcut-fullca-20261016c/{present-final.json,missing-final.json}`；lab MSI `d8771b1b…` | 非官方发布包；失败场景独立见T08-3/4；历史per-user Advertised失败仍保留 | C13；后续正式在线包/实际业务健康另验 | MSI owner；Win11 SYSTEM/保护快照 |
| T08-3 MSI shortcut失败present | GREEN-local 实包失败回滚；**真实exit1603/old_state5/phase=pass**，before_link=true→after_link=true | 完整产品实验包，InstallExecute后ShortcutLabFail return3；恢复snapshot前取证，受检bytes changed=[]/added=[]、旧ProductCode恢复 | `T/shortcut-fullca-20261016c/{rollback-present-final.json,rollback-present.log}`；失败MSI `90051585121df95802cc2b9e3934acd3ecccf4dba9af162b360f4496b0d7cb93`；原ProductCode `C220D796-2FB4-45F5-A921-87CF071C2E49` | handoff报告双服务Running；原JSON after.services仅列Agent Running，主服务after独立结构化记录缺口保留，不补造全量服务证据。未见1401/1406；非正式发布包/在线远控 | C13；审查服务原始采集边界，不重复计upgrade或快照cleanup | MSI owner；Win11 SYSTEM |
| T08-4 MSI shortcut失败missing | GREEN-local 实包失败回滚；**真实exit1603/old_state5/phase=pass**，before_link=false→after_link=false | 同SHA失败包独立场景，原ProductCode/受检bytes恢复、changed=[]/added=[]；恢复snapshot前取证 | `T/shortcut-fullca-20261016c/{rollback-missing-final.json,rollback-missing.log}`；同T08-3失败包；日志MainEngineThread1603 | handoff报告双服务Running，JSON after同样仅Agent；末取证仍见SYSTEM msiexec宿主PID2072，worker Wait/日志已1603，不冒认宿主完全退出；snapshot仅清理 | C13；保留present/missing独立回执及旧per-user失败 | MSI owner；Win11 SYSTEM |
| T08-2 EXE shortcut | GREEN-local；删除替换竞态、Unicode长脚本有效 RED→最终**9pass** | 原生临时桌面/COM/BAT；非产品安装 | `tests/exe_desktop_shortcut_handoff.zh.md`；adapter `7ff27801…`、EXE `da33cabf…` | 完整 lib 成功退出缺失；父进程/UAC/SYSTEM及安装rollback未验；就地写非原子保证 | C14；新构建绑定后实际父子安装验收 | EXE owner；Win11 |
| T09-1 Defender | GREEN-local mock/契约；真实 opt-in NOTRUN；历史异常 UNKNOWN | SOS单重启保护开启/排除为空仅该观察；standard true→false根因未知 | 新计划T09、`docs/windows-defender-install.zh.md`、VM记录重启条 | 五固定Path/零Process/CFA两EXE真实读回、策略拒绝和早期启动缺证据 | C15；只读采集后分默认/opt-in场景 | Defender owner；独立 Windows VM |
| T10-1 lock | NOTRUN；AST通过不算 RED/GREEN | 正常远控/可恢复登录前置及真实锁定未验 | `T/lock-logoff-driver/{static-check.zh.md,RUNBOOK.zh.md}`；driver `ae03693b…` | manifest默认两确认false；缺 OS锁定事实/握手/新帧/输入证据 | C16；满足 runbook 后由桌面 owner 执行 | 桌面 owner；Win10/11＋控制端协调 |
| T10-2 logoff/macOS | NOTRUN/UNKNOWN；无已核真实完成回执 | 注销独立于杀GUI；LoginWindow/TCC/FileVault独立 | 新计划T10、同 lock runbook、macOS handoff | 不用 WTSActive/请求成功/采样存在冒认动作成功；macOS真实环境未证 | C16；每OS/lock/logoff独立run | 桌面 owner；双机/适用macOS |
| T11-1 Purge marker/native | GREEN-local；marker7pass，历史DACL拒绝未执行MSI | 当前 native 已接可达入口；**完整真实 /x 注销仍未通过** | `tests/management_agent_purge_policy.zh.md`末节；`T/marker-takeover-a167becc466a42339026aa34b276ccf2/`；EXE `16f3ae6d…` | 不能照旧VM摘要写“尚未接生产”；新接线no-run≠验收；SCM/COM旧制品漂移，junction race未覆盖 | C17；冻结 marker/native/CLI 后先定向，再真实注销 | purge owner；Win11 SYSTEM |
| T11-2 Purge周期/注册 | REUSE 限定观察 passed；完整注销 BLOCKED | purge后周期/真实重启后未复活；早期boot约1分46秒缺采样；另查旧MSI仍注册/ARP存在 | `T/purge-win11/验收.zh.md`、`T/purge-registration-win11/`；VM记录 | 不复活≠MSI注销；native新路径不能继承旧观察结论 | C17；保留失败，验证 ProductCode/双ARP/任务服务目录 | purge owner；Win11保护现场 |
| T12-1 中断/断电 | REUSE 部分实际断电；新增路径 NOTRUN/BLOCKED | v203 P09/P10/P11、v209 P09b历史可复用；非全矩阵 | `T/acceptance-ledger-20261014.zh.md`待办4、VM记录 | 最终全失/事件/agent.update安装意图与终态cleanup窗口缺补测；被拒操作保持阻塞 | C11；先核历史制品，制定受影响窗口清单 | 事务 owner；独占VM/故障窗口 |
| T12-2 严格门禁/覆盖率 | BLOCKED；47业务case通过≠分支80%；56/56仅no-run | Flutter172为单元/widget；桌面全链未闭 | 同旧ledger、VM记录；coverage branch records=0；统一freeze `34c9cb79…` | fmt/strict Clippy/analyze未绿；旧workspace WebRTC/doctest失败；当前整仓执行未知 | C18 分别执行并保存真实结果，不以no-run替代 | 门禁 owner；冻结构建环境/VM |
| T12-3 跨平台/包选择 | NOTRUN/UNKNOWN 最新整合；Windows局部实包可复用 | EXE/MSI/AppImage manifest选择＋可运行平台启动未全闭；macOS/Android不能由Windows替代 | 新计划T12、`docs/agent-recovery-macos-handoff.zh.md`、tests release/platform/signing入口 | 当前签名/七包构建及真实跨平台能力未证；Android仅standard；Management Agent现行Windows范围 | C19；适用真实目标各自封存 | 平台 owner；Windows/macOS/Linux/Android环境 |

### T02-UI 四文件冻结与原件可用性

以下四个完整SHA来自实际已读的 `T/api-ui-time-fix-20261016/source-sha256.json`。独立review closed采用本轮用户确认；指定的最新API review/source-final及总TDD manifest本次未在可读目录定位到，**不虚构T文件路径，不声称已核source-final相等性**。缺原件只记证据可用性待核，不据此改写已确认的审查结论。

| 受审对象 | 已封存 SHA-256 |
| --- | --- |
| sqlite/agents.html | `ece2f94d337275776281523fe6ace1a8530ac1b898f693b81f4ac46550cb73b6` |
| sqlite/devices.html | `199e8e7730fbba1308f9fa4e67aaeca12c2daef4c2f249f863dfe0909d967924` |
| docs/agent-observation-times.zh.md | `b3c1029e6fc782e09542f2347ea84833f4d9c8ebf86566435f69382388642820` |
| docs/agent-console-plan.zh.md | `80898666da479253334960933f7040bf59446a290fa99834149bf597b1017ed1` |

T01分场：旧缺实现 `284deed…` 的Unimplemented RED保留原始exit101；八项候选 `5b92d427…` 不冒认取得本次九项结果。最初ACL `T/tdd-main-acl-win10.stdout` 只有start1，`T/tdd-acl-readonly-evidence.json`终态UNKNOWN仍成立；后续独立v2场 `T/tdd-acl-v2-actual.stdout/stderr` 才证明1项业务失败，worker exit=null，不补101。最新GREEN `9c825a65…` 的vm.stdout实际列出9项并汇总9pass，vm-result.json明确exit0、counts_match=true、changed=[]、services_match=true；同目录交回仍写“待GREEN”是执行前状态，以后续实际回执更新。模块 `a5904090…` 的P2独立review closed采用用户确认的session工具输出，无artifact，不声称本轮重新审查。产品未接线、真实注册源正向E2E未闭；不把旧8项、ACL单项和新9项相加。

### 最新产品接线检查点（覆盖上表对应旧阶段，不覆盖旧冻结原始结果）

本节只纳入下列真实T文件；执行后result优先于同目录执行前handoff的“待VM”。T01-5/COORD/HTTP与T02-3中“未接线”仅描述各自旧freeze；当前已有接线候选，但**ordinary完整四项durable RED仍未prodclosed**。旧0510现场缺少合法installation_id是生产前置缺口，不能填空、猜身份或据fixture通过宣称支持旧0510。

| ID / 范围 | status；RED / GREEN | E2E | sourcebinding / evidence | blocker | nextcommand | owner/resource |
| --- | --- | --- | --- | --- | --- | --- |
| T01-REALWORKER | **GREEN-local baseline：1pass/0fail/0ignore、exit0、1.72s；一个case含两个场景**，不制造RED或按两场加计测试 | 真实worker/thread/channel→HTTP→producer→真实reader→报告；approved：正式签名/MSI校验后健康边界1次明确失败，`preparation_refused`保留`isolated worker health unavailable`、ready=false；notapproved：404/WaitingSource、无sidecar、reader waiting_approval、health零次；两场无current、state/身份/config旧字节保持 | `T/t01-real-worker-freeze-20261016/{vm-worker.result.json,vm-worker.stdout,sha256.json,management_agent_first_install_worker_real.zh.md}`；EXE `d18006f1147dc7df55d7d1397f5fe78df2eaf497c246a2932ba3daca669e6f75`；reader `568dc713d3032adc975ee492b1bb58bb48a3d2df632cd81e70ce0654d567707a`；producer `b069c2e1…`；测试 `3f824a59…` | changed=[]/services_match=true；cfg(test)替代root、SCM/注册源查询、健康及来源发现。非fake prepare，但仍非真实注册源正向/SCM健康/资源成功初始化/实际安装E2E；旧gapworker提取候选不得替代本场 | C33精确已执行入口；后续真实注册源/健康正向E2E | worker owner；Win10隔离fixture |
| T01-HTTP-REVIEW | **HTTP deadline P2 review closed（用户本次session交接）**，测试事实复用T01-HTTP，不新增通过数 | 仅总预算边界审查，不扩大为产品初始化E2E | 测试/source原件仍为 `T/t01-http-budget-green-freeze-20261016/{vm-result.json,vm-stdout.txt,sha256.json}`；producer `b069c2e1ebb5720fa5cf241e0451be47e21a0bf03d674e4946cfb3c8de0949d5` | 无本次独立审查artifact，不虚构路径或声称重新审查；closed只绑定该模块/范围 | 保留session来源；无需重复HTTP两case | review/evidence owner；只读 |
| T02-PRODUCT-WRITER | **GREEN-local：16父项pass/0fail/2ignored、exit0、0.35s**；原14项被包含，child不累计 | 真实Outbox/dispatch/report_pending文件与跨进程fixture；新增安装前bound intent/重复dispatch拒绝、3010 outcome＋terminal、部分ACK重开只重发缺通道/原terminal保持；不执行完整runtime OS/HTTP adapter | `T/t02-client-terminal-20261016/product/{vm-writer.result.json,vm-writer.stdout,delivery.json,checkpoint.zh.md}`；EXE `045a0718261810d8e1955b00ba58185cfde983936f849b6eaaa36e48514c146d`；receipt `3bd51fe280e35f86a8c839a5691036a2b195933c6b62486cba87b7b4f8a06786`；测试 `9b1ab7ba…`；runtime `4b6e8bdd…` | changed=[]/services_match=true；产品已新增普通MSI分派、固定outcome CLI、启动reconcile候选，不能继续描述成完全未接线；独立监督/自检/真实升级instance保持未验，完整四durable RED仍未闭 | C34记录16项；C31转完整runtime验证，C35先supervisor RED | updater/installer owner；Win10 |
| T02-ROOT-CHECK | **编译检查PASS：当前根仓库cargo check exit0；不是runtime执行GREEN** | 不代表完整EXE链接、Clippy、测试或安装E2E | `T/t02-client-terminal-20261016/supervisor-red/{check.json,current-check.stdout.log,current-check.stderr.log,checkpoint.zh.md}`；before/after一致，`source_unchanged_during_check=true/runtime_executed=false`；runtime `4b6e8bdd28273c095f63047c482a2db38b3716a42a45419eab52b68e0470b5a0`；updater `58217d67…`、core_main `e89bcf02…`、windows `ca8e413b…` | 保留既有warnings；仅证明本命令/feature/源码检查通过 | 已执行 `cargo check --locked --lib --features flutter -j2`；仅出现新漂移/失败再复查 | build owner；宿主构建环境 |
| T02-NONSEND-OTHER | **UNKNOWN关联／另一non-Send报错记录保留**；不被ROOT-CHECK绿覆盖，也不据其判4b6e8当前失败 | supervisor交接记载曾在async直接await probe持有ObservedProcess；另一owner确切日志/命令/feature/source未定位 | 实际可读来源 `T/t02-client-terminal-20261016/supervisor-red/checkpoint.zh.md`第5–9行；该记录与check.json分开，未伪造原error artifact | 当前probe在spawn_blocking内驱动已有Handle；尚不能证明另一失败是旧源码。禁止无依据unsafe Send或以成功check推断其他feature通过 | 补另一场原始stderr、完整命令/rustc/features/runtime SHA再对账 | build/evidence owner；只读 |
| T02-SUPERVISOR | **RED0pass/2fail、exit101→GREEN2父pass/0fail/1ignored、exit0、0.66s**；实际启动child1pass/0.16s是父测试内部执行，不计第三个业务case | 临时受保护runtime中复制EXE真实启动、自检/ready-release/lease使用生产代码；cfg(test)路由child并使用固定测试mutex，installer回调只发布1603 outcome；**无真实MSI** | `T/t02-client-terminal-20261016/supervisor-red/vm-result.json`；GREEN `supervisor-green/{vm-result.json,vm-stdout.txt,delivery.json,checkpoint.zh.md}`；EXE `000bf561638850fad6fb7dcfd9bcd5dc068b5caff87278e3d2058d49ef16e041`；receipt `a12c064f…`、runtime `2f9ed821…`、supervisor `bd7d571e3a1f4b10522c96d0efb8f50454b03b9841c46f3ec6cc3c5d2199be21`、测试 `0f9eeba2…` | 两场changed=[]/services_match=true；独立监督局部绿不证明正式Global mutex、真实MSI杀旧server、Flutter副本依赖或admission兼容。dispatch契约已收紧，旧product16不能自动复用为新freeze全量绿 | C35登记已执行；C37审查admission，C31产品runtime/outcome-writer E2E仍gap | supervisor/product owner；Win10隔离fixture |
| T02-PRELAUNCH-P1 | **RED2fail/1pass/1ignored、exit101→GREEN4pass/0fail/1ignored、exit0、0.21s；P1 static review closed（limited，session交接）** | 真实复制失败/锁/占用目标、typed preparation-failed收据与重开retry；写失败收据不可靠时persistence_unconfirmed仍阻塞；已dispatch未知不伪造准备失败。child入口被父调用不加计 | `T/t02-client-terminal-20261016/preparation-red/{vm-result.json,vm-stdout.txt,delivery.json}` EXE `dfc2e70b…`、runtime `4b6e8bdd…`；GREEN `preparation-green/{vm-result.json,vm-stdout.txt,delivery.json,check.json,checkpoint.zh.md}` EXE `de3c97082c42e2ee80498cfad7fac55e8aa60d2ce504a9bd4ef6676d00310de1`；receipt `98daaea1…`、runtime `3c048d2b5efcd08f047c618a5b12412db33205b4fab6457abe7445e8d5fe3493`、测试 `32edc81f…`、投影 `f894613b…` | changed=[]/services_match=true；static review关闭仅该P1绑定/重试判定，用户session确认无独立artifact；固定生产ROOT枚举、完整runtime安装/reconcile未验。intent后未写失败收据就崩溃仍未决；未删除历史/伪造outcome | C36登记精确已绿入口；C31继续产品E2E，不重开已闭局部P1 | runtime/review owner；Win10隔离文件fixture |
| T02-ADMISSION-CONDITIONAL | **disabled对照PASS：sequence_exit0/reached1、harness exit0；enabled持锁无permit确证RED：sequence_exit1603/reached空、harness exit101**；不是两场全绿 | 真实MsiOpenPackage/MsiSequence，独立线程持实际Global安装mutex；enabled执行生产CA、disabled无该CA；fixture无InstallInitialize/InstallFiles/注册表/服务动作，不是产品安装 | `T/t02-client-terminal-20261016/admission-red/{vm-result.json,vm-disabled.stdout,vm-enabled.stdout,delivery.json,checkpoint.zh.md}`；harness `53ba15952076a1898458753649a15cfc204db5dc5cd086c2b6cf87d56bb934df`；CA源码 `56b08974…`；disabled MSI `ee15210f…`、enabled `051e21d0…` | counts_match=true/changed=[]/services_match=true；ordinary lease无合法Agent permit与session challenge应答时不支持enabled准入。不得释放锁强装或伪造permit；目标与旧缓存包的RemoveExistingProducts兼容均未闭 | C37；先产品准入capability gate/拒绝保全契约，再真实兼容E2E | admission/product owner；Win10串行 |
| T02-ADMISSION-1501-TABLES | **静态disabled事实**，不计运行测试PASS | 精确正式standard/SOS1501 MSI中无准入schema属性、CheckAgentInstallAdmission CA及对应执行序列；JSON空数组是相关项查询无命中，不表示整个MSI表为空 | `T/t02-client-terminal-20261016/admission-red/formal1501-tables.json`及checkpoint；standard包 `6a37d22d6857b75318a175b160e6368beb4e3462ba5cc3ce33a4b878bf9189bc`；SOS包 `17f8ee985719936177c7b8c3cf4078e6879882dfde7bbd3d6f9ca37af15ad519` | 只绑定这两个包，不外推后续发布或旧已安装缓存包；正式1501表disabled不能证明ordinary真实productcompat完成 | C37关联新旧实际包表及签名来源，保留默认off | package/evidence owner；只读 |

### Admission gate最新增量与防重复运行索引

以下为最新接续状态，优先于旧T02-ADMISSION-CONDITIONAL/C37“待实现gate”描述。enabled原CA持锁1603的事实不变；新增gate是在进入安装之前明确拒绝不支持的包，**不是把enabled安装修成支持**。产品runtime/outcome-writer E2E及ordinary完整四RED仍未closed，主任务未完成。

| ID / 范围 | status；RED / GREEN | E2E | sourcebinding / evidence | blocker | nextcommand | owner/resource |
| --- | --- | --- | --- | --- | --- | --- |
| T02-ADMISSION-GATE | **RED1pass/4fail/1ignored→GREEN5pass/0fail/1ignored、exit0、0.42s**；child各次成功不追加业务数 | 真实生产install前置gate、check_pair/inspect_capability读取MSI表；target enabled/cached enabled/target metadata缺失/cached metadata缺失在intent前拒绝；正式1501双disabled控制组到达intent边界，包字节保持 | `T/t02-client-terminal-20261016/admission-gate-red/{vm-stdout.txt,delivery.json}` EXE `78b0fabb…`、runtime `3c048d2b…`；GREEN `admission-gate-green/{vm-result.json,vm-stdout.txt,delivery.json,check.json,checkpoint.zh.md}` EXE `fa2717fb86f58bebfe63061eadc57a51641037e33aea109379e82d60831aa88b`；runtime `a218db18c42c2c845c8d62985c80b20634b613732b3004b96aed7800dde43109`；admission `0d36d65149a604e11eed881c7be1f40f153d5a8e1e4a5d71fbcfd11e9c211ada`；receipt `171c7ccc…`、测试 `118203e8…`、投影 `c0fad490…` | changed=[]/services_match=true；缓存发现使用fixture替身，未证明真实MsiEnumRelatedProducts/LocalPackage正向。supported仅同family MSI且target/cached均明确disabled；unknown/unavailable拒绝，EXE→MSI未获支持 | C38登记已执行；C39持锁重查新RED，之后真实注册/缓存发现及产品E2E | admission/runtime owner；Win10独占 |
| T02-ADMISSION-RECHECK-P2 | **确证RED0pass/2fail、exit101→GREEN7pass/0fail/1ignored、exit0、0.71s；review closed（用户session确认）** | InstallGuard成功取得后、current_exe/identity/intent前重新check_installed；锁后cached变enabled或missing两场拒绝；七父项包含旧gate5，child内部执行不计新增 | `T/t02-client-terminal-20261016/admission-lock-red/{vm-result.json,vm-stdout.txt}`；GREEN `admission-lock-green/{vm-result.json,vm-stdout.txt,delivery.json,check.json,checkpoint.zh.md}`；EXE `a253b8226007beb42160738b0877837f8d9faa57f7f1dfa309d2943be179dc86`；runtime `d2cd04db7af10277b39b46e479dc2007fbadb18c25ad772061e37aacc0674565`；测试 `705b613c2ea9ff4ae399d236a43a7771e7d0bd4d91eef6501e8b37a0b84fc958`，admission仍 `0d36d651…` | closed仅锁后重查切片；未取得独立review artifact，不冒认本轮复审。仅支持明确disabled边界，真实注册缓存discovery/源basename/产品MSI/newprocess ACK未闭；原ordinary四RED本轮未重跑，不closed | C39仅历史已执行入口；下一步N01–N04 | runtime/evidence owner；冻结证据保留 |

已核可读T冻结的续接索引（不是新增执行，不相加）。本节之前旧阶段表的nextcommand仅保留追溯用途，不是当前待执行队列；当前nextowner/动作统一以N01–N08为准，已闭gate/ACL/supervisor/preparation不因旧行写待测而重跑。

| worker / 切片 | 最后已核制品 / 实测 | 源码冻结及复用边界 |
| --- | --- | --- |
| T01 realworker | `d18006f1…`；1case/2scene已绿 | `T/t01-real-worker-freeze-20261016/sha256.json`：reader `568dc713…`、producer `b069c2e1…`；本次目录核对未发现更晚worker result；不重新跑旧gapworker候选 |
| T02 product writer | `045a0718…`；16父项已绿（含14） | `product/delivery.json`：runtime `4b6e8bdd…`、receipt `3bd51fe2…`；仅历史产品候选冻结，不能当最新gate/supervisor整合全绿 |
| T02 supervisor | `000bf561…`；2父项已绿、真实child属内部执行 | `supervisor-green/delivery.json`：runtime `2f9ed821…`、receipt `a12c064f…`、supervisor `bd7d571e…`；testmutex/非真实MSI边界保持，不因原handoff写“待VM”重复运行 |
| T02 preparation | `de3c9708…`；4父项已绿 | `preparation-green/{delivery.json,check.json}`：runtime `3c048d2b…`、receipt `98daaea1…`；P1 limited静态审查已闭，不重列待RED；不得套到新gate整合源码 |
| T02 admission gate旧冻结 | `fa2717fb…`；5父项已绿，后被gate-lock7包含 | `admission-gate-green/check.json`：runtime `a218db18…`、receipt `171c7ccc…`、admission `0d36d651…`；保留历史，锁后重查已由下一行闭合，不再待RED |
| T02 gate-lock最新冻结 | `a253b822…`；7父项已绿/1ignored，review closed（session确认） | `admission-lock-green/{delivery.json,check.json,vm-result.json}`：runtime `d2cd04db…`、receipt `171c7ccc…`、admission `0d36d651…`、测试 `705b613c…`；check前后相同且exit0仅为编译证据，产品E2E仍未闭 |

此处T02相对目录均位于 `T/t02-client-terminal-20261016/`。source变化不抹去历史PASS，也不自动触发把所有旧测试再跑一次；后续owner只对新P2和确实受影响的整合路径安排新冻结，先核进程/现有任务/result避免重复副作用。

### Runtime integration与MSI source fixture最新回执

| ID / 范围 | status；RED / GREEN | E2E | sourcebinding / evidence | blocker | nextcommand | owner/resource |
| --- | --- | --- | --- | --- | --- | --- |
| T02-RUNTIME-INTEGRATION | **GREEN-local：5父项pass/0fail/3ignored、exit0、9.87s**；install/supervisor/reconcile child实际由父启动，内部次数不累计 | 保留生产install准备、Outbox、supervisor握手/lease/selfcheck、reconcile_all与report HTTP/ACK判断；真实独立进程及loopback HTTP丢连接/重发。0/3010/1603为固定installer stub；系统身份/实例/门禁、admission、安装路径/ACL前置、健康probe和HTTP签名身份/目的地为明确fixture边界 | `T/t02-client-terminal-20261016/integration/{vm-result.json,vm-stdout.txt,delivery.json,checkpoint.zh.md}`；EXE `626ae2c45d3f9dee0704671afe13c1bb0346ccb14f35491c4d198559586d6b7a`；源receipt `171c7ccc…`、supervisor `7c2b32be5d3500b4369303b971a5472a4118bad9351f59e50d47230ada50a7b9`、runtime `d2cd04db…`；投影runtime `d3f4ffe4…`、supervisor `f2a8761e…`、harness `6fa6429e…`，均见delivery | changed=[]/services_match=true；真实MSI/SCM/正式DLL/签名发布及产品E2E未closed。旧四RED可按下表关联隔离语义覆盖，**不删历史RED、不称原测试原样重跑转绿**；并发sender/dispatch及无command服务端重复行P2仍pending | C40记录已执行；N03/N04只补真实产品边界，不重复隔离suite | runtime/integration owner；Win10/loopback |
| T01-MSI-SOURCE-SETUP | **BLOCKED_SETUP；Win11两安装场景均未执行，不是business RED** | pre-install Snapshot的Test-Path读取LocalService RustDesk.toml发生UnauthorizedAccessException；driver exit1、stdout空，未启动安装；原名MSI与package.msi比较未获得运行结果 | `T/t01-win11-receipts-20261017/{owner-result.json,driver.stderr,driver.stdout,cleanup-verification-v2.json}`；`T/t01-source-name-fixture-20261016/sha256.json`：脚本 `a25a366f…`，两MSI相同 `85b2aa30a22d3a14cbfdb25981dd22974ff90e323984834d29f03bc33ca71d46` | 独立清理：fixture ProductCode `{127981AF-9DA4-4B73-A731-9E07CCCB0901}`，context2的MsiGetProductInfoExW返回1605；marker=false、active_msi空/inprogress=false、installation_attempted=false，install/uninstall_exit均null。owner前后state=-1；**不是卸载exit1605，data_preservation仍UNVERIFIED** | C41先修采集权限前置/核残留再移交owner；不得重用空run作执行证明 | MSI source/Win11 driver owner；正常提权后独占 |

原ordinary四项历史RED与新隔离语义的对应关系（同一integration freeze内，不跨行相加）：

| 原缺口 | 已执行integration父case | 已证实范围 / 仍缺 |
| --- | --- | --- |
| installer启动前durable intent | `integration::intent_precedes_independent_installer` | 安装父退出时intent/dispatch在、outcome未在，释放stub后outcome=0；真实MSI及生产部署待验 |
| 新进程恢复原attempt | `integration::new_process_requires_health_before_installed` | raw3010保留，health失败零terminal/零请求，ready后新reconcile进程经HTTP形成installed及两通道ACK；健康是fixture，真实IPC/实例保持待验 |
| terminal持久化/丢ACK | `integration::dropped_ack_restarts_same_terminal_then_converges` | loopback读完请求后断连接，terminal已在/ACK缺失；新进程同terminal重发，201oktrue后再reconcile无请求/旧bytes保持；生产服务端闭环待验 |
| 失败历史不被成功retry覆盖 | `integration::failed_history_survives_successful_new_attempt` | stub1603只报failed/msi_exit_1603，新tx/独立command成功后旧terminal bytes保持；真实安装失败恢复未验 |

第五父case `integration::absent_installation_instance_is_explicitly_unsupported` 明确缺实例时报 `ordinary installation id absent`、ROOT/intent不创建，**不证明旧0510首跳可用**。旧projection的历史RED保持原绑定，新隔离GREEN仅推进语义覆盖，产品E2E及当前主任务仍未完成。

### Win11官方基线、命名实测与real-MSI driver门禁

| ID / 范围 | status；实际结果 | sourcebinding / 原回执 | 边界 / blocker | nextowner / 下一步 |
| --- | --- | --- | --- | --- |
| T02-PACKAGE-BASENAME-RISK | **potential mapping risk; durable-chain result not observed** | Win11 per-user context2 two install/uninstall fixture runs returned exit0 and registered PackageName matched each input basename (`t01-source-original.msi` and `package.msi`); separate Win11 context4 probe on official 1501 retained signed-primary filename `rustdesk-1.5.3-2026101501-sos-windows-x86_64.msi` | context2 `T/t01-win11-complete-20261017/run-dbca48290ed643958b30329b2f0c70e4/result.json` (USERUNMANAGED(2)); context4 `T/t02-win11-baseline-20261017/detail.json`; official MSI SHA `17f8ee985719936177c7b8c3cf4078e6879882dfde7bbd3d6f9ca37af15ad519`; durable candidate fixed copy name `package.msi` in `T/t02-client-terminal-20261016/real-msi/review-fixed.zh.md` | Context2 proves fixture basename behavior only; context4 proves official baseline registration only. It is not established that a durable-chain `package.msi` installation changes context4's registered source to that basename or that producer then fails. 0f46 has no MSI log/outcome, so it does not establish this risk at runtime. Preserve pending tx a020; no install/retry for probing | MSI source/installer owner first specify an isolated mapping RED/GREEN probe; any product attempt requires pending review and separate approval; retain primary signature/basename validation |
| T02-PACKAGE-NAME-CANDIDATE | **host no-run 编译成功；VM未运行，NOT GREEN**；7 case候选、0 ignored | production helper将已验签target.primary经`package_gate::cache_name`绑定，并贯穿receipt/runtime copy/runner/supervisor/MSI `/i`；旧receipt缺字段时回退`package.msi`，字段存在但绑定或值错误则拒绝 | `T/t02-client-terminal-20261016/package-name/checkpoint.zh.md`；候选EXE `ordinary_update_package_name_contract-d92d94e2e0b7ad1f.exe` SHA `9B8CD2D8F952C88B3F9CFD935FE23960B5B9C4CB00B4B4391F37D5405C255065`；package_name.rs `2D6584A67609388694A65F1DCFDAB589FBA1AD355C492F7C5999441888DE3242`；tests `7CF7A5E1A40E062ED549550A07430D131E6BD6EAC39C0D753444A8A4DAC4CAF4`；supervisor `636C94F13810A3552A77237D0181DBAFFF099A484968CC44BC44B606499445BF`；runtime `B54CCB53216C2813F48E120F9DA0C24024FF401922E441BB9E52861796F815AF` | 仅production helper逻辑测试候选，不查询Windows注册PackageName、不证明 durable MSI basename映射；无VM runtime结果。旧receipt fallback契约不等同当前正式包安装兼容；a020 pending不动 | package-name owner取得VM执行窗口并保存每case结果；T02-PACKAGE-BASENAME-RISK仍独立待映射验证 |
| T02-WIN11-OFFICIAL-BASELINE | **正常baseline升级PASS，inspect-only PASS；不是新runtime自动升级E2E**。官方SOS0510→1501一次msiexec raw exit0；inspect_exit0；health-v2 build1501/version1.5.3、local_responsive/registered/no_active_sessions均true，双服务Running/Auto/LocalSystem，8份LocalService/rdlab配置SHA保持 | `T/t02-win11-baseline-20261017/{checkpoint.zh.md,upgrade.json,after.json,detail.json,health-raw.json,inspect.stdout,final.json}`；原名官方MSI `17f8ee985719936177c7b8c3cf4078e6879882dfde7bbd3d6f9ca37af15ad519`；实际ProgramData agent `f97cbb6d…`、版本1.5.3+2026101501；保护快照 `a7cf2f12-3698-4193-bd71-df1287a95d67`，完成快照 `f4ae5d09-a69f-4be1-a153-3a979929ba11` | 新installation_id `1f2510c7bdd334b6e33e4a3e90c7c6ce`，before旧字段null/缺失，**不是实例保持证明**。CLI status只有root文本，健康依据独立v2；final中id/uuid正则空值不是身份丢失，校正来源为identity-final.json/实际字段。未执行driver install/reconcile/API ACK；final无相关进程/InProgress、固定tx不存在，不因此重装 | Win11 baseline owner；保留健康现场及源包，先核进程/固定tx/回执；禁止因中断重复官方升级 |
| T01-SOURCE-NAME-CONTEXT2/4 | **两个观察分开保留**：context2 per-user fixture原名和package.msi两场install/uninstall exit0，PackageName分别等于各自输入basename；context4官方1501查询保留signed-primary原名、LastUsedType=n | context2 `T/t01-win11-complete-20261017/run-dbca48290ed643958b30329b2f0c70e4/result.json`（USERUNMANAGED(2)，两输入字节同SHA `85b2aa30…`）；context4 `T/t02-win11-baseline-20261017/detail.json`；官方MSI `17f8ee98…` | 仅证明两个各自场景中的source记录。**无法据此推断 durable chain 的 package.msi 会被context4改名、也无法推断 producer 必然拒绝**；0f46无MSI日志/outcome，不能充实该因果。context2其它setup失败/cleanup不确定仍分开保留 | durable新链映射为待验证问题；须先设计隔离映射RED/GREEN，a020 pending审查/新审批前不做产品安装 |
| T01-REGISTERED-SOURCE-PROBE | **GREEN-local：Win11真实context4 registered-source probe 1pass/0fail/0ignore、raw exit0、0.95s**；独立只读场，不累计到命名fixture/worker | `T/t01-win11-registered-probe-20261017/{result.json,test.stdout,before.json}`；精确case `t01_installed_1501_real_registered_source_matches_signed_release`；EXE `8954b3ad0f6d95efa35ad5e05f3e79eed7274e1b72b310d1506031dbd7ba2d11`；输入正式SOS MSI `17f8ee98…`、resolved `2ca7049c…`；产品 `{A1F34A5F-4F09-4174-9B82-EC0EE48B65FD}`，生产InstalledSource返回官方原basename及实际源文件 | 原8个受检文件SHA保持（不是前次baseline的8份配置集合）；双服务Running、PID6128/5152及路径保持，tx前后不存在，hashes_preserved/services_preserved=true。证明注册源正向这一层，**record/resource initialization E2E仍缺**，不证明新durable package.msi兼容或完整资源ready | first-install/source owner；已通过probe不重跑，N05补初始化完整场景；N02只补新durable源名 |
| T02-REAL-MSI-DRIVER-0F46 | **inspect exit0；install driver exit1、supervisor handoff前error32；MSI是否启动未知** | `T/t02-win11-0f46-20261017/{checkpoint.zh.md,inspect.json,install.json,install.stderr,pending-observation.json}`；driver `0f46de15…`；lab tx `T02LabOrdinaryUpdates\a0200015000000000000000000000001` | 无supervisor acquired、outcome、terminal、MSI verbose log；不是MSI rollback/no-op。8文件archive hash verified，产品hash/进程/服务检查保持；事务禁止删除或重跑 |
| T02-REAL-MSI-DRIVER-REVIEW-5B570 | **driver_review/runtime辅助回归6pass/0fail/0ignore、exit0；不是driver install GREEN** | `T/t02-client-terminal-20261016/real-msi/{review-fixed.zh.md,vm-driver-review-result.json,vm-driver-review-stdout.txt,review-freeze-check.json}`；回归EXE `1208a454…`；candidate driver `5b57042c…` | 六项只验证policy坏类型、缺省/空/N/Y、重复观察与evidence目录失败；`review-fixed.zh.md` 明确install暂停，另有两场景静态类型/重复证据修正需独立六项VM回归，不可标runtime GREEN。source basename仍单列T02-PACKAGE-BASENAME-RISK；不能据helper回归关闭0f46 pending |

签名口径：`upgrade.json`真实Authenticode状态为 **NotSigned**（无Authenticode代码签名），不改记Valid；inspect使用生产package_gate核验**正式发布manifest签名以及包size/SHA**通过，是另一条校验链。两者不可互相替代，也不因官方来源/manifest签名通过宣称MSI已有Authenticode。baseline也不作为新durable自动安装、自动恢复或新进程HTTP ACK产品E2E证据。

## 历史已成熟切片（限定复用，不倒退为“全未测”）

以下原始证据路径主要由已查阅 `T/acceptance-ledger-20261014.zh.md` 与 [VM记录](agent-vm-system-driver.zh.md)提供；未逐包重算摘要，sourcebinding 为历史封存绑定，当前工作区相等性 unknown。未找到独立运行回执的项目明确保持 unknown。

| ID / 范围 | status；RED / GREEN | E2E | sourcebinding / evidence | blocker | nextcommand | owner/resource |
| --- | --- | --- | --- | --- | --- | --- |
| H-EVENT-1 恢复事件底座 | REUSE；文件逻辑22红→绿，标准target41通过；4项是41子集 | 尚非全量部署上传 | `T/localevents-standard41/{manifest.json,test.result.json,summary.json}`；VM v228 | 256容量无30天自动清理；不得跨freeze叠加136 | C11 核标准target清单/引用源码后重跑受影响case | event owner；Win10 |
| H-EVENT-2 R1对账 | REUSE；RED2pass/2fail→GREEN4；Corrupt/Storage及旧字节保留 | SCM生产对账/节流日志未全证 | `T/r1-reconcile-p2/{window-result.json,green-manifest.json}`；EXE `0c5bc55d…` | 专包其他文件漂移不使本切片自动失效，也不扩大结论 | C11 按manifest选择精确4项 | event owner；Windows |
| H-EVENT-3 R4 finalize/poll | REUSE；finalize18＋旧回归1；poll9，标准target9重跑不再累加 | 只事件对账不调用backend；完整resident API未闭 | `T/localevents-r4-finalize/`、`T/r4-poll-p2/`、`T/r4-standard-vm-20261014/summary.json` | 前8坏候选阻后续的保守限制；不同freeze | C11 核标准agentbin及9项命名 | event owner；Win11 SYSTEM |
| H-EVENT-4 全失补报 | REUSE；17业务＋普通R4九项为独立回归；空child不计 | worker→helper→MSI 86.58s；锁争用76.94s；上传副本1pass/1.76s **分段** | `T/full-loss-degraded/`、`T/unreported-isolation/`、`T/r4-event-runtime-final-20261014/`、`T/r4-event-lock-sealed-20261014/upload-only-acceptance-manifest.json` | 初次HTTP500为fixture DB命名；即时GREEN-after证据缺口不以后续健康补造 | C11 后续补同次常驻上传ACK链 | event/API owner；Win11＋隔离PHP |
| H-R0 | REUSE；14项门禁/预算/ACK冷却通过，有效mediator实验红→绿 | 真实UDP两代RegisterPk/OK；未部署300秒installed worker/listener整链 | VM v224/v225 | 同owner连续观察、活动门禁及真实常驻链欠E2E | C11 核R0封存，补300秒常驻 | recovery owner；Windows/本地UDP fixture |
| H-R1-R2 | REUSE 局部；服务动作false误成功、disable门禁有历史红绿；R1约16秒恢复通过 | 实际停止/启动及固定服务修复历史记录；完整当前故障组合UNKNOWN | VM记录v5–25、最终报告10.1；两OS operations各8及固定门禁另列 | 不把手工启用旧门禁当默认部署；Running挂起/访问拒绝/MarkedForDeletion/活动场景未全闭 | C11 核对应OS/agent摘要，补受影响固定服务场景 | recovery owner；Windows SYSTEM |
| H-R3 | REUSE；有效红后跨轮持久8项；整目录/二次启动E2E另1 | 真主EXE/整目录缺失→完整fallback ready/可重启 | VM v206–209；健康快照 `2247f7ae…` | 存在但损坏EXE仍保守交R1；不代表完整R4 | C11 核R3来源/清理回执 | recovery owner；Win11 |
| H-R4-1 schema2 | REUSE；freeze5/来源14分批；实际成功1/273.29s、验证失败rollback1/220.60s | **恢复快照前**fallback ready、agent/bootstrap/身份/config/任务验证，非快照回滚 | `T/r4-diagnose-win10/`、`T/r4-validationfail-win10/{freeze.json,rollback.evidence.zip}`；制品分别 `316f2280…`/`1612d7a6…` | 非同freeze两场；未全部署/生产HTTP ACK | C11 保留原包/source，补当前整合差量 | R4 owner；Win10 |
| H-R4-2 自动离线 | REUSE；v210契约失败是有效RED；后续独立backend绿 | v213成功Committed/494.36s；v215失败RolledBack/1527.23s，至少5轮同ID不重装 | VM v210–215、`T/autoschedule-v213-review.zh.md`、`T/autofailure-v215-review.zh.md` | 正式keyCLI/实际活动/最终中断窗口未全覆盖 | C11 核计划、来源ID及终态 | R4 owner；Win11 |
| H-R4-3 全失worker | REUSE；有效主RED `service installed contract mismatch`；新seal成功74.17s/失败74.07s各1pass | 成功Committed；无client锚点失败 **RollingBack/client_recovered=false**，agent保全≠client RolledBack | `T/r4-current-source-sealed-20261014/acceptance-manifest.json` SHA `766c0b88…`；EXE `619bab76…`，1257文件source_delta={} | 测试key/unsigned manifest；正式部署整链未证；旧83.45/104.74s不冒认current | C11 核seal再补生产等价整链 | R4 owner；Win11 SYSTEM |
| H-R4-4 rescue缓存/轮换 | REUSE；v216红2→绿3；轮换11＋after_commit幂等1各按freeze | 实际395084857字节/106条验证；current/previous394文件；未再安装/真实断电轮换 | VM v216/v219/v220 | 24h墙钟未测；仍degraded/ready=false，不改常量补绿 | C11 核freshness/rotation清单 | rescue owner；Win11 |
| H-R5-R6 | UNKNOWN 完整最新执行；规格不算GREEN | 固定批准在线fallback/全失败熔断整个场景缺已核回执 | 升级方案/测试runbook；下载镜像见H-INSTALL-2 | 未找到独立完整阶梯终止/预算E2E回执 | C11 先定位原始run，缺失则另建RED/场景 | recovery owner；隔离网络fixture |
| H-AGENTUPDATE | REUSE；MSI能力红绿；成功214.55s、失败恢复226.89s各1pass | 完整0502→0503 MSI；旧client/agent/bootstrap/配置/身份恢复；SQL终态各一次 | `T/agent-update-prep/{acceptance-final.json,checkpoint.zh.md}`；同EXE `82594b81…` | 测试poll补报≠实装resident自主poll；旧包9能力/新snapshot12分开 | C11 补升级后新resident自然claim/ACK | agent/API owner；Win11/SQLite |
| H-INSTALL-1 MSI事务 | REUSE；安装意图/结果/接续等低层红绿；真实跨build成功与rollback均有 | v154两OS成功240.33/223.68s；v155/165回滚及resume零backend、原错误字节保持 | VM v154–166、对应 `T/agent-local-crossbuild-v154*`、`agent-terminal-native-v165*` | 不覆盖普通updater T02；MSI admission默认off仍未闭 | C11 核每OS制品/SQL/源包 | installer owner；Win10/11串行 |
| H-INSTALL-2 primary/mirror | REUSE；预算/失败回退红绿 | v206目标与回滚primary503、同域mirror真实完整下载、实际升级/回滚 | VM v200–206及旧ledger；本次未独立展开v206原始归档，当前source相等性UNKNOWN | 同域mirror不证明DNS/CDN整体容灾；rescue hook未随该helper验收 | C11 核v206下载/SQL回执 | installer owner；Windows/loopback双包 |
| H-INSTALL-3 EXE旧包矩阵 | REUSE；standard成功/rollback，SOS首装/成功/rollback历史passed | 真实native，lab key/loopback/旧0502资源；非正式在线首跳 | `T/exe-v226-{report.zh.md,verified.json}`、`T/sos-v228-{report.zh.md,verified.json}` | 不能关闭T03当前Pending或四条正式旧GUI在线迁移 | C07 逐包source绑定 | EXE owner；Windows |
| H-INSTALL-4 admission | BLOCKED；v232五/v233四行为＋matcher3历史分组保留 | 原生MSI回滚不等于agent事务rollback | `T/acceptance-ledger-20261014.zh.md` admission行 | default-off；legacy/反向全程互斥/缺LocalPackage等未闭；独立工具拒绝回执未找到，细节unknown | C11 先读原拒绝与result，不重发被拒动作 | MSI owner；Win11 |
| H-SILENT | UNKNOWN；短 `legacy-p1-hidden-green.json`仅passed布尔，不据此定GREEN | 无窗口安装、父子隐藏、UAC/SYSTEM完整产品场景本次未找到充分回执 | `T/legacy-p1-hidden-green.json` source `339685bd…`；`tests/windows_exe_agent_hidden_fixture.ps1` | 单布尔不能证明测试数、真实窗口观察或当前源码；不能由静默参数/exit0推断成功 | C20 先查 `legacy-hidden-result.json`及驱动日志，绑定缺失保持UNKNOWN | EXE/桌面 owner；Win10 |
| H-API-1 既有合同/UI | REUSE；SQLite65/状态30/HTTP12、v173浏览器三流程各属独立层级 | 1440/390确认/取消/能力门禁有历史E2E | 最终测试报告、VM记录；`T/client-contract-7181/binding-green-guest.json`标准7项 | 同实例坏存储接纳/新实例409已局部证；完整高权限批量/容器/broker负载未闭 | C11 核API版本和旧schema；补UI/负载 | API owner；隔离SQLite/浏览器 |
| H-FLUTTER | REUSE；19suite/172唯一case通过；不与地址簿30相加 | widget非真实导航/会话；最小化还原passed，导航UIA仅2节点阻塞 | `T/flutter-full-win10-case-audit.json`、`T/acceptance-ledger-20261014.zh.md`；VM nav2条 | salt存储差异已知但具体写入者/认证影响unknown；analyze247 issues/exit1 | C12/C18；保留已通过最小化场景 | Flutter owner；Windows桌面 |

## 可执行接续命令索引

这些命令是后续 owner 的入口，本轮均未执行。带安装/卸载/锁定动作的命令须在对应隔离基线、独占资源和回执前置满足后执行；不能把本文当作新开 VM/故障注入授权。没有确定驱动时，nextcommand 先读取证据或规格，避免编造可运行 E2E 命令。

| 命令ID | 入口（PowerShell；`Get-Content`为只读取证） |
| --- | --- |
| C01 | `$env:RUSTDESK_T01_PRODUCER='red-formal-sos-1501'`；`& '<新SHA producer EXE>' --ignored --exact t01_producer_formal_exact_build_publishes_schema1 --test-threads=1 --nocapture`；另外两case按 producer交接逐项exact运行，各1项 |
| C02 | `$env:RUSTDESK_T02_VM='ordinary-terminal'`；`& '<新SHA terminal EXE>' --nocapture --test-threads=1 fixture::t02_`；父项必须实际4个，不加全局ignored |
| C03 | `& '<绑定rescue-projection EXE>' t01z_ --nocapture --test-threads=1`；预期13执行/1忽略，Z-R06另待契约 |
| C04 | `$env:RUSTDESK_T01_DELIVERY='ordinary-msi-1501'`；`& '<绑定delivery EXE>' --ignored --exact t01_formal_approval_tamper_refused_preserves_original_inputs --test-threads=1 --nocapture` |
| C05 | 同C04环境；`& '<绑定delivery EXE>' --ignored --exact t01_ordinary_msi_missing_approval_preserves_installation_observation --test-threads=1 --nocapture`；先核交接安装条件 |
| C06 | `Get-Content -LiteralPath "$env:TEMP\opencode\t02-api-host-20261016\T02-result.zh.md" -Encoding UTF8`；依回执先重建runtime再执行 `run_baseline.py` / `t02_gap_test.py`，每项独立DB |
| C07 | `Get-Content -LiteralPath "$env:TEMP\opencode\legacy-realrollback-current-4d4cc3\checkpoint.zh.md" -Encoding UTF8`；与sync-1501/source-original/diagnostic逐项核对，完成源码/进程/任务核验后才安排新case |
| C08 | `& '<Z1 EXE>' 'management_agent::status::tests::health_observation::' --nocapture --test-threads=1`；另 `& '<Z1 EXE>' 'management_agent::status::tests::application_projection_requires_live_identity_proof_and_keeps_registration_separate' --exact --nocapture --test-threads=1`；勿用Z3扩展EXE冒认原6项过滤 |
| C09 | `& '<Z2 EXE>' 'management_agent::status::tests::health_observation::blocking_pipe_' --nocapture --test-threads=1` |
| C10 | 按 `T/t04z-dual-20261016/交回.zh.md`设置 `T04Z_DUAL_ROOT`，exact执行 `management_agent::status::tests::health_observation::dual_token_fixture_server`；两token分别设ROLE=admin/system并exact执行 `dual_token_fixture_client`，均带 `--ignored --test-threads=1 --nocapture` |
| C11 | `Get-Content -LiteralPath 'docs/agent-vm-system-driver.zh.md' -Encoding UTF8`；按行对应T目录读取manifest/result及精确测试名；缺原始日志或不匹配时先记UNKNOWN，不启动历史安装脚本 |
| C12 | VM Flutter目录：`flutter test test/ab_online_reload_test.dart test/peer_online_reload_test.dart --reporter expanded`；完整四suite名按peer交接核对；真实session流程依新计划T07另建驱动，不以此代替 |
| C13 | `Get-Content -LiteralPath 'tests/windows_desktop_shortcuts_handoff.zh.md' -Encoding UTF8`；先核fullca的present/missing最终回执与旧rollback日志；新失败场景单独冻结完整包 |
| C14 | `Get-Content -LiteralPath 'tests/exe_desktop_shortcut_handoff.zh.md' -Encoding UTF8`；核adapter/cases/EXE后用原9项fixture重跑；完整lib构建及产品安装各另留退出回执 |
| C15 | `Get-Content -LiteralPath 'docs/agent-recovery-tdd-plan-20261016.zh.md' -Encoding UTF8`；按T09核观察器实际入口与before/early/after输出；未核路径不编造安装命令 |
| C16 | `Get-Content -LiteralPath "$env:TEMP\opencode\lock-logoff-driver\RUNBOOK.zh.md" -Encoding UTF8`；按Prepare→Act→Collect及已冻结manifest执行；各OS/动作单独run |
| C17 | `Get-Content -LiteralPath 'tests/management_agent_purge_policy.zh.md' -Encoding UTF8`；以末节marker/native摘要与真实CLI为准，先重冻定向测试；禁止旧EXE直接执行purge |
| C18 | 逐条：`cargo fmt --all -- --check`；`cargo test --locked --workspace --no-fail-fast`；`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`；Flutter目录 `flutter analyze`、`flutter test`。客户端运行移交VM；编译目标与执行case分报，另产真实业务分支报告 |
| C19 | `python -m unittest discover -s tests -p test_release_platform_selection.py -v`；macOS签名相关另 `python -m unittest discover -s tests -p test_macos_signing.py -v`；真实EXE/MSI/AppImage构包/启动须平台owner提供已核入口 |
| C20 | `Get-Content -LiteralPath "$env:TEMP\opencode\legacy-hidden-result.json" -Encoding UTF8`；补核窗口观察、实际测试名、计数、raw exit、制品/source，不以passed布尔关闭silent |
| C21 | 本次已执行命令登记：VM环境 `RUSTDESK_T01_PRODUCER=red-formal-sos-1501`，`& '<SHA=9c825a65…的封存producer EXE在guest核验路径>' --ignored t01_producer_ --test-threads=1 --nocapture`，实际9项通过。本轮不重跑；后续由接线owner先核 `T/t01-producer-acl-green-freeze-20261016/sha256.json`，补产品调用点及真实注册源正向E2E的独立计划 |
| C22 | 已执行入口：VM环境 `RUSTDESK_T02_VM=ordinary-terminal`；`& '<SHA=ccbae932…的ordinary_update_writer-e34940343a23991b.exe在guest的核验路径>' --nocapture --test-threads=1 persistence::`；实际14pass/2ignored child，禁止全局ignored；旧9/10不累加，本轮不重跑 |
| C23 | 同C22环境；`& '<SHA=0f628d33…的wiring EXE>' --nocapture --test-threads=1 wiring::`；历史实际2pass/4fail/exit101，后续GREEN按C29/C30不同freeze分列，不再记待RED |
| C24 | 同C22环境；`& '<SHA=6697f525…的current-red EXE>' --nocapture --test-threads=1 fixture::t02_`；应5父项/1ignored child，新failed/rollback case与原4分别报告 |
| C25 | `Get-Content -LiteralPath "$env:TEMP\opencode\api-ui-time-fix-20261016\source-sha256.json" -Encoding UTF8`；按四文件SHA补核最新review/source-final原件及TDD manifest；原件未定位前不编造读取命令或摘要，不因补归档重复计测试 |
| C26 | `Get-Content -LiteralPath "$env:TEMP\opencode\tdd-acl-v2-actual.stdout" -Encoding UTF8`；另读同名stderr及 `T/t01-producer-acl-green-freeze-20261016/{vm-result.json,vm.stdout,sha256.json}`。精确case为 `t01_producer_existing_users_readonly_acl_refused_preserves_bytes`；RED exit=null保留，GREEN已包含于9项，不新增通过计数 |
| C27 | 已执行coord过滤：`& '<09140d21… EXE>' --ignored t01_coordination_ --test-threads=1 --nocapture`；producer回归另 `--ignored t01_producer_ --test-threads=1 --nocapture`；环境 `RUSTDESK_T01_PRODUCER=red-formal-sos-1501`。两份原result分开，不要求重复运行 |
| C28 | 已执行HTTP过滤：`& '<90c51341… EXE>' --ignored t01_http_ --test-threads=1 --nocapture`；同C27环境。精确父case为 `t01_http_fast_silent_headers_and_oversize_preserve_data`、`t01_http_slow_chunks_exhaust_total_budget_preserve_old_data` |
| C29 | ACK精确入口：`& '<fb21d0e2… EXE>' wiring::t02_http_ --nocapture --test-threads=1`，同C22环境；已证据为原八项运行中的七项ACK通过，不虚构独立七项进程exit0；八项整轮exit101/DLL fixture1失败保留 |
| C30 | 已执行runner入口：`& '<cc8d80d6… EXE>' wiring::t02_real_runner_ --nocapture --test-threads=1`，同C22环境；实际3010/1603两项通过。其review closed为会话交接，无独立artifact；不新增durable终态证明 |
| C31 prodreconcile | 旧“先实现接线/运行旧projection”建议撤销，接线候选已存在；原证据仍见writer/product各checkpoint及T02-2。当前只安排N03/N04完整runtime/独立writer/newprocess HTTP ACK验证；旧 `6697f525…` 投影不证明新实现，原四durable RED未重跑、未closed |
| C32 gapworker | 旧提取候选不再作为下一运行命令；`T/t01-worker-wiring-red-20261016/{extraction.json,sha256.json}`及 `0c6abf1b…`仅保留历史。真实worker分层1case/2scene已由C33证明，下一步N05首次资源完整初始化 |

### 最新精确接续入口

| 命令ID | 入口与边界 |
| --- | --- |
| C33 | 已执行：环境 `RUSTDESK_T01_PRODUCER=red-formal-sos-1501`，`& '<d18006f1…真实worker EXE的guest核验路径>' --ignored --exact t01_real_worker_http_producer_reader_preserves_health_failure_and_waiting --test-threads=1 --nocapture`。一case两scene已绿，不再用C32旧提取候选证明本实现；下一真实注册/健康正向需单独场景 |
| C34 | 已执行：环境 `RUSTDESK_T02_VM=ordinary-terminal`，`& '<045a0718…product writer EXE的guest核验路径>' persistence:: --nocapture --test-threads=1`；16父项含旧14，不追加为30；两ignored child仅由父case启动 |
| C35 | 已执行GREEN：同C34环境，`& '<000bf561…supervisor EXE的guest核验路径>' persistence::supervisor_ --nocapture --test-threads=1`；2父pass/1ignored child入口，child实际被调用一次。原365cf8c9… RED保留，不与GREEN叠加；本轮不重复执行 |
| C36 | 已执行GREEN：同C34环境，`& '<de3c9708…preparation EXE的guest核验路径>' preparation:: --nocapture --test-threads=1`；4父pass/1ignored child。下一步只读核 `T/t02-client-terminal-20261016/preparation-green/{delivery.json,check.json,vm-result.json}`，将当前失败收敛与独立监督整合后另验证产品runtime，非重跑旧case补全产品 |
| C37 | CA已执行入口（环境同C34）：`& '<53ba1595…admission-fixture.exe的guest核验路径>' '<已核disabled.msi绝对路径>'`及enabled.msi独立调用。前者0、后者harness101/CA1603分别保留；下一步读 `T/t02-client-terminal-20261016/admission-red/{checkpoint.zh.md,formal1501-tables.json,vm-result.json}`，由owner补目标/原缓存包准入策略与真实升级兼容测试；不重发安装或改变默认off |

| 命令ID | 最新gate接续入口 |
| --- | --- |
| C38 | 已执行gate GREEN入口：环境 `RUSTDESK_T02_VM=ordinary-terminal`、`T02_ADMISSION_INPUTS='<已核inputs的guest绝对路径>'`；`& '<fa2717fb…gate EXE的guest核验路径>' preparation::gate_ --nocapture --test-threads=1`；实际5父pass/1ignored child；按admission-gate-green/inputs.json与delivery核输入，不重复运行来补P2 |
| C39 | 已执行GREEN历史命令：环境 `RUSTDESK_T02_VM=ordinary-terminal`、`T02_ADMISSION_INPUTS='<已核guest inputs目录>'`；`& '<a253b822…EXE的guest核验路径>' preparation::gate_ --nocapture --test-threads=1`；实际7父pass/1ignored。新增两项RED使用 `preparation::gate_lock_`，已闭合，不再排队重跑；原gate5被包含，不计5+7 |

C31/C32已撤去过期运行建议；C37/C38/C39均仅为已执行历史入口，gate5及锁后重查均已有GREEN。当前待办只按下列N队列调度，不把历史命令表当作自动重跑列表。七项新freeze通过不覆盖全部product/runtime测试，原ordinary四RED本轮未重跑，仍未closed。

### 当前下一步（每项独立owner，均未完整执行）

| ID | nextowner / resource | 下一步与验收缺口 | 证据与状态 |
| --- | --- | --- | --- |
| N01 注册缓存discovery | admission/discovery owner；已安装MSI的独占Windows VM | T01真实InstalledSource正向已通过，不重复；ordinary的MsiEnumRelatedProducts/per-machine LocalPackage唯一性、InstallLocation、缓存身份/拒绝路径仍独立核验 | registered-source probe不等于完整ordinary缓存发现；gate7缓存替身边界仍保留 |
| N01a package-name helper | package-name测试owner；Win10隔离VM，不运行installer | 对7个真实production-helper case逐项运行：验签primary basename绑定、receipt/runtime copy/runner/supervisor/MSI参数一致、缺字段旧receipt的package.msi回退及非法名拒绝 | `T/t02-client-terminal-20261016/package-name/checkpoint.zh.md`中EXE `9b8cd2d8…`及module/test SHA；目前host `--no-run`成功但无VM runtime回执，不能登记GREEN；不测试MSI注册source |
| N02 source-name mapping | installer/source owner；Win11 baseline与a020 pending保护 | context2两输入名观察、context4官方原名查询均已完成且互相独立，不重跑。设计隔离测试说明 durable txn副本名`package.msi`与注册source名称的映射 | durable路径具体映射未知；不能先验为有风险成因或producer拒绝；a020 pending保护下不做试装，需先形成隔离RED/GREEN设计并获独立审批 |
| N03 prodinstaller/outcome writer | real-MSI driver/review owner修两项pending，Win11；不得重复安装 | 5b570 driver_review六项是辅助策略/evidence回归而非MSI E2E；0f46是handshake前旧attempt并留有a020 pending。新安装意图先过source basename映射、pending处置及主协调动态审批 | `T/t02-client-terminal-20261016/real-msi/vm-driver-review-result.json`、`review-fixed.zh.md`；`T/t02-win11-0f46-20261017/pending-observation.json` |
| N04 newprocess HTTP ACK | updater/reconcile/API owner；独占VM＋隔离HTTP/SQLite | integration新进程/真实loopback ACK隔离语义已绿；下一步只补真实身份/实例/映像/IPC健康与实际安装后的新server，以及API持久化/两通道收敛 | 原ordinary四RED保留，已关联integration五父项中的四项语义；原projection未原样重跑，完整prod链未闭，不重复隔离suite冒充产品验收 |
| N05 T01首次资源完整支持 | first-install/rescue owner；Win10/Win11安装基线 | Win11 registered-source probe1pass及隔离worker两scene已通过；仍须批准record/producer产品接线、完整资源写入/验证、ready与断网恢复，分别证明supported/enabled/ready | probe不关闭record/resource initialization E2E；realworker两scene均ready=false。旧0510 before无installation_id，新1501生成instance，不宣称旧0510支持此runtime |
| N06 peer E2E | Flutter/session owner；控制端＋目标VM统一窗口、隔离账号 | 自然连接触发hash-sync PUT/reload；两online一offline、新帧像素挑战、焦点/关闭后状态及SQL关联 | 模型/widget30已绿，真实会话E2E仍未做；不重复累计单元回归 |
| N07 Defender | Defender/install owner；独占Windows VM | 默认不选与opt-in分场，before/early/after实际偏好/事件/策略读回及拒绝保全 | mock/历史只读观察保留；真实完整opt-in与历史true→false根因未闭 |
| N08 锁定/注销 | desktop/remote owner；两机协调、正常可恢复登录路径 | lock/logoff独立run，OS动作事实、注册/握手/新帧/输入分别取证；macOS适用场景另列 | 驱动AST不能替代执行，完整锁定/注销E2E未做；不因采样/请求成功标通过 |
| N09 Win11 real-msi 结案门 | real-MSI driver owner＋主协调审批人；Win11，原lab tx保护 | 只读诊断0f46 os error32/handshake及archive，审查原pending的可审计处置；不删除/覆盖/重跑原txn。未来若主协调另批尝试，使用当时最新freeze/driver实物SHA并留msiexec log/raw exit/outcome/terminal/product-state | archive 8文件摘要verified；无acquired/outcome/MSI log。当前不能确定supervisor握手前MSI是否曾短暂启动，不称确定“MSI未启动”，也不称rollback/no-op |

| 接续ID | 已执行或下一步入口 |
| --- | --- |
| C40 | 已执行：`RUSTDESK_T02_VM=ordinary-runtime`；`& '<626ae2c4…integration EXE的guest核验路径>' integration:: --nocapture --test-threads=1`。5父pass/3ignored，child由父显式调用，禁止全局ignored；不重复运行，产品补测按N03/N04另冻结 |
| C41 | 旧setup场仅归档：`T/t01-win11-receipts-20261017/cleanup-verification-v2.json`；不再要求重跑两命名场。当前只读 `T/t01-win11-complete-20261017/run-dbca48290ed643958b30329b2f0c70e4/result.json` 与baseline/detail.json核context2/context4；下一真实durable场先修driver四issues，不发布install命令 |

**当前T02 real-MSI边界更新：** `c184`/`5b570` driver review freeze与旧 `36bc`仅作历史；最新0f46仅inspect0后一次install attempt，supervisor handoff前error32，无MSI日志/outcome/terminal/acquired。8个事务文件archive hash已验证且原a020事务保留。它不是MSI rollback/no-op，禁止清除或以旧SHA直接重跑；未来任何尝试必须先审查pending，并由主协调动态审批当前EXE/source freeze。context2 per-user PackageName行为与context4官方原名记录是两个观察，不证明新durable package.msi映射或producer拒绝；该映射作为独立待验证问题，不预设风险成因。

本轮只续记以上原回执与下一owner，不启动任何队列动作。尚未交付精确可执行产品入口的项先补绑定和驱动，不编造命令；历史通过不作新的测试次数。

## 优先队列与完成判据

| 优先级 | 独立任务ID | 下一步 / 退出条件 | ownerresource |
| --- | --- | --- | --- |
| P0-1 | T01-REALWORKER、T01-HTTP-REVIEW | realworker一case两scene已绿，HTTP deadline P2 session review closed；下一步为真实注册源/SCM/健康正向与完整资源准备/安装E2E。旧0510缺合法installation_id必须保留前置阻塞，不宣称支持 | worker/安装owner＋Win10串行 |
| 暂停，转其他任务 | T01-STATE-DIR | 旧stage Win11 RED绑定旧binary保留；当前c191正向candidate与negative第二case均NOTRUN/UNKNOWN，无state result、无业务GREEN。不得重复UAC；整条跨线程回执优先工作转其他任务，旧复测建议不作为当前执行授权。历史signed earlier VMSessions只按真实T原件确认，缺原件保持UNKNOWN | 总账接续；当前不调度VM或子代理 |
| P0-2 | N01–N05/N09 | integration5、official baseline及已执行registered-source observations不重复；新增package-name helper七case仅host no-run，先按N01a在隔离Win10验证，不安装。statefile ACL、5b570 review-helper、gate-lock各为不同freeze；c1910候选未有VM结果；0f46 a020 pending保留。durable basename映射、真实MSI/outcome/newprocess ACK及首次资源初始化仍未闭，**主任务未完成** | package-name测试owner先做N01a；state-dir、driver owner分开source-binding/授权 |
| P0-evidence | T02-UI | 独立review closed（用户确认）及四SHA已登记；C25只补定位review/source-final原件和总manifest，尚无原件路径不冒认已读；104/39/6不重计、不要求重开review | API evidence owner；宿主只读 |
| P0-3 | T03-3 | 查明Stop-Main创建时间/映像具体拒绝，安全处理合法其他进程且拒PID复用/未知映像；最新源码真实成功与健康后失败回滚各封存。旧T03-2不覆盖 | EXE owner＋Win10独占 |
| P1-1 | T04-Z4、T05-1、T06-1、H-EVENT-4 | 当前真实resident/CLI/fallback/后台同窗口，升级后自然poll/补报ACK；保留已成熟R0/R3/R4和分段上传证据，只补部署差距 | recovery/API owner＋Win11或Win10 |
| P1-2 | T07-1、T08-1/2/3/4、T09-1、T10-1/2、H-SILENT | MSI present/missing实包1603/State5/bytes回滚已纳入，保留主服务after结构化记录边界；后续补正式在线包/EXE父子安装、真实会话、Defender、lock/logoff及无窗口观察，不重列两场为未执行 | 桌面/安装owner，双机需统一调度 |
| P2-1 | T11-1/2 | 新source绑定定向native后，实际固定MSI注销证明ProductCode/双ARP/服务/任务/目录；保留旧注册残留与bootgap | purge owner＋Win11 |
| P2-2 | T12-1/2/3、H-R5-R6 | 补最终受影响中断窗口、跨平台实包、严格门禁与≥80%真实业务分支；已拒操作维持BLOCKED，证据不全维持UNKNOWN | 平台/门禁owner；冻结环境 |

### c191 两个 case 最新会话回执（2026-10-17）

本次按用户跨线程交接仅登记总账：主用只读脚本已确认 guest 无 `tdd-state-dir` receipt/stdout/stderr，services 为 Running；屏幕截图记录 UAC 首次可见，但确认后结果未产生。正向 candidate 与 negative 第二 case 分别保持 **NOTRUN/UNKNOWN**，均未取得业务 GREEN；不补造退出码、测试计数或成功执行事实。上述观察来自用户会话交接，本轮未独立读取脚本回执或截图原件，不虚构其 T 路径。历史 `c191 signed earlier VMSessions` 疑似执行的原始 T 证据本轮未定位/核验，明确 **UNKNOWN**，不将疑似历史执行覆盖当前两个 case 状态。

当前调度以本条为准：不重复 UAC，整条跨线程回执优先工作转其他任务；本轮只写 ledger，不操作 VM、代码、CI、commit 或 subagent，也不启动其他队列任务。上文 C42 及分 Win10/Win11 复测建议仅保留为历史待办。

### package-name、真实 source 与 Win10 receipt 实际回执复核（2026-10-17）

本节为本次精读后的优先口径；上文 package-name 七项/旧 schema1 描述仅绑定原候选冻结。只修改本总账，未运行测试、构建、VM、CI、commit 或子代理，未写入其他文件或认知元数据。下面登记已有执行结果，不新增或重复累计测试。

| 独立证据层 | 已核事实 | 不能据此得出的结论 |
| --- | --- | --- |
| 真实 context2 source fixture | 已精读 `T/t01-win11-complete-20261017/run-dbca48290ed643958b30329b2f0c70e4/result.json` 与 `tests/t01_source_name_fixture.zh.md`。Win11 `RDLAB-WIN11\rdlab`、elevated=false、USERUNMANAGED(2)；同字节 MSI 两次分别以 `t01-source-original.msi`、`package.msi` 安装，实际 PackageName 分别随输入名变化，LastUsedType=n，install/uninstall 均 exit0；与原名比较分别 true/false。卸载后 registration=1605、marker=false、fixture_absent=true；1605 是查询结果，不是卸载退出码。受检五文件摘要及双服务状态保持 | fixture 仅 HKCU 标记，无产品 payload/服务/CA；没有调用生产 `installed_source`。它证明 per-user 命名行为，不能证明 ordinary durable per-machine 升级后的注册变化或 producer 必然拒绝；不再把两场观察另计成新增测试 |
| 官方 context4 originalname | 已精读 `T/t02-win11-baseline-20261017/detail.json`：context=4、PackageName=`rustdesk-1.5.3-2026101501-sos-windows-x86_64.msi`、LastUsedType=n、LastUsedSource=`C:\RustDeskAgentLab\t02-real-msi-input\`。T01-REGISTERED-SOURCE-PROBE 的既有正向结果继续保留 | 官方原名基线/只读 probe 与 context2 改名实验不是同场，不能拼接为新 durable 链的 source 映射 GREEN；0f46 无 MSI log/outcome，a020 pending 保护不变 |
| package-name owner 旧七项冻结 | 已精读 `T/t02-client-terminal-20261016/package-name/checkpoint.zh.md`：EXE `9b8cd2d8…`、helper `2d6584a6…`、测试 `7cf7a5e1…`，root Cargo `--no-run` 成功，预期7 case/0 ignored；**实际七测未运行，NOTRUN／NOT GREEN**。交接中的 schema1 `package-name.json` 与缺文件回退属于该旧冻结；“注册source改名已覆盖”只能按上两行的独立边界理解 | 预期数不是 pass counts，编译不是执行，不能据此给当前 schema2、真实签名校验、MSI source 映射或完整产品链记 GREEN |
| 当前生产 schema 名称绑定／主目标代码 | 本次只读源码核对：`src/updater/ordinary_update_receipts.rs` 的 `prepare_at/write_intent` 写 schema2 `IntentRecord.intent_binding.approved_package_name`，`open_at/package_name`读取；`runtime.rs::install`从输入包路径取名并传入 `Outbox::prepare`及副本路径，runner/supervisor消费该名。schema1缺sidecar的回退现在另受 `legacy_msi_installation_evidence` 限制，不能沿用旧交接的无条件回退表述。`tests/ordinary_update_package_name_contract.rs`已增加 helper/receipt 测试，`package_plan`测试注释明确其待runtime接线；这不是旧七项源码 | 仅确认代码存在及调用关系，未重新冻结摘要或证明当前主目标编译/运行。名称进入 intent schema/digest 不等于所有 Binding/terminal digest 已纳入名称；也不等于签名primary→mirror下载→runtime整链已证实。当前源码不能继承 `9b8cd2d8…` 旧 no-run 回执，更不能继承 receipt 并发两项的执行结果 |

N01a 接续修正：先由 package-name 代码 owner 对当前主目标、schema/helper及测试重新绑定源码和制品，再交测试 owner；旧七项候选保留 NOTRUN，不直接按旧 EXE/SHA 验收当前代码。本轮不调度该动作。N02 仍为真实 durable source 映射缺口，N03/N04/首次资源初始化均未因上述静态绑定而关闭。

#### T02-RECEIPT-PUBLICATION：Win10 两轮、两个父用例

已精读 `T/publication-win10-20261017-result.json`、`T/publication-win10-20261017-runtime-result.json`及`T/publication-win10-20261017-handoff.zh.md`。两轮同 EXE SHA-256=`A9391BAD01686E139FD819ACCB306676042CE68128B76CAA69D2199749CEC89B`，身份 `RDLAB-WIN10\rdlab`，JSON均为 elevated=true；环境/精确命令由handoff记录为 `RUSTDESK_T02_VM=ordinary-terminal`、`--exact <case> --test-threads=1 --nocapture`，每例独立 TEMP/TMP。

| 轮次／父case | 实际终态与计数 | 判定 |
| --- | --- | --- |
| 第一轮：两个同名父case的启动尝试 | UTC 2026-10-16T22:17:08.2604355Z–22:17:10.4259574Z；PID7092/664均 exited、raw exit=-1073741515（0xC0000135），stdout/stderr均空、one_test_passed=false | **BLOCKED_SETUP**：未进入断言，无 Rust test summary，不记2个业务FAIL，也不制造RED→GREEN |
| 第二轮 `same_outcome_second_writer_is_idempotent_after_first_exit` | UTC 22:21:00.4915734Z–22:21:01.2537360Z；PID340，exited、exit0；stdout **1 passed / 0 failed / 0 ignored / 2 filtered**，0.09s，stderr空 | **GREEN-local**，仅该父case |
| 第二轮 `conflicting_second_writer_fails_after_first_exit_and_preserves_winner` | UTC 22:21:01.2639117Z–22:21:01.8074970Z；PID280，exited、exit0；stdout **1 passed / 0 failed / 0 ignored / 2 filtered**，0.09s，stderr空 | **GREEN-local**，仅该父case；冲突child失败是预期断言，不是父case失败 |

第二轮仅在测试进程 PATH 前置客体现有 `C:\RustDeskAgentLab\t02-admin-20261016`；runtime DLL SHA=`D1F4225DF2CD877DBF130D5668A021DCE3F94118455FF5EC952061C30AFC9CE7`。未安装运行库或产品。**本切片只有两个唯一父case通过**，两次启动、内部 child、过滤项、保全断言均不额外计数，不与旧 writer/product/integration、package-name 七项相加。

执行语义按该 EXE 的owner交接限定：两个child到达发布barrier，A成功退出后才释放B；同结果幂等，冲突结果含 `ordinary receipt immutable conflict`；winner/原intent字节保持、staging无残留、reconcile为 `failed/msi_exit_1603`。父case成功stdout不含child原始输出，无独立child数值退出码回执，不能补造。当前工作区测试已不同：`tests/ordinary_receipt_publication_barrier.rs::writer_child`在 `record_outcome` 前发布ready，而handoff描述的是staging后barrier；没有该EXE对应的完整source manifest可核时，保留已执行结果及owner语义来源，**不将它外推为当前源码或所有sender/dispatch并发已绿**。T02-RUNTIME-INTEGRATION所列其余并发/API重复行缺口仍保留。

#### 保全与 UAC status

- 两轮JSON的每例及整体均为 files_preserved/services_preserved/boot_preserved=true；第二轮 final_error=null、active_after=[]。保全清单实际为 **9个现存文件SHA保持＋2个路径始终ABSENT**：现存文件是主EXE、DLL、meta.toml、agent EXE、state/agent.json，以及LocalService和rdlab各两份RustDesk/RustDesk2配置；ABSENT是 `RecoveryBootstrap/rustdesk-recovery-bootstrap.exe`和`RecoveryBootstrap/bootstrap.json`。**未定位“原11 files”的独立manifest，不能称原11个现存文件精确保全完成**，也不将缺失路径记成文件hash成功。
- 双服务均Running，RustDesk PID2892、RustDeskManagementAgent PID2952，PathName/StartName/StartMode及boot时间保持。handoff另报告最终receipt/consent/msiexec进程和本轮fixture目录为空、guest sessions所列进程terminated；这部分按owner报告登记，不伪造独立查询原件或宣称本轮亲查VM。
- **receipt两轮UAC：** owner记录两次先截图确认Windows PowerShell/Microsoft Windows后Alt+Y，实际两份JSON均证实提升成功。UAC交互细节来源为handoff，截图本轮未独立核验；最终consent为空为owner报告。提升成功不消除第一轮DLL setup阻塞，也不是额外测试PASS。
- **T01 c191正向ACL及negative/lock：** `T/t01-statedir-rerun.ps1`仅证明Win10 runner计划两个exact case及finally checkpoint；现有会话交接为两次UAC/黑屏尝试、未产生对应candidate结果。两case分别记 **reported attempted / terminal UNKNOWN**，纠正上文混写的NOTRUN/UNKNOWN；无raw exit、无pass counts、无业务GREEN。旧stage Win11 RED、T02 state-file ACL旧GREEN、receipt两轮提升结果均不能移植。Win11 fresh c191终态亦UNKNOWN，暂停、不重复UAC。

归档完成条件：每个实际执行case保存源/制品摘要、环境/令牌、精确命令/UTC/raw exit、父子计数、失败类型与旧数据、清理和复审；将TEMP证据迁为稳定可复跑资产后再更新证据可用性。本账不把TEMP可读等同跨机器已归档。回归面：本轮仅更新本总账的证据分层、receipt两轮计数/保全及UAC状态，既有代码和运行路径行为无改动。

### Win10 state-dir 唯一 owner 只读封存（2026-10-17）

两例继续记 **reported attempted / terminal UNKNOWN**，raw exit、测试计数均未知。本轮未发起 RunAs/UAC/测试、未改 ACL、未清理。只用普通 rdlab guestcontrol run 采集；令牌 administrator=false，SID 后缀1000。证据在 `T/win10-statedir-readonly-20261017-072527/`：`metadata.stdout`、`metadata.stderr`、`metadata.exit.json`、前后 `sessions-*.stdout/stderr/exit.json` 与 `archive-manifest.json`。采集 guestcontrol exit=0；宿主包装器在文件及摘要归档完成后的控制台打印遇到 GBK UnicodeEncodeError，不是测试退出码，也未据此重跑。

- 观察 UTC `2026-10-16T23:25:28.1521038Z`。旧 guestcontrol start session ID1 仍列出，PID4020 为 successfully terminated；前后清单一致，没有关闭该会话。
- 全进程名称/PID/父PID/session枚举只有本次观察 PowerShell PID4464，没有测试 EXE、遗留 PowerShell worker 或 consent。没有采集命令行或 identity 文件内容。
- `C:\RustDeskAgentLab\tdd-state-dir-result.json` 及根目录两例各自 stdout/stderr 共五个确切路径均 ABSENT。较早 `t01-statedir-*` 子目录存在旧 result/stdout/stderr，仅采文件名、长度、UTC时间、SHA-256、owner、SDDL、presence，不读取内容，不替代本次终态。
- 限定前缀树枚举无错误，未见 fixturestate 条目；保留现存旧测试产物，未清理。此结论只覆盖 `C:\RustDeskAgentLab` 中已列前缀及其非 reparse 子树，其他路径的 fixturestate 状态 UNKNOWN，不能宣称全机无残留。目录 length 原始字段为 PowerShell 适配值1，不代表字节长度；目录 hash=null。
- RustDesk PID2892、RustDeskManagementAgent PID2952 均 Running、StartName=LocalSystem。系统启动时间原始值1792083589865毫秒；本次未执行 SYSTEM 探针，不能声称完成 SYSTEM 文件保全验收。

结果缺失按用户交接归因为 guestcontrol start 的 shell 生命周期中断；本轮证据直接支持“受管shell终止且终态文件缺失”，未采到历史生命周期事件，故因果链按交接判断保留，不升级为独立实证。失败前置检查同样可能使旧 worker 未写 result；没有终态不能补造业务失败或通过。

后续正常运行方式（仅方案，本轮未执行）：由唯一 owner 在可见交互桌面准备就绪后，用一个长期存活的 `guestcontrol run --wait-stdout --wait-stderr` 承载普通父 PowerShell；父进程调用一次已核对 worker 的 `Start-Process -Verb RunAs -Wait -PassThru`，由操作者查看并同意真实可见 UAC。无可见同意不得继续或重发。父进程等待 worker 退出，并在同一进程中验证本次唯一 run-id 的最终 result 已原子落盘、两例 stdout/stderr 与 raw exit 完整后才输出并返回；不另开 guestcontrol session 轮询。worker 必须对前置检查失败、异常、取消和超时写明确终态，不能依赖当前脚本最后一行才落盘。宿主超时覆盖人工同意及测试耗时；超时/取消/缺result保持 UNKNOWN，禁止自动重试或清理。该运行方案仍需后续明确执行授权，本轮仅封存。

回归面：仅本总账补充证据及运行方案；源码和既有运行路径无变化。新观察器与原始归档仅位于宿主 TEMP，未复制到来宾。

### A01/A02 当前聚合回执与编译边界（2026-10-17）

T01–T12全部保留，A01/A02作为新增聚合证据层登记，不取消、不覆盖既有条目，也不把聚合计数回写为原始T01–T12测试结果。

| ID | 当前状态与计数 | 证据绑定 | 边界与下一步 |
| --- | --- | --- | --- |
| A01 | **当前聚合：helper 18 + raw widget 8 = 四文件 aggregate 43 passed；final `--no-pub`。** RED冻结回执为18例，另有camera RED从1 fail修复到green 1；API camera分支bug已修复。 | 四文件及各自current source hash尚须由主采集重新核对；aggregate是本轮最终聚合口径 | 只保留聚合回执语义，不把widget/RED子项重复累加到T01–T12；当前源码绑定、四文件摘要和最终命令需主采集确认 |
| A02 | **当前目标为四文件 aggregate 43 green 的候选口径；helper/model原先有Win10 20 green，但model lifecycle曾3 fail，之后出现`connectionreadyfinally`1 failure并已修复。最终43的当前源码与编译状态仍待核实，暂不记GREEN。** | 必须由主采集取得四文件 current source hash、最终命令及完整43项回执；历史20 green、lifecycle失败和修复后局部回执分别保留 | Android APK/E2E本轮未运行；A02不能据此宣称移动端或完整产品验收完成 |

#### A01/A02 编译与环境审计

- package-name最近一次根仓库编译曾因 `runtime.rs` 字符串截断产生7个编译错误；producer owner修复后已有 `cargo check --locked --lib --features flutter` exit0，最新 `runtime.rs` SHA 为 `0970e87396cb58f797abc2446656524b986fbd1284c4c1d893baad63f68ca845`，仍有warnings。该cargo check只证明库检查完成，不改变package-name七case当前状态。
- package-name七case的当前源码是否仍可compile未知，需由另一owner重新绑定当前主目标、四文件/测试source hash并核验；旧`--no-run`或旧EXE不得冒认当前compile，更不能写成GREEN。
- `Pubspec.lock`可能被Flutter pub自动变更，是否属于用户既有工作尚未完成差异归属审查；在归属核实前保留该审计项，不恢复、不覆盖。
- Win10 state-dir两个candidate的UAC启动回执仍缺失，两个case继续为 **reported attempted / terminal UNKNOWN**，无raw exit、pass count或candidate result。禁止重复blind UAC；必须先有可见同意、唯一run-id、长期存活父shell及原子落盘的逐case终态回执，缺任一项保持UNKNOWN。

本次仅更新ledger；未运行测试、VM、CI或Flutter命令，未修改业务代码、未提交、未使用subagents。
