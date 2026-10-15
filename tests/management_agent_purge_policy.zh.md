# 本地主动 purge 的独立策略切片

本切片只定义本地管理员主动 purge 的调用顺序，不接入现有 purge，
不提供 Windows Installer、注册表、服务、文件系统或 admission 实现。
Backend 是 purge 专属接口，不改变任何共享 Backend。

## 本轮证据

- 宿主用 rustc 1.90.0 --test 编译独立目标，不执行测试程序。
- Win11 普通 rdlab 进程执行，fixture 仅使用内存 identity/source/state 数据对象。
- 历史源码字符串检查：0 passed / 1 failed，FIXTURE_EXIT=101。
  它不是业务 RED；后续审查已删除该测试，历史回执仅保留为缺口记录。
- policy 首次编译因 purge_policy.rs 不存在失败；没有制造硬编码失败实现。
  这是缺失实现证据，不是可执行的 policy 行为 RED。
- 添加独立 policy 后，Win11 7 passed / 0 failed / 0 ignored / 0 filtered，FIXTURE_EXIT=0。
- 首次动态 CRT 测试启动无输出且 transport_exit=126，不计 RED；改静态 CRT 后取得上述有效证据。

宿主证据目录：`C:\Users\Administrator\AppData\Local\Temp\opencode`。
文件：`purge-gap-red-static.guest.json`、`purge-policy-green.guest.json`。
RED EXE SHA-256：`b4c4a7dafbcf1d78428ebb9f8c1c515e70357bdf051a04197d66214106ebb749`。
GREEN EXE SHA-256：`37756181cc33af69fcccd79a074b11b7e08ee81e17b2c466f6aa9298fb20ddbf`。

## 契约边界

RegisteredVerified → installer 恰好一次 → 独立 probe → AbsentVerified 才 cleanup。
1605 同样必须再次确认 AbsentVerified；初始已验证不存在时不启动 installer。
1618、1612、其他失败、Pending、3010/1641、身份不符、多候选、查询未知和仍注册
均禁止 cleanup。fixture 逐项断言调用序列和 identity/source/state 字节保留。
3010/1641 为 RebootRequired，不冒称完全清理成功。cleanup 自身报错为 CleanupFailed。

Product 是策略测试输入，不是安全授权凭证；本模块没有身份构造器、签名验证或
真实注册实例识别。未来接线前必须设计并验证可信身份来源、固定产品和机器 context
绑定、锁和 marker 生命周期、历史 MSI CA、实际 cleanup 自删除与失败行为。
AbsentVerified 也必须包含该固定产品所有相关注册视图已明确不存在的证据。
真实 cleanup 失败可能已部分删除；当前 fixture 的无变更失败不能证明文件系统回滚。

本轮未运行实际卸载、SCM 修改、admission、API、Win10 或线上动作。
没有测量覆盖率，没有实际产品 E2E；7 项 fixture 通过不表示原完整注销缺口已修复。
原 purge/cleanup 和其他现有业务路径没有修改。AOCI 工具未挂载，未进行索引维护。

## 只读发现增量

新增 purge_discovery.rs 及 management_agent_purge_discovery.rs。
首次复用 verify_pair 的 fixture 行为 RED 为 1 passed / 1 failed：替换缓存身份仍被接受。
补充三方身份及版本比较后，最终 Win11 4 passed / 0 failed，FIXTURE_EXIT=0。
包含 Registered/Absent/Foreign/Ambiguous/Unknown 与 policy 组合的内存副作用测试，
以及真实 MsiEnumProductsExW/MsiGetProductInfoExW machine-context 查询。
真实目标仍为 {71A6889B-6003-42DB-989F-B7AE68260DDB}，版本 1.5.3.20261005。
缓存和源包复用 msi_metadata::inspect 只读解析；源由 InstallSource 与 source-list
PackageName 读取，不接受调用者路径。旧 installed_source helper 在本机返回 path
unavailable；保留失败记录，没有修改该共享 helper。

最终证据 purge-discovery-bound.guest.json，EXE SHA-256：
`a02ff26d0162338c5a8cb08f876d9daf38dc260b0359e9dac74c16defefe6b8d`。
RED：purge-discovery-red.guest.json。两次源读取失败分别记录在 green/final 回执，
不能将文件名 green 当作通过。宿主单进程 rustc 编译，未在宿主运行测试。

新模块未接生产模块树。approved_installed 计划复用固定 rescue 的
local_repair_source(None) 校验批准来源，但 cfg(not(test)) 入口本轮尚未编译验收。
元数据一致不是包真实性证明：缓存/登记源的签名或批准摘要绑定、PackageCode、
全视图 Absent 证明、已登记产品改名时的发现、源文件跨读取竞态仍未闭合。
因此 native_candidates 结果仅为观察，不能当作可执行卸载授权；当前 adapter
只用于 fixture，不可接原 purge。没有测量覆盖率，也没有完整身份链通过结论。

未来主路径顺序：验证固定批准来源及完整已注册实例 → 持锁复核事务空闲 →
写禁止恢复 marker → 独立协调器停止恢复入口但保留身份/源/状态 →
固定已验证 ProductCode 的 runtime installer → 明确终态与原实例全视图复查 →
确认不存在才清理永久资产和自身。任何拒绝/Pending/重启待完成保留数据，
admission 拒绝原样返回，不释放锁或修改参数绕过。runtime adapter 和上述
身份缺口未验收前，不实施这条接线。

## 身份链续作检查点

新增注册复查行为 RED：purge-identity-red.guest.json 为 4 passed / 1 failed，
旧 adapter 在第二次查询未知前已调用 installer；新增双读一致检查后通过。
purge-identity-held.guest.json 为 6 passed / 0 failed，含原生只读枚举、
PackageCode 交叉比较、fixture 源持有句柄拒绝写入（错误 32）且旧字节保持。
该回执不包含随后新增的缓存摘要检查运行证据。

approved_installed 已增加固定 state/当前 identity 持有句柄、edition/kind 校验，
复用 local_repair_source(None) 的批准引用、签名和 DPAPI 当前身份核对，再以
package_gate::verify 和 lock_verified_file 重验批准 MSI；缺 rescue 仍返回错误。
verified_native 对批准 ProductCode 精确查询前后复核，仅 259 表示枚举不存在，
同时要求两个 HKLM ARP 视图明确 NotFound；源与缓存需匹配批准摘要/大小及 PackageCode。
缓存经过 Installer 改写、与批准原包不等字节时保守拒绝，当前不支持重建等价缓存。

src/management_agent.rs 仅增加两个 cfg(windows) 模块声明以标准编译非测试入口，
没有调用或修改 purge。非测试构建进行中，结果以 purge-standard-build.json/log 为准；
禁止将 cfg(test) 6 项通过当作批准链已执行。签名损坏、缺批准、身份替换和完整
非测试批准链 guest 验收尚未完成；这一检查点不宣称完整只读身份切片可用。

## 非测试构建确认

标准 `cargo build --locked --bin rustdesk-management-agent -j 1` 已完成 exit 0。
已修复新增 MSI summary FFI 与既有声明的类型不一致，再次标准构建通过；
不是通过 cfg(test) 或 stub 依赖绕过 approved_installed。agent 未执行。
最终源码和产物清单：TEMP/opencode/purge-identity-checkpoint.json。
purge_discovery.rs SHA-256：525bbc1daf2ab8bda249d65487db63b5bcb9b8d864483bb2e15176382abbb5cc。
非测试 agent EXE SHA-256：39c322498e3a835bc1a59de26824e4fdc1cbdeaf26b72883a0353dbef2dc3e3c。

未闭项保留：approved_installed 的真实签名/缺批准/state身份替换矩阵没有 guest
执行回执；verified_native 非测试双视图 Absent 和摘要拒绝矩阵也没有运行回执。
metadata fixture 的公开构造器仅 cfg(test)，不能当作批准授权。
当前 helper 复用恢复验证门禁，在 disable-recovery 已存在或当前身份缺失时可能拒绝，
不能直接用于 marker 后继续 purge 或已被旧 purge 清空的现场。
注册双读只检测观察期间变化，不提供对外部 Installer 的原子锁保证。
源码存在并已编译不等于这些运行时契约已验收；完整只读身份切片仍未交付通过。

## SYSTEM 非测试实际拒绝检查点

标准 build 同时构建 agent 和 examples/purge_identity_readonly.rs，exit 0；
discovery/policy/package_gate 三个标准 test target --no-run 也 exit 0。
example include 实际 management_agent 模块，未 stub 依赖，未使用 cfg(test)。
普通用户执行返回 purge identity outside fixed profiles；随后专用 SYSTEM 任务
执行相同 EXE，返回 `READ_ONLY_REFUSED=update signing key is not trusted`，退出码 1。
这不是批准链成功，也不能据此断言正式公钥验证了实验室源包。没有更换 key，
没有 labkey fallback，没有继续 msiexec/SCM/cleanup。

证据：purge-native-readonly-run.json、purge-native-system.json。
SYSTEM fetch 最后的任务状态查询因普通用户无权限而 transport exit 33；程序输出和
独立 exit 文件已经取得，退出码 1 独立于该查询错误。
本轮专用 RustDeskPurgeIdentityReadOnly 任务已移除，cleanup 回执 exit 0；
RustDesk PID 3868/5104、agent PID 7028 保持。没有全量文件摘要基线，不能用 PID
不变声称所有身份/source/tasks 字节已完成前后比对。

最新清单 purge-identity-checkpoint.json：
- management_agent.rs：f1b3dcf7fe023df77635f6658cd6e8e8e19407e483d3fc9d41663a33f69dd4fc
- discovery：525bbc1daf2ab8bda249d65487db63b5bcb9b8d864483bb2e15176382abbb5cc
- discovery tests：7bf496fd78326f93e3090035abd5602e9d135be562f32727e734bf7969a65c9e
- readonly EXE：fadc585c2112a2cf0caee7c6c77944f0fe9880e878d793e3af4edd4acbc971fc
- agent EXE：fa5a50c033afa9e156fc3836328696d927995597f99ad290afc3cb8b1bcd29c0

安全停点是生产批准验证真实拒绝。原 purge 仍无新调用；锁生命周期评审、正式签名
成功分支、真实摘要/Absent 拒绝矩阵和注销验收尚未完成。

## Approved 清理自锁缺陷闭环

协调器改为接收 owned Approved；在最后的 Absent 复核后释放共享的真实文件
locks 和生产 LockedPackage，再调用同步 cleanup。已验证值身份在 cleanup 返回前
保留，外层仅在 cleanup 成功返回后报告 Complete，没有异步 spawn。
测试与生产使用同一个 locks: Vec<File> 字段，不以 cfg(test) 删除文件锁。

Win11 真实隔离临时文件：purge-lock-red.guest.json 为 6 passed / 1 failed，
旧借用协调器因共享锁导致 CleanupFailed；purge-lock-final.guest.json 为
8 passed / 0 failed，包含自持锁删除错误 32、释放后同步删除成功、外来锁仍占用时
CleanupFailed 且剩余字节保留。只删除测试创建的临时文件，没有产品目录操作。
最终测试 EXE SHA-256：37d36a434f969851d6a70eb1e9499abe8e7d581dc61fe4a57ed8181bc49295e6。
最新标准 agent/example build exit 0，三个标准 tests --no-run exit 0。

当前 discovery SHA-256：42871a8a095453cc0e772be4ef5091b55e6b4157d6fb1a31aab2838dac0ff94a。
当前 discovery tests SHA-256：8c8524a14d540ef057119213c82b97394f8f7bc8a37043211843b39cd5379087。
完整最新清单仍为 purge-identity-checkpoint.json。

上一轮已落盘的主动批准改用 verify_readonly_status 代码仍是未验收候选；
本轮未扩展门禁范围，也不以锁测试通过宣称 disable-recovery/stop/事务矩阵通过。
原 purge 保持旧调用路径，无新卸载接线；未运行真实 MSI/SCM/product cleanup。

## 主动身份候选文件验收

新增 purge_approval.rs，生产 installed 与 cfg(test) fixture 均调用同一 verify；
仅 fixture 显式注入公钥，不修改生产信任。approved_installed 复用其 Evidence，
不再在身份识别内部执行管理员/InstallGuard 授权；这些由后续执行入口负责，
当前不提供执行授权。事务 decode 检查仍拒绝非终态及未知数据。共享 rescue
恢复门禁未修改。

Win11 SYSTEM 仅执行指定测试
purge_approval_signed_files_preserve_stop_markers_and_refuse_foreign_active_bad_signature：
1 passed / 0 failed / 139 filtered，56.53 秒，exit 0。
使用既有批准 bundle 的隔离副本和既有显式测试公钥，实际运行签名/资产/当前身份
验证；没有 MSI 解包。disable-recovery 和 stop Y 的本地文件字节保持。
foreign instance、当前身份不匹配、Applying 事务以及生产 key 不信任均拒绝，
逐场比对 state/identity/marker/intent/bundle 字节。真实原 bundle 字节保持。

证据：purge-approval-artifact.json、purge-approval-guest.json。
非test agent/example build exit 0，标准 rescue tests --no-run exit 0。
隔离 fixture 目录已清除；RustDeskPurgeApprovalFixture 专用任务已移除；
原 PID 3868/5104/7028 保持。未留下本轮运行中的 fixture。

证据限制：候选已在之前改成只读 verifier，没有人为复原错误实现；本轮没有
取得旧恢复门禁业务 RED。测试名称虽含 bad_signature，实际负例是生产 key
不信任 fixture key，尚未覆盖签名字节损坏；也没有真实 InstallGuard 竞争测试。
stop intent 文件不被身份 verifier 消费，本测试证明其保持，恢复流程仍需独立
回归确认。这些未闭项不能由 1 passed 替代，不接原 purge，待主agent独立review。

## 独立负例与恢复门禁对照收敛

本轮 Win11 首批 5 个独立测试通过，181.33 秒；证据 purge-approval-guest.json。
签名同一受信 fixture key 的第一个签名字节翻转，真实 package_gate 验证器精确返回
`install manifest signature verification failed`；完整批准入口先返回
`rescue approved reference mismatch`，两层断言不能混称整链执行到签名。
包截短返回 `rescue verification size mismatch`；同长度首字节翻转返回
`rescue verification hash mismatch`。逐场保留被测 state/identity/marker/intent/bundle、
manifest 字节以及故意损坏的包摘要，未自动修复输入。

purge_preflight::acquire 是独立执行准入，仅管理员检查与现有 InstallGuard，
不接身份 verifier，不接原 purge。父进程持真实测试命名互斥，独立子进程返回
`purge execution busy`；父释放后再次取得成功。child 入口不计空跑，父测试断言
子进程确实 1 passed。该测试不验证生产管理员拒绝分支。

第二批成对回归 1 passed，54.23 秒，证据 purge-approval-pair-guest.json：
同批准数据 + disable-recovery：主动识别成功，原 first_install::check_gates 返回
`first installation locally blocked`；无 marker + stop Y：主动识别成功，原门禁
返回 `first installation user stopped`。被测状态和标记/配置字节保持。仅在隔离
fixture 切换场景时删除自己创建的 marker。共享恢复模块仅增加 cfg(test) 转发，
没有改生产判断。此为旧/新策略差异证据，不声称制造或修复一个新的业务 bug RED。

标准非test agent/example 与标准 tests no-run 均通过。两批各自绑定制品，不能当作
同一个 EXE 的全量通过；最新源码/制品见 purge-identity-checkpoint.json。
本轮 fixture 全部退出、隔离目录清理，专用任务移除，原 PID 3868/5104/7028 保持。
没有全量生产目录摘要验收，不据此声称完整运行时无变化。

可交独立 review 的契约：身份读取不受恢复 intent 门禁影响；批准签名、实例、
当前身份、包内容及非终态事务仍必须有效；执行端另取管理员和安装互斥；
身份返回不是执行许可。生产 key 拒实验室资料仍保持，无 trust fallback。
原 purge 未接线；最终原生注册/缓存竞态、清理和实际卸载尚不在本 slice 通过范围。

## P2 maintenance preflight

purge_preflight::acquire 现在返回持有 InstallGuard 的 Context。取得锁后立即
symlink_metadata 固定 state/maintenance.json；任何对象存在均返回
`purge execution maintenance present`，只有 Win32 2/3 明确不存在放行，
其他查询错误返回 `purge execution maintenance unknown: ...`。
Context::recheck 提供后续每个副作用前的相同只读检查；不解析、删除或接管维护文件。
不参与同一 mutex 的外部写入者仍可能在复查后写入，不宣称双读提供原子事务。

Win11 RED：purge-maintenance-red.guest.json，0 passed / 1 failed，旧入口存在
marker 仍返回 Ok。GREEN 同制品普通用户 5 passed / 1 failed，跨进程被既有
mutex ACL 拒绝，保留该失败；SYSTEM purge-maintenance-system-guest.json 为
6 passed / 0 failed，0.03 秒，exit 0。包含文件、目录、junction、无效路径查询
unknown、无 marker + disable/stop 保持、持锁后新维护复查、真实子进程竞争。
测试 child 的空入口在父进程被过滤，仅由父明确启动并确认 1 passed。
测试 EXE SHA-256：62e71b196ded067acced5295001ce82ad79ec9389d440e3fe76e90cc62102283。
查询 unknown 使用真实无效路径，不等同于 ACL AccessDenied 场景已验收。
管理员生产检查保持，但本轮 cfg(test) 未测试非管理员分支，不冒称通过。

非test agent/example build 和 preflight 标准 test --no-run 均 exit 0。
SYSTEM 隔离文件均清理，专用任务移除；普通用户失败 fixture 的独立临时目录
可能保留作为失败证据，未接触产品。原 PID 3868/5104/7028 保持。
原 purge 无新接线；后续 native backend 必须使用 verified_native，不能用
Adapter 的 metadata classify 当完整批准验证；Absent 后释放 Approved 文件锁
再同步 cleanup 的契约仍保持。本 P2 实现与上述证据交独立 review。

## 执行顺序 fixture 检查点（未接生产）

新增 purge_execution.rs 专属 Backend（不修改共享 Backend）：recheck → verified
probe → recheck → 持久禁恢复动作 → recheck → installer → recheck → 再 probe
Absent → recheck → release_approval → 同步 cleanup。保留 Backend 返回的原始
String 错误；1605 与 0 都必须复查，Busy/缺源/失败/Pending/重启待完成不 cleanup。

Win11 purge-execution-fixture.guest.json：3 passed / 0 failed，0.06 秒；EXE SHA-256
a76885b4702f5df4879d35ebf4ac586aca472c345073ea33820cf0231508c77d。
只用真实隔离临时文件与模拟 installer/probe，测试自身清理临时目录；没有 MSI。
本轮无有效 RED；不能冒称完整 TDD 或 native 执行通过。

未完成：native Backend、verified_native 接入、严格 EXE/MSI/state 路由、
独立签名/hash helper、自删除 Pending/完成回执、持久错误日志、VM 健康快照及
真正注销验收。原 purge 没有新调用，只增加模块声明用于非test编译。
本检查点不代表完整用户目标已完成，不应将此执行器直接接线或运行真实卸载。

## native backend 未完成检查点

purge_native.rs 已有固定系统 msiexec /x、Approved 私有 ProductCode、
verified_native 前后查询和持久 disable-recovery 实现；无任意路径/命令参数，
无 admission override。标准非test agent/example build 与执行器标准 tests
no-run 均 exit 0。没有新 guest/native installer 运行证据。

执行入口当前在副作用前明确拒绝：外部 helper 的签名/hash/plan/回执和自删除
生命周期未实现，不能先卸载再必然失败于 cleanup。原 purge 尚未接新路径。
这属于实现缺口，不是工具审核拒绝、权限限制或不可修复的环境阻塞。
未创建 VM 健康快照，未运行 msiexec/SCM/产品删除；没有本轮残留构建/VM测试任务。
完整任务仍未完成，EXE/MSI 严格路由、helper、错误回执及真实注销均待继续。

## 外部 helper fixture 最终回执

最新 purge-handoff-guest-1792021940993216500.json：2 passed / 0 failed，7.29秒，exit0。
测试真正复制当前二进制，检查 helper hash/固定事务路径/plan id，父 PID/create-time
存活时 execute_once 拒绝且写独立 progress Pending；父退出后两个 helper 竞争
create_new claim，claim 绑定 plan SHA256、nonce、Owner。只胜者执行隔离文件删除，
释放文件锁后删除成功才由动作返回 typed Complete，终态写 result.json。
父测试等待两个并发子进程都完成断言；第三个顺序 helper 拒绝且原 result 字节不变。
wrong helper hash、foreign transaction 保留 fixture 原数据。成功审计目录保留，无自启动。

历史 RED 是受保护 workspace 根重复创建失败；随后 owner 不合格属于 fixture
启动位置失败，改为 SYSTEM 复制至受保护目录后通过。没有取得重复claim漏洞的
旧实现行为 RED，不能追认这一项 TDD。终态失败→Complete 拒绝分支尚未单独测试。

完整源码/制品：purge-handoff-checkpoint-1792021997896051800.json。
handoff SHA256 3dfb385cedb9be1fbaadcafa601b64bbbe793089c9f083b5d4ef2b5bc604ae99；
测试 EXE SHA256 830e8d38181ecc2efd990a4a4f5b9b317a57625c0a7d468a68898f6cba82d68a。
manifest 已包含全部 purge 源、当前测试及 agent/example/test EXE，不复用旧清单。
标准非test build/test no-run通过，任务已移除，purge-helper进程数0，原PID
3868/5104/7028保持。未操作产品目录、MSI、SCM或API。

接线状态：helper身份/单实例/父退出/回执基础行为可交review；实际批准重核、
native backend claim后的安装和清理、错误回执完整性、CLI Pending handoff仍未接。
原purge保持旧路径，native execute仍拒绝，不应把fixture通过当生产注销通过。

## 错误回执 RED/GREEN 与旧证据更正

新增 error detail 测试时发现 child done 字节数组长度不同导致编译失败，驱动曾
复制旧 EXE；1792021940993216500/1792022245414763000 的 2 passed 不证明新增
done/error 断言。已修编译错误并给驱动增加编译退出码检查；复制时遇 sharing
violation 等待构建退出后重试，没有执行重复 installer。

有效 RED：purge-handoff-guest-1792022560429849300.json，2 passed / 1 failed，
错误回执缺少 error 导致子断言不完成。补丁把原始错误写入终态，若保存回执失败
仍组合保留原错和保存错误。
最终 GREEN：purge-handoff-guest-1792022768134534100.json，3 passed / 0 failed，
10.74秒，exit0。包含真实父子/双helper/顺序拒绝、错误终态不再执行与字节保留。
当前测试 EXE SHA256 54715fa0c9e7be5891d6a778627c1a6159ca5d9febc51b653a995ba85c8900b6；
handoff源码 SHA256 233b79a060bab04a7e8945a0826e6821078ea4ff1bb27ed5e6647212a6c2d3e5。
本轮没有生产 CLI 分派/native cleanup 接线和实际 MSI；完整任务未完成。
## 最新生产薄接线检查点

### 2026-10-15 Win11 native fixture 新绑定 GREEN

本轮仅调整 tests/management_agent_purge_native_service.rs 与 logs 内测试驱动，
purge_service.rs、purge_native.rs 业务源码保持。宿主仅 jobs=1 --no-run 构建，
Cargo JSON 唯一 compiler-artifact 复制到唯一 EXE；新 guest 路径、新 SYSTEM 任务、
result nonce 与 EXE hash 逐项绑定，copyfrom 使用全新目标并比对 guest 文件 hash。

最终 nonce：344f615071b748c7829a6cca88bec246。
证据目录：logs/purge-native-344f615071b748c7829a6cca88bec246/；
实际回执：receipt-3edeb40ab5f14e53bd2bed2f3d89a313/result.json。
三项分别 1 passed / 0 failed，exit 0：initialStopped 5.73 秒、Running 5.57 秒、
真实 COM 3.46 秒。测试 child 只由 SCM 启动，不算父测试通过数。

initialStopped 确认 SCM Stopped/PIDNone 时原进程存活，release-exit 前注册与
payload 字节保持；release 后首次调用遇退出枚举竞态，精确返回
`purge process image query failed`，注册与 payload 保留；ObservedProcess 确认
原进程退出后测试重试成功，才删除隔离 payload。这是安全拒绝与显式重试通过，
不能描述成首次调用直接成功或生产自动重试已实现。Running 路径一次等待真实退出。
此前同一竞态的 exit101 原始失败保留在 nonce bc74139ffb5a42b086dca852bec831c4，
不将其冒充两项既有业务修复的旧实现 RED；该历史 RED 缺口仍在。

COM 测试以 include_str 提取当前 native 两段真实 PowerShell，只替换固定任务名为
RustDeskAgentTestTask-PurgeCom；实际 COM missing 解 InnerException 得 0x80070002，
Stop/Delete 脚本均成功。创建真实专用任务后，Stop 完成再写隔离 maintenance，
实际 Context::recheck 返回 purge execution maintenance present，未调用 Delete，
任务 XML 与 maintenance 字节保持。未直接调用生产固定任务入口及 Native::cleanup。

最终 EXE SHA256：d102e07f73f16ff128070d775732f88ca54f257962f6f816a608aa40871a7e7b。
源码清单 SHA256：128c8a3f664d68a36d8b24e63d61482bd541fe9575fa8eea9a4429ba2179786d。
purge_service.rs：ea50e139e9667efdba71ab3f748d6a4aeccb976727127a153a3b8dcf415fd7c6。
purge_native.rs：384a8d6d24b8ac85f254b17bd1c1d302ab2289f30dbcc4c56989e2b9ac61f569。
native test：b1718ffe2cb1e513131af0b2a5caee11b35034293912e6b872f12e1b47cbc454。
source-before/source-after/source-final 对应源码摘要一致，evidence-hashes.json 封存日志。

前后应用 ready；服务 PID5104/agent7028/GUI1856、安装身份、身份文件、agent 与
源 MSI hash 保持。04:33:04Z 最终核验无 msiexec、fixture 进程/服务/目录、
本轮任务；本轮 guest 可执行文件和 worker 已删除，证据日志保留。
没有真实卸载、产品服务 Stop/Delete、产品目录删除、API/Win10/发布操作或 commit。
本结果仅关闭这两组定向 native fixture 验证，不替代生产卸载整链验收。
AOCI 工具未挂载且仓库无 .mcp.json，未能执行 AOCI 维护。

### 三项 reader review 修正候选

#### 初始Stopped与COM异常续作未通过

purge_service initialStopped/PIDNone 不再 unwrap_or(false)，改为固定basename
候选进程枚举+ObservedProcess+完整image验证；任何候选查询失败拒绝。COM三处
GetTask catch按既有helper_runtime解InnerException后只认0x80070002。未实测COM。

新真实fixture service编译先因ServiceAccess::ALL_ACCESS不存在失败，改明确权限。
实际测试先遭命令行未加引号的无空格fixture路径拒绝，修严格等价形式；后续仍
未形成可靠RED/GREEN。最后回执1792036987807596900引用旧2500ms断言，而当前
源码已改release-exit同步，明确属于旧制品/旧任务证据绑定故障，不能认作本版结果。
停止重跑并清理专用任务；query确认测试服务不存在、固定fixture目录不存在，
原GUI1856/服务5104/agent7028保持。标准build/no-run通过不代表native验收通过。
两个P2尚未关闭：native SCM初始Stopped/Running矩阵及真实COMmissing/maintenance
仍待正确冻结制品重新验收；未运行产品MSI/SCM/任务删除。

approved_installed 不再使用调用者 Config::file，purge_approval::installed 仅尝试
两条固定 service profile，按完整 state/bundle/签名/identity摘要验证；存在的
foreign 或不可读候选拒绝，多候选有效也拒绝。没有全局 Config 切换。
新增 purge_service：固定 SCM 命令/账号检查、ObservedProcess PID/create句柄、
进程映像路径核对、停止后同时要求 Stopped 和进程退出，再删除注册。
task Stop/等待与 Delete 拆成两次固定COM脚本，中间 Context recheck，删除前
再查 task instances，未知失败保留。

本轮有效guest回执 purge-handoff-guest-1792035377251812100.json：三项各1passed，
CASE_EXIT均0。身份测试完整批准fixture：single成功、ambiguous/foreign拒绝并保字节；
服务测试仅wait_stopped查询回调，并未实测fixture SCM/PID/image；任务测试仅
Context维护复查+临时task文件保持，未执行实际COMfixture。没有本轮有效旧实现RED。
因此不能声称用户要求的三项native行为闭环全部完成。

标准非test build/test no-run通过。曾遇VirtualBox guest session满，逐个关闭已
确认进程终止的旧session；脚本执行策略导致旧输出被误读取，旧handoff回执不计本轮。
最终三项回执有明确独立测试名。专用任务已移除；未运行真实产品MSI/SCM/任务删除。

### 后续源码核对更正

当前生产 MSI 路径已经可达，不再是 explicit-refuse 占位。已修 child 重复 claim，
由 execute_once 唯一 claim 并在其中获取 Context、重新 Approved、对账 installation
和 ProductCode；父 start 获取 Context/verified_native；CLI Pending 为退出码10。
state 固定根/edition/kind/target 明确校验，禁止 serde 默认 kind 意外放行。
marker 新文件先验父目录；日志移至外部 helper 事务目录，避免成功清理丢日志。
cleanup 检查固定外部 helper 位置、维护状态、固定任务停止/删除/不存在、服务
stop/delete 返回码及 query1060，并检查两个固定目录删除后不存在。

上述本轮修改仅非test build/test no-run通过，没有对应新guest行为证据。旧
handoff3GREEN不能覆盖这次native接线。没有真实MSI、SCM或产品删除，没有本轮
健康快照；原service PID5104/7028保持，GUI PID1856（已不同于旧3868）。
这份候选不能分发或用于实际purge；仍待新CLI/marker/log/task/service/cleanup
定向测试、保全快照与实际固定MSI注销。不是工具拒绝，是实现/验收未闭。

`purge()` 已按固定 agent state 的 `package_kind` 分流：MSI 进入 Pending helper
启动，EXE 保留 legacy 路径，unknown/read failure 拒绝清理；CLI
`--purge-helper --transaction-id` 已由 main 薄包装接入。只改专属 purge 模块和
main hook，没有修改 workflow、Android/macOS 签名、旧 updater、API 或共享 transport。

标准非test agent/example build 与测试 no-run 均 exit 0。当前 Win11 RustDesk PID
3868/5104、agent PID 7028 保持，helper 进程为 0。

native 路径仍未开放：child 已能取得固定 helper plan/Approved，但完整生产闭环还
缺少真实 Absent+双 ARP probe、固定服务/任务/目录存在性确认、installer log/result
receipt 绑定以及当前 helper 不在删除范围内的安全证明。`Native::cleanup` 当前仍
含直接 SCM/目录删除，不能运行。没有执行真实 MSI、SCM 或产品清理。

这不是工具拒绝；是为避免半卸载而保留的源码机制缺口。原 purge 仍未被 native
入口直接替换，等待上述闭环和独立 review。

### 2026-10-15 marker 接管原生文件 slice

接管时 purge_marker.rs、purge_native 的 HeldMarker 薄接线和三项测试已存在。
原产品 RED 直接核验 TEMP/opencode/purge-e2e-3cb3e03796964dc88acc95327ea53285/
failed-helper-evidence/result.json：workspace DACL inheritance not protected；
该失败没有执行 MSI。此次未重新执行产品 purge、MSI 或产品 SCM 操作。

补四项测试：CreateNew 竞争保留胜者、Context 拒绝与祖先 rename 拒绝、已有可信
marker 字节/ACL 不变、SYSTEM 建立 fixture 后真实 rdlab 创建不可信 owner marker。
宿主 jobs=1 no-run，Cargo JSON 唯一 artifact 冻结；Win11 SYSTEM 七项实际执行
7 passed / 0 failed / 0 filtered，exit 0，6.35 秒。测试只导入 marker 模块，
无 installer 调用。静态 junction 拒绝与固定句柄 rename 拒绝分别验证；尚未完成
真实普通用户并发 ancestor junction 替换压力测试，不把两者合称完整竞态验收。

证据：TEMP/opencode/marker-takeover-a167becc466a42339026aa34b276ccf2/
checkpoint.json、guest.json、cleanup.json、build.jsonl、build.stderr。
EXE SHA256 16f3ae6d388e5df693ee7282753d2213c5db61dd2ee39b775725910ddcab5b57。
marker 源码 SHA256 4c6b5165b8030a0578dc1441165db534cf9e4c33c0480b70abbec504dcf0bed8。
native SHA256 a354c16bcc9a016428e4da66e343363277faffd41a7e06ce86422d9731f1c5c9。
test SHA256 83a7e9cde1af6fedc1f6dabf52c6e2d5f7b56d83eebae55068c95a94242dcb80。

管理员清理回执确认本轮任务及 guest 制品目录已移除，产品 PID1856/5104/7028
保持，无 installer/helper。发现接管前已存在 fixture bfb5a6b3-d63c-4284-b21b-addfddbf3cc1，
未擅自删除。health 仅进程/测试任务层面，未重做711文件健康摘要。
本次业务源码没有进一步改动，回归面仅新增 marker 测试；不代表全量 purge 验收。
未运行全 workspace/Flutter/覆盖率，AOCI 工具未挂载且无 .mcp.json，未维护索引。
