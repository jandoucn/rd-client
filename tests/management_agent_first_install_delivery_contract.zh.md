# T01 首次批准交付契约与 Win10 保全观察交接

状态：2026-10-16，独立review三项修正完成，Rust `--no-run` 重编退出0；原始正式catalog输入已取得，PHP语法检查及SOS/standard两个真实SQLite正向/撤销保全fixture全部通过，双输入包已封存。**本用例是缺批准保全观察，不是业务RED；未来producer的入口及契约定义后另建RED。** 未运行客户端测试，未操作VM/生产/CI。优先交接无installer tamper测试；普通MSI重装须独立审查并准备基线。

## 已证实的契约和边界

需求来源是 `docs/agent-recovery-tdd-plan-20261016.zh.md:57` 的 T01：普通安装及升级后从合法批准、签名来源准备救援；缺批准应阻塞，不能无条件要求 ready。此需求尚不规定每条路径必须使用 sidecar。

| 路径 | 源码事实 | 对 T01 的含义 |
| --- | --- | --- |
| 手动 Flutter Tag Build | `.github/workflows/flutter-tag.yml:203` 组装快照；`:254` 调用 OSS 发布；`.github/workflows/publish-oss.yml:88` 用 UPDATE_SIGNING_KEY 调用发布器 | 发布签名来源在发布器，不应由客户端或测试伪签 |
| 签名 manifest | `.github/scripts/publish-release-to-oss.py:161` 定义签名 payload；`:283` 构造含 size/hash/signature/key_id 的 targets；`:374` 发布 API 后读回核对 | 先有最终 MSI/EXE 字节，后有包含最终包 hash 的签名 manifest。把该 manifest 反嵌同一 MSI 会改变包 hash，不能这样补 producer |
| MSI 构建 | `res/msi/preprocess.py:473` 只生成 bootstrap.json；`res/msi/Package/Components/RecoveryBootstrap.wxs:14` 初次目录与任务交给受门禁约束的 Agent startup | bootstrap.json 只有 Agent hash/edition/kind/normal path，不是 first-install 批准，也不含最终 MSI 的发布签名 |
| 首次读取 | `src/management_agent/rescue_first_install.rs:13` Approval 严格只允许 schema/product_code/manifest；`:631` 读固定 release-cache/first-install.json；缺失返回 waiting_approval；`:658` 验签；`:666` 读取 MSI 注册源；`:677` 真实健康检查 | 消费者存在，但源码未给普通安装接入签名 manifest 的生产交付步骤。sidecar 本身不是私钥签名文件，权威来自内部 target 签名与注册包字节绑定 |
| worker | `src/management_agent.rs:1606` 启动首次 worker；`:1766` notify；`rescue_first_install.rs:897` 固定布局、300秒节流、落 first-install-status/last-check.json | worker 只会消费已有批准；已存在 current 时直接退出，不会补全输入 |
| API approved package | `D:/github/rustdesk-api/sqlite/agent_operations.php:124` 引用摘要；`:164` resolve 校验实例/代次/命令/权限；`:179` 加当前版本 rollback | API 返回目标 manifest 与回滚 manifest，没有 product_code，也不写客户端磁盘；不是“普通首次安装批准”端点。没有合法命令时拒绝是正确行为 |
| MSI transaction prepare | `src/management_agent/operations_runtime.rs:65` resolve 与下载；`:146` prepare Request；`:173` 交接前重新 resolve；`preparation.rs:122` 将批准 manifest 写入事务目录 | 准备阶段不能伪造 committed 或发布新安装救援。不能把 API 返回成功等同安装成功 |
| MSI transaction commit | `src/management_agent.rs:273` 和 `:303` 在 Committed、finalize/drop 后调用 after_commit；`rescue_assets.rs:904` 只接受已提交 MSI，`:941` 读事务 manifest，`:1187` 发布 current | 已有无需 sidecar 的正规 producer 路径。不能对这条路径强加“必须写 first-install.json”测试 |
| 普通在线更新 | `src/updater.rs:1321` 下载验签；`:1340` 只传包路径等给 update_new_version；`:2098` 不接收 manifest；`:2113` 走 update_me_msi，成功后删下载包 | 普通更新没有把已验签的完整 manifest 移交首次准备；下载临时包还可能与注册源寿命冲突，后者需实际包验收，不能凭此认定现场文件被删 |
| EXE | `src/platform/windows_exe_migration.rs:199` 解析显式本地 approval 参数；`:614` 写 inbox manifest，`:623` 调独立 exe-transaction-prepare；`exe_preparation.rs` 处理 EXE 事务；首次 reader 只接受 MSI key | EXE 不属于 installed_msi sidecar 合同。本交接不把 EXE 未产生 MSI sidecar 记为 bug；EXE 救援能力需独立规格 |

这些是逐路径源码证据，不是仅以“搜索没有 writer”判 bug。普通 MSI 路径未交接完整 manifest，是已观察的接线状态；当前没有既有正式 producer 函数或 API 合同可供直接调用，不能以缺 sidecar 断言业务缺陷。下面仅为待审查的候选设计；待合法批准入口定义后另建 producer RED，不造 approval。

## 发布 2026101501 的证据限制

本地 HEAD 是 `9d8d430c01811965c7585d3b57530481eaa15174`。`git show 0251f6b67:version.json` 显示 build_seq=2026101501、build_number=20261015.1、source_commit=7fb14007816d4473f4798b9d50f3baa6392d13f6。该提交的 first-install reader 和无生产 writer 的发布目录检索与上述路径一致。

本地未找到匹配 `*2026101501*` 的 tag；本轮没有访问 GitHub/CI。续查已存在的 `C:\Users\Administrator\AppData\Local\Temp\opencode\release-2026101501-38709818173-verification\verification.zh.md`：既有封存回执绑定 tag=`v1.5.3-build-2026.10.15-01`、run=`38709818173`、checkout=`9d8d430c01811965c7585d3b57530481eaa15174`。其第43行明确说明 Release 没有签名 stable.json 附件；本轮目录及内容检查与之相符。version.json 内旧 source_commit 字段不能取代该 checkout 证据。

本轮重算两正式MSI与snapshot哈希，均匹配封存记录。该目录没有签名更新manifest；release-snapshot.json不可冒用。此前因此阻塞。后续用户交付原始catalog，按正式发布器映射并验原签名后解除输入阻塞，见末尾最新回执；未向生产API补取，不声称恢复了当时stable.json原始字节。

## 最小受限 producer 方案（未实现）

建议在首次准备所属模块新增自包含的 `deliver_installed_msi_approval`，由受保护的 Agent 首次准备协调入口调用。其职责只是交付输入，bundle 的既有健康、身份、fallback、DPAPI 和发布校验继续由首次 reader 执行。

1. 输入为既有固定 API manifest 地址取得的原始签名 manifest、已识别本地实例/target/当前 build、Windows Installer 查询得到的 ProductCode 和注册源；没有任意 URL、任意路径、脚本或命令参数。普通 updater 可将已验签原 manifest 经受限安装交接保留；裸 MSI 安装后由 Agent 协调入口从固定源解析精确当前 build，latest 不匹配就阻塞，绝不降级或猜批准。
2. 先检查固定目录 ACL/reparse、本地禁用/维护/停止意图/互斥；`package_gate::verify` 校验生产 key、edition/arch/kind、精确 build；锁定注册包验证 size/hash，检查 MSI 产品及注册 basename。ProductCode 来自本地 MSI，不由发布 workflow 预猜。
3. **时序**：普通安装已完成、注册源可验证、实例及包身份已建立后，在首次 reader 本次 prepare 前原子落 `C:\ProgramData\RustDesk\ManagementAgent\release-cache\first-install.json`。不得在安装中、未提交事务期间，或健康目标未确定时覆盖旧批准。远程 MSI 事务仍走既有 commit→after_commit，不需要并行写 sidecar。
4. 文件沿用 schema 1 `{schema,product_code,manifest}`，小于等于 262144 字节；同目录 create_new 临时文件、sync、受保护 rename。重试同一绑定幂等，不同已有批准/有效 rescue 保留并报明确错误；临时失败不删除原包、旧 sidecar、身份、当前程序或旧 bundle。
5. 生产公钥不变；测试 key 仅 cfg(test)，正式 gate 必须拒绝测试 key。没有允许的精确历史批准来源时保持 waiting_approval 并诊断缺失阶段，不能把 API command resolve 改成无命令授权或随意拼下载地址。

这不要求改变 MSI/EXE 签名、macOS 签名、workflow 或生成 committed 事务。

## 本次新增可执行 fixture

`management_agent_first_install_delivery_contract.php`：先用既有生产公钥验证冻结 manifest 的原始 target 签名（payload 与 libs/base/src/update.rs:1043 对照），再调用真实 API 的 agent_binding_authorize、agent_status_store、agent_package_resolve，隔离 SQLite memory DB，构造同版本批准重装的目标＋rollback reply；直接查数据库确认 resolve 不篡改 command 状态；撤销 release 后断言 HTTP 409 与旧 status 保留。只写全新测试输出目录的 resolved.json，不写 sidecar。Rust 正式 gate 仍独立复验签名及包字节。这是 API 函数/数据库集成 fixture，不覆盖设备签名 HTTP 路由。

`management_agent_first_install_delivery_contract.rs`：两个显式 ignored 的 VM-owner 测试。默认执行可能筛出零测试，**必须使用 --ignored 和下面的精确名称**。

- `t01_formal_approval_tamper_refused_preserves_original_inputs`：正式公钥验证输入，篡改 hash 保留旧签名必须精确返回 signature verification failed；跨包型 reply 绑定失败；原包 hash 不变。无安装器调用、无网络、无服务变更，不使用 test key；仍须在 Win10 执行，且需要原签名 manifest 生成的 resolved.json。
- `t01_ordinary_msi_missing_approval_preserves_installation_observation`：仅 RDLAB-WIN10，已安装正式同版本 MSI、无 rescue/transactions/sidecar 的干净基线。冻结 API reply→正式验签→双服务固定契约及真实1501 IPC健康/无会话→loopback原包下载→固定msiexec同版本重装。**安装进程返回后才重新读取报告mtime基线**，只接受随后mtime变化且读前读后mtime相同的完整JSON发布（最长360秒）。观测证据保留安装完成UTC、基线采集UTC、基线/新报告mtime、观测UTC、测试PID、前后双服务PID及健康IPC PID；不向worker报告注入schema或时间字段。验证缺批准时unavailable/waiting_approval/ready=false、无新bundle、身份/配置/文件/实例保全。身份与配置使用布尔比较和固定脱敏失败消息，不输出TOML原字节。测试不写sidecar、不造Committed。

在当前无批准入口的基线上，预期观察waiting_approval且旧安装保全；不能要求未来sidecar产生，也不据此记业务RED。缺正式包/manifest、验签失败、无注册源、非Win10、安装非零/3010、无安装后新报告均为输入/环境阻塞。已有或后来出现合法批准时不适用缺批准观察，保留现场，不删除批准。

## Win10 owner 命令

先在隔离 API 测试环境导出，不触碰生产。`out` 必须不存在，父目录已存在；正式 manifest 从冻结发布证据复制，禁止手填 signature/key_id。

```powershell
php tests/management_agent_first_install_delivery_contract.php D:/github/rustdesk-api C:/RustDeskAgentLab/frozen-1501-manifest.json windows-x86_64-msi-sos C:/RustDeskAgentLab/t01-delivery-input
```

将原始正式 MSI 以 manifest 中 primary 的 basename 放入 `t01-delivery-input`。为 standard 测试应重新建立独立现场并用对应 target 导出。SYSTEM 执行以下两个测试；构建可由宿主完成，运行只在 VM。

```powershell
$env:RUSTDESK_T01_DELIVERY='ordinary-msi-1501'
cargo test --locked --test management_agent_first_install_delivery_contract t01_formal_approval_tamper_refused_preserves_original_inputs -- --ignored --exact --test-threads=1 --nocapture
cargo test --locked --test management_agent_first_install_delivery_contract t01_ordinary_msi_missing_approval_preserves_installation_observation -- --ignored --exact --test-threads=1 --nocapture
```

日志目录由测试打印，为唯一 `C:\RustDeskAgentLab\t01-delivery-<uuid>`，保留resolved.json、ordinary-msi.log、observed.json、原下载包，用于保全观察审查。owner记录测试数、退出码、失败断言、输入摘要；没有producer业务RED断言。

## 未覆盖及后续责任

- 该用例是“正式包输入→普通MSI安装入口→缺批准worker”的本地保全观察，未覆盖首次全新安装、0510→1501在线updater、EXE、断网恢复、rollback全流程。mtime与UTC/PID只建立测试观测时序，不代表worker报告已有实例/freshness字段契约。
- 一旦实现 producer，扩展真实 updater E2E 和离线精确 manifest source fixture；不能只让测试自己调用 producer 来替代产品入口接线。
- 当前 schema 1 sidecar 无 installation_id/过期字段；本轮不修改该 contract，不能宣称已有实例/时效绑定。producer 应核当前实例且 reader 应重复验证；若需新增字段，先与该 owner 冻结版本契约及迁移测试。
- 仓库只改本前缀的 3 个文件，无生产路径行为变更；未改任何其他 owner 文件。TEMP 保留本轮编译日志和测试 EXE供 owner 接续。AOCI 工具未提供，仓库也未发现 aoci.txt/.mcp.json，未建立或改写索引。

## 2026-10-16 续接实际编译与输入回执

工具定位依据为 T02 既有回执和 TEMP 构建脚本，未以 PATH 缺工具结束：

- PHP：`C:\Users\Administrator\AppData\Local\Temp\opencode\php\php.exe`，INI=`...\php-agent-test.ini`；实际 `-v` 为8.5.11 NTS VC2022 x64，`-m` 明确列出sodium/pdo_sqlite。T02 专属8.3 php子目录已不在，仅保留其日志；实际使用共享 PHP 工具，不冒用T02运行版本。
- Rust：`...\rustup\toolchains\1.90.0-x86_64-pc-windows-msvc\bin`，VC=`C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat`；`CARGO_HOME=...\cargo`、`LIBCLANG_PATH=...\libclang-agent\clang\native`、`VCPKG_ROOT=...\vcpkg-agent`、`CARGO_BUILD_JOBS=4`，复用 `...\exe-transaction-v218-target`。1.75 工具链存在，本次使用既有全仓成功构建的1.90，不声称验证1.75 MSRV。
- 实际命令：`cargo test --offline --locked --test management_agent_first_install_delivery_contract --no-run --message-format=json`。38.14秒，Cargo `build-finished.success=true`；独立 test target compiler-artifact `fresh=false`，编译成功。启动宿主工具报告超时后，先核对本次 cargo/rustc 进程，再读取完成日志确认，未重复执行构建。未执行 EXE，包括未调用 --list。
- crate 注册：Cargo 自动发现 `tests/management_agent_first_install_delivery_contract.rs`；这是独立 integration-test crate。通过 `#[path]` 将 package_gate/package_download/msi_metadata/workspace/handoff/health/health_windows/service_control 编译为该测试 crate 的兄弟模块，`super::` 在该结构可解析；不是访问生产 librustdesk 的私有 management_agent 模块。已由实际编译确认，无需修改 Cargo.toml 或生产模块可见性。
- PHP `-l` 退出0；以指定封存目录的 release-snapshot.json 实际运行 fixture，退出255、抛 `INPUT BLOCKED: 输入不是正式签名更新 manifest，release-snapshot.json 不可替代`。失败发生在 SQLite初始化前，未创建输出目录、未生成 resolved.json。**真实 SQLite 正向执行次数=0；不能报 PASS。**
- `rustfmt --check --edition 2021 --config skip_children=true` 可执行，报告该新测试的格式差异；不是编译错误。本轮未对被包含的生产文件应用格式化。限定 `git diff --check` 不检查未跟踪文件，不用它代替编译结果。

初版证据：TEMP根下 `t01-delivery-build.stdout.jsonl` 和 `t01-delivery-build.stderr.log`，仅作历史记录。**review修正后的当前证据**为 `t01-delivery-review-build.stdout.jsonl`（第992行test compiler-artifact、fresh=false；第993行build-finished.success=true）与 `t01-delivery-review-build.stderr.log`，实际构建命令退出0。PHP `-l` 再次退出0。未执行客户端EXE（包括--list），无installer/tamper业务运行回执。

### 交付对象与固定摘要

测试 EXE（宿主绝对路径）：

`C:\Users\Administrator\AppData\Local\Temp\opencode\exe-transaction-v218-target\debug\deps\management_agent_first_install_delivery_contract-e5e658960d4c71c7.exe`

当前EXE SHA-256：`21d993bdf940b7f935df120ddbf23975e46dea30b6a0fcdf8f781cc0c8abad73`。同路径已由review重编更新，初版哈希失效。

绑定测试源码SHA-256：Rust=`73f434f4994a76f6d0ca14073a99058592c17a97bee56aaa6262cf1f185b0187`。review重编后没有再修改Rust源码。PHP随后增加catalog合法映射、原始输入保留及SQLite回执导出，旧PHP摘要不再代表当前版本。

正式包根：`C:\Users\Administrator\AppData\Local\Temp\opencode\release-2026101501-38709818173-verification`

| 输入 | 本轮实算 SHA-256 | 可用性 |
| --- | --- | --- |
| rustdesk-1.5.3-2026101501-sos-windows-x86_64.msi | 17f8ee985719936177c7b8c3cf4078e6879882dfde7bbd3d6f9ca37af15ad519 | 已封存可交付，29461582字节 |
| rustdesk-1.5.3-2026101501-standard-windows-x86_64.msi | 6a37d22d6857b75318a175b160e6368beb4e3462ba5cc3ce33a4b878bf9189bc | 已封存可交付，29482072字节 |
| release-snapshot.json | dee017abaedcedb27e8911b846e8297334847f32f96c698b96251204d224dc56 | 仅发布身份证据；不可作签名manifest |
| 原始catalog及映射resolved.json | 见末尾最新清单 | 已取得原始签名并通过PHP/SQLite；VM运行未执行 |

待原始签名manifest取得并通过PHP fixture后，将 EXE 与原 MSI、resolved.json 一起移交；输入固定放 `C:\RustDeskAgentLab\t01-delivery-input`。VM不必有Cargo，原生EXE参数为：

```powershell
$env:RUSTDESK_T01_DELIVERY='ordinary-msi-1501'
& '<测试EXE完整路径>' --ignored --exact t01_formal_approval_tamper_refused_preserves_original_inputs --test-threads=1 --nocapture
# 下一条会重装，当前未授权执行；独立审查与基线准备通过后才运行。
& '<测试EXE完整路径>' --ignored --exact t01_ordinary_msi_missing_approval_preserves_installation_observation --test-threads=1 --nocapture
```

每条预期精确选择1项；其余来自导入模块的既有测试必须被过滤。**当前仅将第一条无installer的tamper测试交给Win10 owner执行**；第二条保全观察仍待独立安装审查和基线。缺输入、0项、安装失败和环境错误不算RED，缺批准等待本身也不是RED。输入包已齐备，不代表VM验收完成。

## 原始catalog解除阻塞与最新Win10交付

用户提供原始输入 `C:\Users\Administrator\AppData\Local\Temp\opencode\original-1501-catalog-gawm0glt.json`，4234字节，实算SHA-256=`acb6f930b3384d5576d60351f3d18de08716ab614b28162e5253e6b95d1d974c`。来源为 `https://download.yan.life/rustdesk/stable/v1.5.3-build-2026.10.15-01/catalog.json`。

其结构是正式发布器`:617`生成的assets catalog，不是UpdateManifest；PHP按同一发布器`:283 build_manifest`映射release/primary/mirrors/size/hash/signature/key_id及固定product/edition/channel。签名逐字保留，生产公钥验证实际payload成功，没有签名生成或私钥操作。映射视图不标为原stable.json字节，包引用只用于本地SQLite fixture。`original-input.json`保留原4234字节，fixture-receipt记录来源URL、输入摘要及映射实现出处。

实际执行PHP8.5.11 `-l`通过；以原catalog分别运行两个target，均退出0：

- 正式target签名有效；真实SQLite建立绑定与status，resolve得到同版本目标＋rollback且manifest相同，command仍accepted。
- update_releases.active真实置0后resolve抛409；完整旧status和command行相同，撤销值仍为0。
- 导出resolved.json、原catalog和fixture-receipt.json（含SQL行证据）。SQLite为memory，进程结束即释放，无生产库、无设备HTTP签名路由测试。

交付目录（均含对应原官方MSI和哈希为21d993…的测试EXE）：

- SOS：`C:\Users\Administrator\AppData\Local\Temp\opencode\t01-delivery-sos-input`
- standard：`C:\Users\Administrator\AppData\Local\Temp\opencode\t01-delivery-standard-input`

各目录的 `handoff-manifest.json` 是明确的文件SHA-256、guest输入目录、环境变量、精确EXE参数及禁止installer执行清单。

| 文件 | SOS SHA-256 | standard SHA-256 |
| --- | --- | --- |
| resolved.json | 2ca7049c9dabda9899182184184cf4c6eeee6cfeb7604bbf54f1af71df42da45 | 264d324adb4100fe829b2d7f5c67706955733016d30b89c9d2febafcde1a725a |
| fixture-receipt.json | ffaca1ea06c3ded50cbc7f1166aaa1dc671483000c64d9b20f38198089f69557 | 3c6306046f38bab5235c9cb216b8b0b9d51053cc8af395fdbc9690e690c40a95 |

两目录的original-input.json均为acb6f930…，t01-delivery-tests.exe均为21d993bd…；官方MSI摘要见上表，复制后实算相同。原包与EXE均未执行。

主Win10 owner串行选一个edition，把该目录完整复制为 `C:\RustDeskAgentLab\t01-delivery-input`（不得与另一edition混合覆盖已有测试现场），核对handoff-manifest后只执行：

```powershell
$env:RUSTDESK_T01_DELIVERY='ordinary-msi-1501'
& 'C:\RustDeskAgentLab\t01-delivery-input\t01-delivery-tests.exe' --ignored --exact t01_formal_approval_tamper_refused_preserves_original_inputs --test-threads=1 --nocapture
```

测试只读本地正式输入并在内存篡改副本，检查正式gate拒绝及原MSI摘要保留；不会调用msiexec，不启动服务，不访问网络。正式Rust gate对PHP生成的package_id与原签名进行独立复验，尚待VM运行确认。没有first-install.json写入；普通MSI观察依然不执行。
