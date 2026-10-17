# Windows 桌面快捷方式存在性保留

## 最新 missing 独立失败回滚通过

复用同一SHA90051585121DF95802CC2B9E3934ACD3ECCCF4DBA9AF162B360F4496B0D7CB93失败包，未build。恢复后的旧0510固定基线，经正常可观察UAC启动SYSTEM worker；仅正常删除授权公共桌面RustDesk.lnk形成missing。恢复snapshot前回执rollback-missing-final.json：exit1603、old_state5、旧ProductCode正确、before_link=false/after_link=false、changed/added空、两服务Running、phase=pass。rollback-missing.log含ShortcutLabFail return3和MainEngineThread1603，无1401/1406。两场present/missing真实失败回滚现均通过，不能沿用下方历史“missing未执行”。

最终恢复同一保护snapshot作清理：旧ProductCode恢复、原图标存在、双服务Running、msiexec0、测试目录不存在；源包未删，本lane任务随恢复清除。回滚pass证据在restore之前，不以snapshot作回滚证明。最后取证前仍见一个SYSTEM msiexec宿主进程PID2072（普通用户无法读取commandline），但worker已Wait完成且客户端/服务日志均1603；未强杀该进程，恢复是测试现场清理，不扩大为已证明该服务宿主当时完全退出。

## 当前正式 per-machine 失败回滚（present）

已完成至少一场完整产品回滚，SYSTEM真实msiexec退出1603，固定ShortcutLabFail在InstallExecute后同步失败；旧产品State5、旧ProductCode C220D796-2FB4-45F5-A921-87CF071C2E49恢复，原公共桌面图标存在，双服务Running，受检ProgramFiles/agent目录/客户端配置原hash无changed/added。回执phase=pass。日志没有1401/1406，不能将此前per-user失败套用为生产Transitive缺陷。

实验副本lab-failure.msi SHA256=90051585121DF95802CC2B9E3934ACD3ECCCF4DBA9AF162B360F4496B0D7CB93，从已封存完整CA+官方1501payload包复制，仅新增Type19/execute序列测试动作，未改产品版本、生产源码或签名链。MSI数据库Commit后初次manifest取hash遇文件锁，退出修改进程后独立取hash成功；原驱动错误保留。该包仅实验使用。

TEMP/opencode/shortcut-fullca-20261016c/rollback-present-final.json及rollback-present.log均在恢复前保全：日志ShortcutLabFail return3，MainEngineThread1603。missing失败回滚未执行，不重复计为通过。最后恢复保护snapshot作cleanup，旧ProductCode/图标恢复、双服务Running、msiexec0、测试目录不存在；任务随快照恢复移除。未手写registry、未强杀安装器。snapshot不是rollback通过依据，回滚pass依据来自恢复前回执。

## 当前正式 per-machine 实测结果（覆盖下方未执行检查点）

已完成两场真实SYSTEM安装，SID S-1-5-18，使用同一非官方lab-sos1501-full-ca.msi（SHA256 d8771b1b0304df6ffa63f28e99b6ebe08c1244cc63f60839134e5a1a9906d675），完整CA+官方SOS1501 payload。旧官方0510产品C220D796-2FB4-45F5-A921-87CF071C2E49，版本1.5.3.29854419、State5；候选88B95114-7D25-41D5-910D-FB06989AC2F0，版本1.5.3.29867639，同UpgradeCode D3DC41E3-3C37-5D52-BB47-F6673705C01B。每场前核SHA、旧身份和版本。

- present：msiexec exit0，新产品State5，公共桌面RustDesk.lnk存在，COM目标C:\Program Files\RustDesk\RustDesk.exe；双服务Running，原四份客户端配置SHA256保持。
- missing：从保护快照重新开始，验证原快捷方式目标后仅删除该.lnk，msiexec exit0，新产品State5，快捷方式未重建；日志RustDeskDesktopAllowed=0；双服务Running，原四份客户端配置SHA256保持。

证据已在恢复前复制宿主TEMP/opencode/shortcut-fullca-20261016c：present-final.json、present.log、missing-final.json、missing.log。实际SYSTEM worker回执同时含前后配置hash和服务路径，不输出密码。新agent状态目录增加service-recovery-enabled，不能称所有数据字节完全未变；未执行agent业务ready、generation/远控或在线E2E，本轮只验安装器快捷方式特性。其他桌面文件未作哈希全量比较，不宣称其独立验收。未注入安装失败，本轮没有产品rollback结论。

最终再次恢复原保护快照b14a0a4f-8097-4984-9cf7-996ac820c70d，作为测试清理（不是产品rollback）。restored-final.json：旧ProductCode恢复、原桌面图标存在、双服务Running、msiexec0、测试目录不存在；保护快照早于本lane SYSTEM probe/安装任务，因此新任务随场景恢复移除，未手写registry。旧源包未删除。最终健康为服务与安装基线核验，不扩大成全量业务健康。

## 当前控制台恢复与 SYSTEM 证明

正常显示已恢复：宿主console/Administrator会话1活动；Win11窗口1650116经IsIconic确认最小化，ShowWindowAsync(SW_RESTORE)和SetForegroundWindow均true。随后VBox screenshotpng恢复成功。客体rdlab会话活动、输入桌面Default；不能将此前OCR仅日期解释为锁屏。

旧8612/4116已消失且旧probe无回执，未重复旧RunAs。可观察后新发两次正常UAC：一次只读SIDprobe，一次注册独立SYSTEM只读SID任务。每次截图OCR都先识别Windows PowerShell/Microsoft Windows/UAC后才正常Alt+Y。管理员探针写入shortcut-elevation-probe2.txt；SYSTEM任务RustDeskShortcut-SystemProbe-20261016写入C:\Users\Public\shortcut-system-probe2.txt，实际内容S-1-5-18。未改UAC策略、未重启、未操作Win10。截图在TEMP/opencode/shortcut-uac2.png和shortcut-system-uac.png。

当前无consent/msiexec。SYSTEM探针任务是本lane新建一次性无trigger任务，目前保留供核对，不能称清理完成。正式MSI未运行，产品现场未修改，无须snapshot恢复。先前“需要人工恢复控制台”的阻塞已解除；后续应先核manifest/旧0510machine产品状态，再安排正式安装worker，不能将SID成功记为installer成功。

## 最新控制台核验（覆盖旧“UAC仍待决”状态）

已只读复查：原RunAs父8612、consent4116及其他consent/msiexec均不存在，SID探针文件不存在；不能认为提权成功，也不能再称该弹窗仍待决。本轮未重复RunAs。两服务Running。

VM仍running、headless会话、VBoxSVGA/单屏/128MB、VideoMode=1024,770,0；GuestAdditions7.2.20 RunLevel3，LoggedInUsers=1。原生screenshotpng仍E_FAIL 0x80004005/TakeScreenShotToArray。读screenshot skill后宿主截图成功，但模型图像输入不可用；Win11进程39024窗口句柄1650116，正常AppActivate返回false，按精确句柄截图成功但OCR文本为空，无法可靠观察客体内容。没有改变显示/UAC策略、没有重启、没有盲键确认。

截图保留TEMP/opencode/shortcut-win11-window.png；完整包及continuation.json摘要保持，尚未执行present/missing安装。最小人工协助是将正在运行的RustDesk-Agent-Win11正常控制台显示为可读画面；无需恢复快照或更改安全桌面。恢复可观察性后才能重新发起一次有新回执的正常提权探针。当前无需人工同意旧弹窗，因为旧consent已不在。

## 最新官方1501 payload 续接

TEMP/opencode/shortcut-fullca-20261016c/lab-sos1501-full-ca.msi已生成，official-build.stdout为exit0；使用官方SOS1501 MSI导出payload+当前冻结完整CustomActions（全部CA编译exit0），不是stub或v186。官方输入SHA256=17F8EE985719936177C7B8C3CF4078E6879882DFDE7BBD3D6F9CA37AF15AD519，ProductVersion=1.5.3.29867639，UpgradeCode=D3DC41E3-3C37-5D52-BB47-F6673705C01B。WiX反编译在Error表转换抛ArgumentNullException，但已导出File；随后按原MSI表映射并核每文件大小，payload1501.manifest.json记录文件SHA256。本地输出非官方包，app业务源码不是本次重新编译。

正常提权步骤：VBox guestcontrol start → PowerShell Start-Process -Verb RunAs启动只读SID探针。consent.exe PID4116存在，父等待PowerShell PID8612；没有探针输出文件，没有msiexec，没有发送确认键。VBox controlvm screenshotpng失败E_FAIL 0x80004005（TakeScreenShotToArray），无法可靠观察预期UAC内容，未盲点/未绕过。当前UAC仍待处理，禁止重复发RunAs或安装。两产品服务只读Running。missing/present生产安装尚未执行，原产品未修改，无须恢复snapshot；完整SYSTEM身份/health门禁仍未关闭。

continuation.json封存本地包/完整DLL及build/package/payloadmanifest摘要；下一owner先核等待进程/弹窗，正常可观察确认或取消后再核官方0510 machine产品状态和版本，不能把包构建成功当实际安装成功。

## 正式 per-machine 续做检查点

完整CustomActions已在TEMP/opencode/shortcut-fullca-20261016c离线Rebuild成功exit0，包含全部现有CA而非desktop stub。当前冻结WXS+历史SOS v186完整payload已生成historical-sos-full-ca.msi，retry.stdout为package exit0，build.json/package.json保留来源摘要。a为命令引号失败，b为缺离线dutil库链接失败；c补齐已有缓存成功。打包超时重试曾重复插入生成组件导致WIX0091，驱动已在生成前恢复当前RustDesk.wxs后成功。生产WXS未为此修改。

未安装该包：历史payload尚未证明与旧0510版本身份适配，不是当前客户端构建。Win11保护快照b14a0a4f-8097-4984-9cf7-996ac820c70d存在；只读核到原RustDesk.lnk、双服务Running，无msiexec，HKLM安装ProductCode为C220D796-2FB4-45F5-A921-87CF071C2E49。当前guestcontrol令牌Medium、Administrators deny-only，既有SYSTEM driver查询未取得可用任务；SYSTEM安装/health尚未建立。本轮没有改VM产品桌面、安装、恢复快照或删除原源包。旧0510→候选missing/present两场仍未执行，不算per-machine验收通过。

## 当前状态（2026-10-16，清理收尾）

- 验证统计：原真实 per-user MSI 候选包矩阵 **9 个断言通过**；随后复用同一候选包的**连续两次 repair 均不重建已删除图标**，两次保持同一 ProductCode=Installed，测试哨兵摘要不变。这是两次增量操作证据，不重复计为新的完整矩阵；本次仅清理和整理文档，没有运行新测试或修改生产源码，原 GREEN sourcehash 保持下方记录。
- 注册清理已确认：两个 fixture ProductCode 均为 -1，RelatedProducts 为空，fixture HKCU 测试键不存在，无 msiexec；事件1034删除状态0、11724删除成功。原卸载子进程 exit 未持久化，事件和现态不能冒充原 exit 收据。`shortcut-cleanup-final.json` SHA-256：DF7012228A53B812604BC64954FC8D84A57D530D9327B0757B64469D671F9681。
- 文件清理已完成：37字节 payload 来自 build_shortcut_msi_fixture.py 的惰性 payload.txt/File 行；OtherProduct.lnk 来自 shortcut_msi_repeat_repair.ps1:31，是本 lane 创建的测试哨兵，不是用户真实快捷方式。逐项核对上述清理收据中的路径、SHA-256，并拒绝 reparse 路径后，仅删除这两个文件，再非递归删除空目录。`LocalAppData\RDShortcutFixture-7BC91` 已不存在，两个 ProductCode 仍为 -1。未重复卸载，未手删 MSI registry。
- 最新文件清理收据：`C:\Users\Administrator\AppData\Local\Temp\opencode\shortcut-file-cleanup-final.json`，SHA-256 **45EF3473A0E96173C575690594A4217332DA6E964E07A442623160A1D2F38A6F**。前后普通用户可读的107个产品文件摘要及两服务状态保持。日志、测试包和收据作为证据保留，不属于待清理安装目录。
- 边界：完整 SYSTEM health、身份/来源、计划任务元数据读取未成功验证，不能称全系统 all clean。此前1603之后图标存在性保持，但产品注册曾回滚成 Advertised(1)，该失败仍保留。1401/1406伴随错误5及1307，位于 fixture MSI产品/UpgradeCode注册恢复，现证据偏向 per-user权限/owner上下文；未证明具体ACL根因，更未认定或修复成生产Transitive bug。

## 连续 repair 增量（2026-10-16）

复用上一轮候选 MSI，无生产源码变化。连续两次 `/famus {D38BA039-2E82-4915-A8B1-B616520CCAB1}`：图标均未重建，两次 ProductState=5，同目录 OtherProduct.lnk 摘要均保持。随后失败升级返回 1603，缺失图标仍缺失，但新增旧 ProductState=5 断言失败；事后查询旧产品为 Advertised(1)，日志有 registry rollback 1401/1406 错误。因此只确认快捷方式存在性回滚，不确认完整产品注册回滚。保留失败，不改生产逻辑掩盖。

当时保护快照可读产品文件摘要及服务状态保持；普通用户看不到的任务不能作为完整 SYSTEM 任务验收。当时正常卸载调用未取得最终退出和注册不存在收据；后续已取得注册清理及文件精确清理收据，当前结果见顶部。未修改任何 RustDesk 产品对象。

证据：TEMP/opencode/shortcut-repeat.json、shortcut-repeat-rollback.log。生产 WXS 表检查包位于 shortcut-production-tables-20261016a，tables.json 已核非空。standard/SOS 桌面相关表相同：ALLUSERS=1，DesktopFolder，目标 [!App.exe]，组件属性324=256+64+4（64位+Transitive+registry KeyPath，无Permanent16）；KeyPath HKCU Software\RustDesk/App.Desktop.Shortcut。FindRelatedProducts25 → InspectDesktopShortcut997 → CostFinalize1000 → RemoveExistingProducts1501。CommonDesktop 在 per-machine 上由 MSI 标准目录语义和 CA PublicDesktop Known Folder 绑定，不是表内硬编码路径。

检查包复制当前生产 WXS，没有测试路径替换，但使用历史SOS v186 payload，两种 edition 都不是新编译客户端；仅包含专属 desktop CA DLL，其他 CA 导出未供应。仅用于离线表检查，禁止安装/发布。limitations.json 标注限制及源码SHA-256。

EXE 给主 agent 的两处接线位置仍明确：第一处在 install_me 生成的 `{uninstall_str}` 执行前、提权互斥内捕获已安装状态和公共桌面存在性；第二处在原 desktopicon copy 位置按首次安装/升级原存在判定写新版 shortcut。当前没有可调用的安全 Rust adapter，不能提供两行虚构函数调用或接入原型 PowerShell。必须由该 owner 实现固定 Known Folder 的命令生成/受保护执行接口后再落实；单靠现有返回值无法安全完成两行补丁。

需求契约：升级前缺失就不重建；已有则允许用新版快捷方式覆盖。首次安装保留原 desktopicon/DESKTOPSHORTCUTS 选择。不承诺保存任意旧参数、描述、图标或其他字段。PowerShell 原型的字节保留测试仅为原型历史证据，不属于产品保证，也未接入产品。

## 生产改动

`DesktopShortcut.cpp/.h` 只读取 PublicDesktop Known Folder 下的 RustDesk.lnk，并验证固定 ProgramFiles RustDesk.exe target；无公共属性路径输入。每次重置私有判定属性。缺失禁建，未知/外来目标在旧卸载前失败，避免旧包删除无法确认归属的文件。

专属 Fragment 在 execute sequence 的 CostFinalize 前运行。RustDesk.wxs 原桌面组件条件增加私有允许标志，并设置 Transitive=yes，使已有 registry KeyPath 的 repair 也重新评价条件。条件关闭时 MSI 可以卸载该组件；因此必须同时验证已有图标的条件保持 true。首次安装仍由原 DESKTOPSHORTCUTS 决策。

回归面：仅 MSI 桌面组件重新评价条件；新增只读 CA、其导出/编译清单。windows.rs、签名、workflow、版本、macOS/Android 没有本轮修改。

## 真实 MSI RED→GREEN

宿主仅本地顺序构建。Win11 使用独立 per-user 产品 RDShortcutFixture-7BC91：UpgradeCode 977C2314-A294-4ED1-B422-29733B5F198F，ProductCode D38BA039-2E82-4915-A8B1-B616520CCAB1 / CAB2。仅本测试产品安装/卸载；未安装 RustDesk 产品、修改其桌面或恢复 VM snapshot。

测试冻结副本仅替换 Known Folder 绑定为当前用户 LocalAppData\RDShortcutFixture-7BC91\Desktop / AppRoot；生产没有测试开关或路径注入入口。复制生产 CA、组件条件及 Transitive 属性、专属 Fragment，真实 WiX 4.0.5 构建 MSI，Windows Installer 实际执行组件、major upgrade 与 rollback。

- RED：删除 .lnk、保留 registry KeyPath，`/famus` 后图标复活，`repair-missing-not-recreated` 失败。
- GREEN：生产仅增加 Transitive=yes，重建同一矩阵的一对候选包；9 个断言全通过，包括首次默认、KeyPath 保留、缺失 repair、缺失 upgrade、已有 repair、已有 rollback、缺失 rollback、首次取消选择、已有 upgrade。两次 rollback 均由 InstallExecute 后错误 CA 注入，要求退出 1603。
- 读取实际 Component 表验证 Transitive bit 64；两份实际升级日志断言 WIX_UPGRADE_DETECTED 设置 → InspectDesktopShortcut → CostFinalize → RemoveExistingProducts；全部通过。
- 最后正常卸载测试产品。红测停止后的测试旧产品在绿测前单独卸载；没有手删 MSI 注册状态。

证据根：`C:\Users\Administrator\AppData\Local\Temp\opencode\shortcut-msi-red-20261016b` 和 `shortcut-msi-green-20261016a`，各含 MSI、冻结 CA、sourcehash.json、receipt.json。绿测升级日志另复制在绿测目录；其余日志保留 Win11 `C:\Users\rdlab`。构建首次隔离修正前的 a 目录从未安装。

GREEN 生产源码 SHA-256：

- DesktopShortcut.cpp：c0f9db998d9e67222aa67c09f276585d1e8a5f561dd98de60b073f1cb1e07cdc
- DesktopShortcut.h：2fb31bff15b07ff7306bccaf276549b10d419634704a676c8042127ce8e6a896
- DesktopShortcut.wxs：c2732a6c446c3cb8672dfc03fec59adc0f735c1109c2ae0738de40b39b51301f
- RustDesk.wxs：0779f623b41af263433ba826790ad98f63212f3b7a40afb9f8cd29f4435bdd72

复现入口：build_shortcut_msi_fixture.py、run_shortcut_msi_fixture.ps1、check_shortcut_msi_tables.ps1。fixture 有独立目录保护和 Win11 限制。测试包使用保留目录标识 DesktopFolder，WiX 发 WIX5437 警告；冻结 CA 在首次安装与升级都强制映射到测试目录。

## 剩余边界

上述证据是实际 MSI 引擎的 per-user 隔离包，不等于正式 per-machine/SYSTEM RustDesk 安装包验收；正式整包、交互 UI、不同账号上下文、旧发布包升级及生产常驻保护仍待分配。历史不同 pathname 不扫描或迁移；自定义安装 target 会保守失败。

EXE update_me 当前不创建桌面图标。install_me 会先执行卸载再无条件复制：主 agent 须协调该 owner，在 `{uninstall_str}` 执行前采样，并在 desktopicon 分支区分首次安装与升级。不能仅删 options 中 desktopicon，否则旧卸载仍会删除原有图标。当前无已验证 Rust adapter，不能把原型恢复函数直接接入提权路径。
