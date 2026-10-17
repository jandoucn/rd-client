# T01 installed state目录ACL：待真实VM候选

## 最新：86856 候选 Win10 两项实跑通过

2026-10-17T05:51:33Z—05:51:41Z，按持续测试授权，复用最终冻结候选，在独立客体目录 `C:\RustDeskAgentLab\t01-owner-86856-20261017` 正常 RunAs 一次，两个精确 case 各运行一次。Shift 仅唤屏，一次截图核验预期 PowerShell UAC 后正常确认；父子取得 Handle、等待退出并 finally 写回执。未新增提权权限或修改产品 ACL。

| case | actual counts | raw exit | 耗时 |
| --- | --- | --- | --- |
| t01_producer_installed_state_inheritance_readonly_candidate | 1 passed / 0 failed / 0 ignored，14 filtered | 0 | 0.97s |
| t01_installed_state_negative_and_lock_preservation | 1 passed / 0 failed / 0 ignored，14 filtered | 0 | 4.96s |

合计两个测试通过；negative 内部六个拒绝分支不重复计作六个独立测试。正向输出 strict_directory 拒绝未保护继承 DACL，而 producer=Ok(())、preserved=true，原批准/身份/config/state/marker 字节和 root/state/cache/批准 SD 保持。

negative 实际使用当前提升用户 `S-1-5-21-1206211974-4030670733-347995936-1000` 构造 owner，读取后确认等于 TokenUser 且非 SY/BA。delete_child、owner、write_file、lock_failed、cache_readonly、reparse 均输出 rejected=UntrustedPath、preserved=true。真实 symlink 创建及拒绝已走到；末尾第二写句柄和目录 DELETE 句柄 sharing violation、第二读句柄成功及 producer 成功断言全部执行通过。两项 stderr 均为空，父进程 raw exit=0。

EXE SHA256 `86856d600dbead1e0a011afde130d2e2930de4a53a1439edd5144dd861123bfa`，运行后相同；test SHA256 `1e51111814426cad3caa81dae0c77e5f04eb9cce1766ddb81e52d03cfca98056`。producer `15a884c47ed55190864a1d647383bd98642f65c5e9053b38c3a5d8a6c888edd9`、reader `ab719ddbc8b8ba048d04a9b23ceb66d9644c1102fa9656ddf24661ea40dbd002`；三份 source 运行前后完全一致。本次没有修改测试或生产代码。

保全回执：119 files、142 ACL，摘要/SD 变化均为空、数量一致；两个服务状态、PID、启动方式、账号及路径一致。本轮 stdout 识别的 8 个 UUID fixture 已先记录 cleanup.json，再删除唯一测试文件 symlink 和自身目录，cleaned=true。三个原 fixture 仍在，旧 pending 未操作；客体本轮候选和回执目录保留。没有产品安装、host 测试、Android/Win11 操作、CI、commit 或子代理。

本次独立证据目录 `C:\Users\Administrator\AppData\Local\Temp\opencode\t01-owner-86856-20261017`，包含 sourcehash.json、summary.json、result.json、parent.json、positive/negative.stdout/stderr、cleanup.json、uac.png、worker.ps1 和 candidate.exe。下述 setup-blocked 记录为历史失败，最终候选已由本段实跑结果补齐验收。通过范围仅为这两个隔离 fixture 用例，不扩展为安装或产品整链验收。

## 2026-10-17 Win10 negative setup 独占收尾

接手核验当前产物为 `46c06227fdad9ac9cbb1504356625e1afe1f3d8eda7668b0263535b036541fca`，未运行旧 c191。先用既有 `t01-state-acl-build.cmd` 独立清单、offline/locked、jobs=1、no-run 编译当前 source，随后仅修改本测试文件。

badowner 改为 OpenProcessToken/GetTokenInformation(TokenUser) 取得当前提升用户 SID，在 CreateDirectoryW 创建时指定合法 owner；排除 SYSTEM/Administrators，创建后 GetNamedSecurityInfoW 必须确认 owner 等于该 SID。未增加特权。安全设置返回后立即保存 last_os_error，再释放描述符，错误明确标为 SETUP BLOCKED。

本轮 Win10 只执行两个精确 case，各一次，实跑 EXE SHA256 `854fd7635f1210bb591ca21a3864d539c540703d70f2445b2e45dd6fa12a21a6`，测试 source SHA256 `25ad5b53da315d6dcf69ad022d955c85981528273f09dc9dff3fdcecdbc1d949`：

| case | 实际结果 | 精确失败阶段 |
| --- | --- | --- |
| t01_producer_installed_state_inheritance_readonly_candidate | exit 101；0 passed/1 failed；0.60s | SetFileSecurityW 给 fixture root 设置 SYSTEM owner，1307 |
| t01_installed_state_negative_and_lock_preservation | exit 101；0 passed/1 failed；0.55s | 首个 delete_child 的空 `G:` 导致描述符转换 1337；尚未进入 badowner |

两项均为 setup-blocked，不是业务 RED。依据该次准确错误，最终仅修正 fixture：正向只应用 DACL/UNPROTECTED，保留 workspace 创建的合法 BA owner；delete_child 的 group 明确为 BA。最终 no-run 编译成功，按本轮两次执行限制未追加 VM 实跑，badowner 与全部 negative 分支的最终 GREEN 尚未验收。

最终源码/产物绑定：

- producer：`15a884c47ed55190864a1d647383bd98642f65c5e9053b38c3a5d8a6c888edd9`。
- reader：`ab719ddbc8b8ba048d04a9b23ceb66d9644c1102fa9656ddf24661ea40dbd002`。
- test：`1e51111814426cad3caa81dae0c77e5f04eb9cce1766ddb81e52d03cfca98056`。
- 最终待实跑 EXE：`86856d600dbead1e0a011afde130d2e2930de4a53a1439edd5144dd861123bfa`。

证据根 `C:\Users\Administrator\AppData\Local\Temp\opencode\t01-owner-close-20261017`：summary.json、sourcehash.json、source-after.json、parent.json、result.json、positive/negative.stdout/stderr、cleanup.json、uac.png；最终待测源码与 EXE 在 final-setup-candidate 子目录。实跑 executable 使用客体 `C:\RustDeskAgentLab\t01-owner-close-20261017\candidate.exe`。一次 RunAs、Shift 唤屏、一次截图核验预期 PowerShell UAC 后正常确认；父子均取得 Handle、等待退出并 finally 写回执。

119 个受保护文件摘要、142 项 ACL、文件/ACL 数量及两个服务状态/PID 前后保持。只清理本轮 stdout 确认的两个 UUID fixture，cleanup manifest 已导出；三个原 fixture 仍在，旧 pending 未操作。客体本轮候选及回执目录保留供核验。未安装产品、未操作 Android/Win11、未运行 host 测试、CI、提交或子代理。

回归面：仅 `tests/management_agent_first_install_approval_delivery.rs` 的 negative owner/setup、错误阶段诊断及正向 ACL fixture setup 改变；这些改动用于避免非法 owner/无效 SDDL 阻断真实业务断言。生产 producer/reader 摘要保持；本文件只记录交接。AOCI 工具未提供且未发现 aoci.txt/.mcp.json，未写认知资产。

## 最新：真实RED后复用实现候选

已核t01-statedir-vm-stdout.txt：strict_directory=Err(DACL inheritance not protected)、producer=Err(UntrustedPath)、preserved=true，0passed/1failed。当前直接#[path]复用同一preservation_state_read.rs，没有total_loss功能桥接依赖。该reader的workspace导入改为父模块路径；旧total_loss父模块仅补等价workspace导入，既有调用签名、ACL判据、生产固定路径检查未改。producer读取已安装root/state时持有Vec<File>到返回；协调前置锁在HTTP前释放，交付时重新取得。cache/sidecar严格规则不改。

cfgtest显式线程scope只绑定C:\RustDeskAgentLab下本功能UUID root及精确intent，祖先检查到隔离root为止；生产继续检查所有真实祖先，不接受测试路径。旧reader测试不设置scope，分支行为不变。

冻结目录TEMP/opencode/t01-state-acl-green-freeze-20261017，EXE management_agent_first_install_approval_delivery-9258a400f1932ed8.exe，SHA256=c1910d99b7c2b41f06f9021fd9ce498d5b3ee899c7db0e90756a4a19312f7bad。host no-run成功6.32秒，完整worker模块图另no-run成功12.52秒，未执行测试。

Win10同环境先执行两个精确case：

```powershell
& '<新EXE>' --ignored --exact t01_producer_installed_state_inheritance_readonly_candidate --test-threads=1 --nocapture
& '<新EXE>' --ignored --exact t01_installed_state_negative_and_lock_preservation --test-threads=1 --nocapture
```

各1项，环境RUSTDESK_T01_PRODUCER=red-formal-sos-1501。负测包括DeleteChild 0x156、不可信owner、已有文件写权限、真实独占文件锁获取失败、cache Users只读拒绝、真实symlink拒绝，以及调用时第二写句柄/目录DELETE句柄sharing violation、第二读句柄成功。symlink前置缺权限必须报环境阻塞，不计reparse通过。负测保留原批准及身份/config/state bytes；正向旧case还保留目录/批准SD字节。VM尚未运行，不标GREEN；以下历史方案保留。

## 2026-10-17 官方ACL已采集，RED后最小方案（未实现）

已读TEMP/opencode/t02-win11-state-acl-20261017/result-corrected.json：ManagementAgent owner=SYSTEM，state owner=Administrators；均DACL auto-inherited、未protected、非reparse；SYSTEM/Administrators FA、CreatorOwner OICIIOID GENERIC_ALL、Users OICIID 0x1200a9、Users CIID 0x116。agent.json只读Users不能独立证明目录合同，但此回执已补目录证据。release-cache不存在。services_preserved=true。不能用此0510现场推定未来1501安装目录一定相同；目录候选case真实RED仍待交回。

最小复用形状：

```rust
// local_repair_total_loss.rs：只增加窄桥接，已有caller不变。
#[cfg(windows)]
pub(crate) fn hold_installed_state_read(
    state: &std::path::Path, intent: &std::path::Path,
) -> Result<Vec<std::fs::File>, String> {
    state_read::verify(state, intent)
}
```

producer固定生产root/state路径调用该桥接，把返回Vec<File>绑定到函数作用域直到返回；不能调用现有verified_state_read_fixture，因为它map(|_|())丢弃所有锁。不暴露directory_acl、不复制宽ACL判定、不改已有state_read::verify调用者或签名；可见性只扩新桥接pub(crate)。scopecheck：问题限于首次批准读取已安装root/state，现有恢复流程不用占位参数，不改其执行路径。

需要替换三个仅属于本功能的前置：coordinate_current_installation的root/state hold、deliver_installed_msi_approval的固定root严格预检、deliver_bound的root/state hold。固定profile验证完成后才请求专用reader，避免新路径绕profile门禁；production仍限制固定root/state与Config2::file。HTTP协调期间若保留这些句柄会阻止文件写入，宜在只读前置检查作用域结束后释放，再由producer在安装互斥内重新取得并持有到最终发布；不持安装互斥等待HTTP。既有producer自身state/identity/config读锁、signature/build/注册包检查、发布前gate复核继续执行。

release-cache仍workspace::create/verify、目录持锁；existing批准仍verify_file。父root的Users 0x116意味着可能创建新子项，不能把这个许可传播给release-cache：已存在不可信cache必须拒绝，创建竞争失败必须拒绝，不能接管或修ACL。目录句柄share_mode(3)禁止共享DELETE，现有reader逐祖先OPEN_REPARSE_POINT并检查非reparse；保持这些句柄本体，不能另开验证后立即释放。现有reader仍用路径查询ACL，不声称它提供未实现的基于句柄ACL快照或防恶意管理员改ACL能力。

测试接入注意：现有state_read::verify在cfgtest只对固定路径及其专属preservation fixture走installed合同，任意UUID路径走strict分支。不能直接换调用后假定当前UUID RED能绿，也不能扩大所有测试路径豁免。RED后需为本测试增加仅cfgtest、作用域受限且不影响其他caller的路径绑定，或使用该专属既有fixture路径并先确认不存在、绝不沿用旧fixture的remove_dir_all清理逻辑。此测试接入选择应与主协调冻结。

RED后必补负测（均先断言错误及旧bytes/SD不变）：

- Users CI 0x116变0x156（包含DELETE_CHILD）拒绝，既有允许ACE不得泛化。
- Users获得现有agent.json、service-recovery-enabled或intent的FILE_WRITE_DATA/APPEND/WRITE_DAC/DELETE分别拒绝；目录接受不等于文件可写。
- state owner改Users、无SYSTEM/BA完整访问、未知ACE/未知flags分别拒绝。
- state/祖先reparse或替换尝试：仅用正常第二句柄请求DELETE验证sharing violation，不做绕过工具限制的junction race；原目录与文件保持。
- 持锁期间第二写句柄访问已有state文件应sharing violation，第二只读句柄应成功；release-cache的Users只读和可写ACL均仍拒绝。
- 父目录允许创建不意味着接管新cache：预置不可信release-cache，要求拒绝且ACL/内容不改。

本轮仅准备方案文档，未改production/readers、未启动构建或VM、未复跑GREEN、无CI/commit/subagents。等待目录case真实RED再实施上述最小改动。

只读核实preservation_state_read.rs:128的专用合同：固定state/profile绑定，逐祖先拒reparse/非目录，打开不共享DELETE的目录句柄，directory_acl逐ACE校验；state必须SYSTEM/Administrators owner，完整trusted访问；特殊Users CI 0x116/0x110与明确只读权限有精确分支，不是任意可写目录许可。文件继续verify_restore_target并持只读锁。release-cache/root不属于可直接套用的宽松state契约。

既有fixture preservation_state_read_fixture.rs:114声明installed继承SDDL，并要求专用reader接受；后续0x156、Users owner、文件可写、intent可写、maintenance不可信、symlink均拒绝。该测试源码不是当前官方机器SDDL回执。当前仅官方agent.json Users readonly已知，state及祖先目录rawSDDL仍需主安排Win11owner只读收集。

本轮仅新增`t01_producer_installed_state_inheritance_readonly_candidate`：在全新独占fixture的state应用上述installedSDDL；root/cache继续严格；文件readrestore_target前置有效；预先放同reference合法sidecar使producer走只读幂等路径。调用真实deliver_at，先检查旧批准、state、身份、config、marker字节及state/root/cache/approval安全描述符完全保持，再期望Ok。若Err(UntrustedPath)，证明该候选ACL与当前producer契约不相容；不等于官方state已确诊。所有ACL写仅fixture设置阶段，producer调用不得修改它们。

生产模块未修改，不修hold_directory、不放宽root/cache、不调用verify_restore_target验证目录。尚未执行，不称RED。主取得本用例真实失败以及官方目录证据后再决定精确state专用reader/祖先句柄复用方案；0x116不可泛化放行。该case沿用Win10 fixture门禁与正式SOS输入，Win11owner当前只收rawSDDL，不修改机器名绕case门禁。

```powershell
$env:RUSTDESK_T01_PRODUCER='red-formal-sos-1501'
& '<新EXE>' --ignored --exact t01_producer_installed_state_inheritance_readonly_candidate --test-threads=1 --nocapture
```

必须1项。host仅编译，禁止无输入/零项/环境失败计业务RED；原现场及其他owner文件保持。无VM/CI/commit/subagents。
