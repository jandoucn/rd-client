# Agent 恢复、自动升级与桌面回归 TDD 执行计划

整理日期：2026-10-16。状态为本日证据检查点，不代表已发布或全部验收完成。

## 1. 目标与证据基线

目标：异地机器主程序故障时，Agent 能在本地门禁允许的范围内恢复固定 RustDesk 对象；Agent 不可用时，有独立可验证入口；升级、失败、回滚和恢复结果能持久化并在 API、SQLite、后台 UI 中闭环。

本计划合并以下输入，作为后续任务拆分、测试和验收入口：

- [物理机自动升级观察](agent-physical-auto-upgrade-observations-20261016.zh.md)：保留原始观察，不改写成已定位根因。
- [完整升级方案](agent-recovery-upgrade-plan.zh.md)、[测试手册](agent-recovery-test-runbook.zh.md)。
- [实施状态](agent-recovery-implementation-status.zh.md)、[VM 记录](agent-vm-system-driver.zh.md)：历史状态以各自日期和源码为准。
- [通讯录测试交接](../tests/peer_online_reload_handoff.zh.md)、[MSI 快捷方式交接](../tests/windows_desktop_shortcuts_handoff.zh.md)、[EXE 快捷方式交接](../tests/exe_desktop_shortcut_handoff.zh.md)。

### 现场证据分离

| 对象 | 已观察事实 | 本计划不得推定 |
| --- | --- | --- |
| Tiaoma / `510206970` / SOS MSI | 主程序与 Agent build_seq=2026101501；应用 ready、registered=true；双服务 Auto/LocalSystem；12 项能力声明；rescue waiting_approval、unverified、ready=false | 12 项命令均执行过、bootstrap 可用、完整离线救援已准备、服务器当前绑定全部有效 |
| ZYR / `497695003` | 生产只读结果显示绑定和七项权限有效；normal 缺失、fallback 存在且应用已注册；client.enable 因 capability_missing 不可执行 | fallback 全文件完整性已验证、已有远程命令执行回执、允许绕过本地停止意图 |
| Bbraun / `193002918` | 先有 all_sources_failed，后 downloaded/installing；新主程序和 Agent 已上报 1501，查询时缺 installed 终态 | 两次尝试属于同一事务、失败一定是网络问题、健康上报可以自动替代安装成功收据 |
| Win10/Win11 VM | 多批局部 fixture、真实安装及回滚证据 | 可合并为同一冻结源码全绿、可替代上述三台物理机现场证据 |

ZYR 后续本机查询另立一条观测：采集精确时间未提供，服务仍为 Running/Auto/LocalSystem，运行于 standard fallback，normal 缺失、fallback 存在且路径可信；application=unknown，构建和注册字段未取得；rescue=bundle_missing/unverified/ready=false。它不覆盖此前后台 ready/registered=true，也不被此前结果覆盖。空字段不解释为 false 或 0；bundle_missing 与 Tiaoma 的 waiting_approval 分别测试。

2026101501 发布构建与当前工作区必须分别绑定。显示版本同为 1.5.3 时使用 build_seq 比较；未发布工作区能力不得套用到现场二进制。

## 2. 执行合同

1. 每项先冻结可验证行为及失败条件，再编写测试并保留真实 RED；环境加载失败、零测试和缺工具不算业务 RED。
2. RED 后才做最小实现；GREEN 后做边界回归、独立审查和适用真实包验收。现有实现先补行为测试；若旧代码已通过，记录基线通过，不制造 RED。
3. 客户端测试在 VM 执行，宿主仅构建、静态检查和整理证据；API PHP/SQLite/浏览器隔离测试可在宿主执行。测试不依赖生产服务才能通过。
4. 低层测试不能替代 E2E。新业务分支覆盖率目标不低于 80%，报告范围、工具和实际分支数据；无分支记录时记未测，不用行覆盖或历史数量替代。
5. 所有失败断言同时检查错误类型/状态和旧数据保留。维护、disable-agent、disable-recovery、Disabled、用户停止、会话和互斥按实际适用合同验证。
6. 最多 15 个子代理共用并发额度；分清写入文件和证据责任，每台 VM 同时只有一个操作负责人。中断后先核进程、源码、任务和回执再续跑。
7. 本轮先完成本地测试与审查，再安排下一轮发布。固定域名、公钥、签名和手动构建约束不变；物理机观察不授权生产停服、删除、重启或故障注入。
8. 工具已拒绝的断电、故障注入等操作保持阻塞记录，不改写调用方式绕过。快照恢复仅用于清理或场景重置，不是产品回滚证据。

状态使用：`待 RED`、`RED 已证实`、`局部 GREEN`、`待 E2E`、`阻塞`、`闭合`。只有当前绑定源码下适用层级全部通过并复审，才标闭合。

## 3. 优先顺序与依赖

| 顺序 | 工作包 | 启动依赖 | 完成条件 |
| --- | --- | --- | --- |
| P0 | T01 救援初始化、T02 更新终态 | 发布源码与现场观察核对；隔离 fixture | 正常安装/升级能合法初始化或明确失败；终态持久化、补报、ACK 对账 |
| P0 | T03 EXE 联合升级竞态与回滚 | 确认中断任务的最终源码和现场 | 最新源码真实成功及失败回滚，停止意图保留 |
| P1 | T04 恢复阶梯、T05 Bootstrap、T06 远程运维 | T01 批准资源；固定身份及本地门禁 | 分层恢复与真实命令闭环 |
| P1 | T07 通讯录、T08 快捷方式、T09 Defender | 实际桌面目标和正常提权通道 | 用户流程 E2E 与配置读回 |
| P1 | T10 锁定/注销 | 正常真实远控及可核验解锁路径 | 锁定/注销期间注册、画面、输入分别验收 |
| P2 | T11 Purge、T12 中断/覆盖率/交付 | 对应功能稳定、适用执行环境 | 产品注销、事务接续、覆盖率及发布门禁 |

T01/T02 的源码调查与测试编写可同 T03 并行；相同生产文件只能分配一个写入负责人。不能让桌面准备工作阻塞独立逻辑和 API 测试。

## 4. 工作包与 RED/GREEN 验收

### T01：真实发布链路的救援初始化（P0，待 RED）

**行为**：普通安装和升级完成后，从合法批准及签名来源准备与当前架构、edition、包型、实例和版本匹配的救援资源；失败不破坏当前可用程序。

- [ ] 对照发布 2026101501 和当前 `rescue_first_install.rs`、`rescue_status.rs`，查清 first-install.json 的合法生成者、签名输入、时序、原子持久化与生命周期。
- [ ] RED：用真实安装/更新入口复现无首次批准记录时 waiting_approval；区分“尚无批准，正确阻塞”与“应生成而遗漏”。未核契约前不预设所有安装都必须 ready。
- [ ] 单元/集成：缺记录、坏 JSON、未知 schema、签名/大小/哈希失败、跨 edition/架构/包型/实例、过期和部分资源均不得误报 ready。
- [ ] GREEN：合法批准来源才能生成资源和完整验证结果；初始化失败保持应用、Agent、原身份和旧有效资源，并记录可行动的错误码。
- [ ] E2E：普通 MSI 安装、0510→修复候选在线升级后，检查 client、offline_package、credentials、agent_backup、bootstrap_backup、recovery_entry 等实际资源；断网真实恢复另场验证。
- [ ] 状态展示区分支持验证、缓存验证、当前重新校验及 freshness；不能手工造批准记录或直接改 ready。

#### T01-Z：ZYR bundle_missing 与首次批准投影

以下是当前源码的行为规格，尚未执行，不能标 RED 或 GREEN：

| 用例 | 输入 | 必须断言 |
| --- | --- | --- |
| Z-R01 | current bundle 缺失，首次报告缺失 | unavailable/unverified/bundle_missing；只读查询不写资源、配置或报告 |
| Z-R02 | bundle 缺失，受保护首次报告声明 waiting_approval | 提示 waiting_approval，但 ready 仍 false；该提示不表示批准完成 |
| Z-R03 | 首次报告不可读、坏 JSON、不可信或超限 | 不伪造等待或就绪；当前实现保留 bundle_missing，诊断不得猜成权限故障 |
| Z-R04 | current 部分文件缺失，另有 waiting 报告 | bundle_incomplete 不被 waiting 覆盖；旧资源和报告保留 |
| Z-R05 | bundle 访问失败或目录不可信 | source_unreadable/source_untrusted 与 NotFound 分离；不删除资源或放宽 ACL |
| Z-R06 | 旧实例、旧 target、过期或未知 schema 的 waiting 报告 | 先冻结报告绑定契约；当前覆盖逻辑未完整校验这些字段，要求绑定时先补真实 RED，不预造新错误码 |
| Z-R07 | 全部资源验证成功 | 当前契约只到 degraded/fresh、缓存组件 verified；full_recovery_supported/ready 仍 false，不通过改常量补绿 |
| Z-R08 | 缓存过期、时钟回退、来源变动、缓存后 payload 损坏 | 区分历史缓存与当前完整性；24 小时边界和到期全量复验失败均覆盖 |
| Z-R09 | normal 缺失但 fallback 实际运行 | 复现首次准备目前固定 normal 健康链的实际失败阶段；保留可用 fallback、服务路径和身份，不无依据重装 |
| Z-R10 | 报告发布被锁、已有临时文件、verified 成功但 last-check 失败 | 断言具体持久化错误及旧目标保全；两报告不冒充一个原子事务 |

测试入口须覆盖生产 snapshot 的首次报告覆盖逻辑；只测内部 snapshot_at 不足。当前检索发现 first-install.json 的生产读取者与测试写入者，未确认正式生成入口；合法外部交付流程仍待核实。查询未知不等于资源后来被删除。

### T02：自动升级终态与重试对账（P0，待 RED）

**行为**：安装后新程序实际运行时，原事务能恢复并补报终态；首次失败保留，重试和成功可关联且幂等。

- [ ] 调查 Bbraun 观察对应的普通 updater、安装器及 Agent 上报路径；事务关联不足时维持未知，不用版本健康推造 installed。
- [ ] RED：本地真实 API/SQLite fixture 复现 installing 后旧进程退出、新进程回归但成功收据未上报；覆盖安装器成功但健康失败。
- [ ] 测试安装前落盘 pending、网络断开、进程退出、重启、丢 ACK、重复终态、乱序事件及重试 attempt 的关联。不得把 check completed 当安装成功。
- [ ] GREEN：同一身份、事务及目标版本的可信收据恢复上报；ACK 前保留，ACK 后收敛；失败及 rolled_back 不改写为 installed。
- [ ] E2E：SYSTEM 后台策略检查→下载→校验→安装→主程序/Agent/Bootstrap 验证→事件→SQLite→UI；全程无人值守正常执行，SOS 仅后台推送。
- [ ] 服务器策略矩阵保留 disabled/notify/download/auto_install、allow_auto_update、启动/定时触发和 agent 独立开关；未知设备策略预配置是已观察既有行为，不擅加 404 契约。

### T03：旧 EXE 联合升级与进程竞态（P0，RED 已证实／部分局部 GREEN）

- [x] 已验证新增停用标记不得被回滚镜像覆盖；缺省 schema 接受、显式坏类型拒绝，70 项事务 fixture＋8 项身份检查通过并复审。服务/健康替身边界保留。
- [x] 已有旧冻结源码真实 SOS 健康后失败回滚证据；不能替代最新源码。
- [ ] 核对最近中断的源码、构建和测试回执，再处理已证实的误拒：原 PID 正常退出，但另一个合法固定进程存活，导致 Pending。
- [ ] RED/GREEN：安全处理其他已枚举合法进程；原 PID 复用、未知映像及停止后新出现映像仍拒绝；不能仅删条件后漏掉新进程。
- [ ] 单独定位尚未解释的创建时间/映像身份检查 Pending，不将两进程 fixture 结论套为原现场根因。
- [ ] 最新源码 standard/SOS 分场实际成功；真实主程序和 Agent 健康后、提交前固定同步失败，验证旧二进制、配置、身份、Bootstrap、维护和原错误保留。
- [ ] 真实正式在线首跳 EXE＋MSI 四路径；本地 mixed-lab 候选、outer exit 0、CLI root 文本不等于在线升级或旧 Agent 健康通过。

### T04：恢复阶梯与 fallback（P1，局部 GREEN／待 E2E）

- [ ] 建立停止、服务缺失、错误 BinPath、文件损坏、normal 缺失、normal/fallback 双缺失的参数化行为测试，每层明确选择下一步的条件。
- [ ] fallback 校验完整运行依赖、包身份与签名；文件存在但无法运行不能算可恢复。
- [ ] 网络不可达与进程故障分开处理，注册失败不触发无界重装；固定退避和本地门禁测试失败状态与身份保留。
- [ ] 复用已有真实全失恢复、保全和事件补报证据前核源码；按当前冻结补执行欠缺场景，验证应用 ready 和真实注册。
- [ ] 没有客户端回滚锚点时只能报告 Agent 保全或恢复失败，不报客户端 RolledBack。

#### T04-Z：fallback 健康与本机/后台观测对账（P0 调查，待测试）

健康调查与 T01 并行，不等待恢复资源准备完成。服务 Running、路径可信、应用 ready、注册和完整救援分别出结果。

- [ ] normal-only、fallback-only、两者都存在时按可信 SCM 实际指向选择 expected executable；SCM 指向缺失 normal 时不得虚称 fallback 已接管。
- [ ] fallback-only + Running/LocalSystem/OWN_PROCESS 分别注入健康成功和失败，成功为 ready，失败为 unknown；normal 缺失不能单独决定探测失败。
- [ ] 错误 edition 路径、额外服务参数、非认可路径、reparse、未知账号、非 Running 状态分别拒绝或不探测，断言调用次数及只读不变性。
- [ ] 实际 blocking 管道覆盖 nonce 错误/重放、PID/创建时间不符、实际映像不符、build 不符、进程退出、权限拒绝、busy/超时、分片和畸形响应。使用另一存在文件测试映像不符，不能拿缺文件替代。
- [ ] ready + registered=false 是合法且独立的健康结果；失败时省略字段、显式 null、布尔 false、数值 0 分别断言，不用 JSON 索引得到 null 就混同缺字段。
- [ ] 同一冻结 SCM/文件/健康输入下，resident 与 local_cli 的 application 结果一致；只给一侧注入访问拒绝时保留两份独立结果，CLI 不覆盖 resident 缓存。
- [ ] 真实 SYSTEM resident 与提升管理员 CLI 在同一 VM 同一窗口取证，关联安装实例、generation、target、服务/server PID 和创建时间、观察来源、采集时间及后台接收时间。
- [ ] 当前产品没有明确应用采集时间字段时，先由测试 envelope 记录；HTTP 签名时间不是采样时间，generation 不是时间戳。若新增产品字段，另按契约先 RED。
- [ ] 成功后再失败不得保留旧 application build/registered 冒充新结果；身份文件读取失败与应用 probe 失败分别断言。
- [ ] ZYR 本地精确采集时间未知，不能补为先前后台时间；权限、管道未就绪、映像不符、build 不符和真实健康变化均暂为假设，不指定现场根因。

主要落点：management_agent/status.rs、health_windows.rs、windows.rs、health.rs，以及 management_agent_status_contract、management_agent_health_windows 相关测试。先复用已有协议测试，再补生产 snapshot 路径的测试专用边界；不为测试重构所有平台观测实现。

### T05：Agent 自身兜底与 Bootstrap（P1，局部 GREEN／待 E2E）

- [ ] 单元/集成：任务身份、固定路径、manifest/签名/hash、备份版本和当前实例一致；资源失效、任务缺失及策略禁止时明确非 ready。
- [ ] E2E：双服务停止、Agent 文件缺失、开机、周期触发分场验证独立入口→Agent→主程序，记录触发来源和真实时间线。
- [ ] 用户停止、Disabled、维护和 disable 标记场景不得被兜底入口覆盖；持续执行被拒时明确退避与能力边界。
- [ ] 防护软件隔离按环境边界报告，不终止防护或绕过自保护。

### T06：12 项能力与后台真实命令（P1，待 E2E）

- [ ] 逐项覆盖 agent.status、service.inspect/start/restart/repair、recovery.reconcile、package.verify、diagnostics.collect、network.probe、package.reinstall/update、agent.update。
- [ ] 正向链：绑定/权限/能力/实例/代次/策略 revision→签名领取→固定对象执行→事件→SQL→UI；主程序离线但 Agent 在线仍能运维。
- [ ] 负向链：撤权、未知能力、旧实例/代次、重复命令、错误批准包、维护/停止/会话/互斥均检查错误及旧状态保留。
- [ ] client.enable 等服务端类型但客户端未声明项显示 capability_missing；API 编排不能赋予旧执行器未实现能力。
- [ ] Windows x64 MSI 12 项与 EXE 9 项分别核验；批量操作仅按已实现服务命令，不把权限全开当能力执行完成。

### T07：通讯录在线状态与真实连接（P1，局部 GREEN／待 E2E）

- [x] 已完成真实模型/widget RED→GREEN，保留 reload 时在线状态、失败旧数据及 GUID 隔离；已有 30 项局部测试和复审证据。
- [ ] 真实 controller→target 会话自然触发 hash-sync PUT 和 reload；同一 fixture 独占账号，绑定客户端/session、访问日志和 SQL，不允许手动 pull 或注入 online 代替。
- [ ] 两个 online peer、一个已知 offline peer；连接、焦点变化、两轮远端像素挑战及关闭后状态均验证。
- [ ] TEMP HTTP 转发只代表限定账号/通讯录请求隔离，native IPC/其他网络未自动隔离；启动前落实出口约束，禁止生产凭据转发。
- [ ] 核实子窗口 token 和事件归属；最终同时要求 Flutter 测试退出成功、真实新帧和关联 PUT/reload 证据。

### T08：快捷方式存在性（P1，部分真实包 GREEN／待失败回归）

- [x] MSI 隔离 RED→GREEN；当前完整 CA＋官方 SOS1501 payload 实验室包，SYSTEM/per-machine present、missing 两场升级 exit 0，产品 Installed、四份配置摘要保持。
- [x] EXE 原删除竞态和长脚本 Unicode BOM 修复已复审，9 项隔离原生测试通过。
- [ ] MSI 实际安装失败回滚验证产品注册、旧数据及图标存在性；此前 per-user Advertised 回滚失败不可被成功升级覆盖。
- [ ] EXE 实际父进程/UAC/SYSTEM 安装与回滚验收；存在则新版覆盖、删除则不重建、外来对象拒绝，不承诺任意旧图标字段保真或就地写入原子回滚。
- [ ] 正式后续包在线升级仍需验证；非官方实验室 MSI 不当官方发布通过。

### T09：Defender（P1，待实测／历史异常未知）

- [ ] 使用已准备只读观察器采集 before/早期启动/after、精确 UTC、状态、偏好、Operational 5001/5004/5007、白名单策略与 AV 注册信息。
- [ ] 默认不选时不新增信任；opt-in 真实安装后恰当包含五个固定 ExclusionPath、零 ExclusionProcess，CFA 适用模式下两个固定 EXE 允许项；以实际读回判断。
- [ ] 组织策略/防篡改拒绝或读取失败必须报失败/unknown，不伪成功，不关闭防护或扩大排除范围。
- [ ] 既有 standard 重启实时防护 true→false 原因保持 unknown；新场实验只能证明新场行为，不能反推历史根因。

### T10：锁定、注销和桌面接管（P1，待 E2E）

- [ ] 先确认正常注册、控制端连接及可观察正常解锁，再锁定；注销独立场景，不用杀 GUI 替代注销。
- [ ] OS 锁定事实独立取证；WTSActive、命令已接受或采样文件存在不等于锁定完成。
- [ ] 关联 service/server PID 与创建时间、注册、真实握手、新帧和无敏感输入响应，区分 UI 假离线、注册断开与采集/输入问题。
- [ ] Windows 10/11 与 macOS 适用场景分别保留；LoginWindow/TCC/FileVault 不以 Windows 服务测试代替。

### T11：Purge 完整注销（P2，阻塞／局部 GREEN）

- [ ] 补齐受保护 marker/工作目录流程的当前源码 RED/GREEN 与独立复审，先核最后中断及漂移，禁止使用旧测试 EXE。
- [ ] 真实 /x 后验证 MSI 产品注册、双 ARP、服务/任务/固定目录完整注销；失败保留原错误和可恢复数据。
- [ ] 旧 fixture 残留注册与外层收尾失败仍保留未通过，不能仅目录不存在就判完成。
- [ ] 已被工具拒绝的 junction race 不绕过，登记对应未覆盖项。

### T12：中断恢复、覆盖率与交付（P2，待完成）

- [ ] 本地 fixture 覆盖下载 primary/mirror、大小/SHA/签名错误、安装失败、回滚失败、丢 ACK、重复命令、事务接续及脱敏错误。
- [ ] 进程中断、断电、重启分别列场景；未获执行条件或被拒绝的场景保留阻塞，不以手动正常重启替代断电。
- [ ] 新业务分支 ≥80% 的实际报告；明确未插桩进程、平台和 E2E，禁止提高排除项或用零匹配过滤器补绿。
- [ ] 适用的 cargo fmt/test/clippy、Flutter analyze/test、桌面 integration_test 和实际目标构建；记录基线问题与新增回归，不重排无关源码。
- [ ] 更新协议验证 EXE/MSI/AppImage 的 manifest 选择；能运行的平台实际启动，其他平台列明环境限制。
- [ ] 发布前最小化 diff、核七包及固定签名约束；本地通过后再按既有手动发布链运行，任一构建 job 失败按合同停止整轮。

## 5. Android 追加工作包

这两项是对用户新需求的追加，不替换或取消 T01–T12，也不降低它们的验收状态。

### A01：Android 控制端鼠标模式双指滚轮

**目标**：Android 控制端鼠标模式中，两指垂直滑动向被控端发送滚轮；触屏模式保留双指缩放/平移，非 Android 逻辑不变。

- [x] RED→GREEN 纯手势策略测试；覆盖阈值、正反向、累积、方向反转、水平/斜向、指针变化、取消、hold-drag、camera、touch mode 与非 Android。
- [x] 真 Widget 指针测试：`TestPointer` 进入 `RawTouchGestureDetectorRegion` 并核对 `InputModel.scroll`；Android touch mode、模拟 iOS、特殊拖动和 camera 分别断言。
- [x] camera 回归 RED→GREEN：鼠标模式滚轮分支不再吞掉 camera 原有 canvas pinch/pan。
- [x] Win10 Flutter 3.24.5 定向运行当前回执为 43 passed；Dart analyze 对本次文件无 error/warning，仅有既有 info/deprecation。
- [ ] Android 实际 APK/设备验证两指事件与远端真实滚动；跑完整 Flutter analyze/test 与实际 Android build；按现有发布约束只验 standard APK，不切签名/发布配置。

### A02：切回前台后恢复 Android 控制连接

**目标**：连接页确实进入后台且同一 session 收到可识别的 DNS/传输断开或 restart 事件时，恢复前台后对原 session 最多触发一次恢复；健康返回不重连，关闭/替换后不复活，恢复失败仍走原错误 UI。

- [x] helper RED→GREEN：暂停期间故障只暂存；恢复一次性消费；关闭及 owner 替换后拒绝回调。
- [x] 接入 model 的原错误事件流和 connection-ready 状态：DNS 文案针对 Rust lookup 错误；已成功恢复后再次 DNS/传输失败由原 `showMsgBox` 处理，不吞提示。
- [x] RED→GREEN：前台错误和自动恢复失败返回原错误路径；connection-ready 清除 pending failure；close 清理 controller/错误记录。
- [ ] 核对 pause→重试中收到 `connection_ready` 后迟到的原失败事件不得二次重连；补齐 timer/用户手动 Retry 交错测试。
- [ ] Android APK/设备 E2E：真实 session 在后台遭遇受控 DNS/网络恢复错误，切回后确认相同 peer/session 的重试、成功/失败 UI、用户关闭边界；覆盖健康 session 不重连。
- [ ] 生产网络源尚未增加稳定 DNS 错误码；当前 Dart 策略匹配现有底层错误文本，保留鉴权、密码及未分类错误原处理。服务器、公钥、ID/API 域名不变。

两个工作包不改变现有固定服务端、下载地址、公钥、Android 长期签名证书和发布流程。以本节追加状态与 T01–T12 原状态并列跟踪，不能因 Android 项完成而称整份 TDD 关闭。

## 6. 每项证据与收尾格式

### 2026-10-16 本轮实际执行回执

| 工作包 | 实际执行结果 | 仍未覆盖 |
| --- | --- | --- |
| T04-Z 模型/投影 | Win10 新增 6 项及已有 1 项通过；normal/fallback 选择、字段省略、registered=false、CLI/resident 相同输入、读取失败保全 | 真实 fallback SCM、1501 双 token 与后台同窗口对账 |
| T04-Z blocking pipe | Win10 3 项通过：管道缺失、畸形响应、另一真实存在 EXE 的映像不符；错误与文件保全均断言 | 真实驻留应用的正向健康；历史 1501 快照恢复失败，未计通过 |
| T01-Z 生产 snapshot | Win10 SYSTEM 13 项通过、1 个未定绑定契约候选 ignored；真实文件锁、信任失败、缺失/不完整、首次报告及缓存投影 | 新绑定契约、完整资源初始化、source_unreadable 实际拒绝场景和覆盖率 |
| T01 正式批准输入 | 原版本专属 catalog 的两个 MSI target 正式验签；PHP/SQLite SOS、standard fixture 均通过，撤销后 409 且旧行保留 | 正规 producer 尚未接入；不把缺批准时 waiting_approval 认作业务 RED |
| T01 篡改拒绝 | Win10 SOS、standard 各精确执行 1 项，均 exit 0；签名/hash/target 拒绝，输入及原 11 个文件摘要、服务保持 | 未执行安装、首次初始化或在线升级 |
| T02 API | 既有 6 项＋补充 6 项真实 HTTP/SQLite 通过；重复、丢响应后重发、乱序及旧失败保留 | 客户端收据持久化与补报、同 command 多 attempt 契约、后台 UI E2E |

证据索引位于宿主 TEMP/opencode：`t04z-final-20261016/交回.zh.md`、`t04z-blocking-20261016/交回.zh.md`、`rescue-window-20261016/交回.zh.md`、`t01-tamper-win10-20261016/交回.zh.md`、`t02-api-host-20261016/T02-result.zh.md`。源码与制品摘要以各回执为准，不把分批结果合成同一冻结全绿。

首次交付测试的三项审查修正已关闭：TOML 比较失败不输出原字节；未交付批准的安装仅作为缺批准保全观察；worker 新鲜度从安装完成后建立基线。普通 MSI 重装观察尚未执行。新增客户端用例均在 VM 运行；正确既有行为的基线通过不伪造 RED。

每个用例记录：T 编号/场景、需求来源、当前状态、仓库提交及工作区摘要、被执行源码/制品 SHA-256、edition/架构/包型/build_seq、VM/实例/代次、命令与真实退出码、实际测试数、失败类型、原数据摘要、日志/SQL/UI/健康证据、清理结果及独立复审结论。

RED、GREEN 必须说明是否同一环境、哪些代码差异导致行为变化。mock SCM、HTTP fixture、真实服务、真实安装和物理机用户输出分别标注。正式测试入口和稳定证据索引应进入仓库；TEMP 原始回执在迁移归档前不能作为跨机器可复跑的唯一依据。

本文件是新的执行计划，不将所有复选框预勾选。每个工作包稳定后只更新实际完成项和证据引用；出现源码变化，重新判断受影响 GREEN 与 E2E 是否仍有效。
