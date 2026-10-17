# EXE install_me 快捷方式 adapter 接线

## 最新 P2 编码修复

仅长脚本落盘分支改为UTF-8 BOM后Base64分块解码；短EncodedCommand的UTF-16编码不变，RUSTDESK_OUTPUT_DIR路径不变。真实Win11临时桌面名`桌面-Ж-😀`强制走-File：旧实现RED为`shortcut open 3`且测试失败（本次并非错误2假成功），修复后实际.lnk COM回读新版目标正确。连同原8项回归，最终9 passed/0 failed/0 ignored/0 filtered out，17.22秒，进程exit0。

TEMP/opencode证据：shortcut-encoding-red.json、shortcut-encoding-green-before.json、shortcut-encoding-green-after.json、shortcut-unicode-green.stdout.txt。GREEN运行前后源码/测试/EXE SHA-256一致：adapter `7ff27801c4975fa8fa7955497dffa9584dc4a060f77dfb03f836b9628e127571`，cases `5ed537b1ff6b2838f8c13e3cc3f3cb11013257e69bf93e95b7755327f4c2eae1`，fixture EXE `da33cabf7183ee2c7734a14db6c8989991f56d330675c7f526a0d1f8156ddda0`。

RED唯一遗留目录6524-1792121000308921400已核文件清单、无reparse后逐文件/空目录删除；GREEN PID4880对应临时目录计数0。无产品安装、计划任务或主项目重复构建；仅宿主offline/jobs1编译fixture。此轮不新增完整产品编译、SYSTEM权限或rollback保证。

## 最新 P1 修复（优先于下方历史说明）

guard_uninstall只识别并移除完整固定RustDesk桌面del行，要求恰好一处，否则在run_cmds前失败；不替换任意用户文本。旧卸载不再移走桌面文件。最终commands以OPEN_EXISTING/OPEN_REPARSE_POINT、零share打开原文件，同一handle验证卷号/文件ID/属性/链接数及原bytes后就地写新版；原文件已被用户删除则退出成功且不重建，替换文件（即使同内容不同ID）拒绝写入。首次安装仍CreateNew。这替代了旧文档中的“旧卸载合法删除后重新创建”路径，不再需要移走文件的恢复receipt。

真实RED：preflight成功后替换文件，实际执行legacy del片段导致`legacy uninstall deleted replacement`失败。GREEN：8项全通过，含preflight之后外来替换及用户删除。长脚本超过cmd行限制时在现有受保护RUSTDESK_OUTPUT_DIR分块编码解码执行，未新增用户可指定临时路径或长期状态。依赖PowerShell Add-Type编译小型kernel32声明，其可用性仍需父进程权限环境验收。

完整standard `cargo check --offline --locked --lib --features flutter`使用VC/libclang/vcpkg环境、jobs1实际运行，多次60秒工具超时后最终工具异常，无成功退出结果；最终无cargo/rustc进程。不能标记完整编译通过。Win11两产品服务查询Running；未调用产品API或安装程序，不等于SYSTEM全量health/hash核验。失败轮两个唯一目录8076-1792119957070373900、8076-1792119963975866500已核对文件清单/reparse后精确清理。

本轮Git blob身份（不是SHA256）：windows.rs bd3b46b1c89a83e4093d0b6c62574c8ea7071f4d；adapter 2147ce523c0d3e94a33529f53b7078456cafc3f3；cases 0f3745f39c2b3390ea6ed49ba268d28b6a0327e9。

仍未验收：实际UAC/SYSTEM父子权限、产品安装回滚、就地写入过程I/O故障恢复，以及生产完整lib编译。就地write失败可能留下部分新bytes；本次修复关闭的是外来对象误删和用户删除后复活，不能扩张成原子回滚保证。

## 当前接线状态

已按授权在 windows.rs 声明模块并接入 install_me：严格 installed_target 查询、prepare、guard_uninstall，以及用commands替换原desktopicon无条件copy。update_me未改。下面的“owner应用”代码为历史接线说明，当前源码使用guard_uninstall组合前置检查。

installed_target依次查询固定64/32位安装登记，注册表读取错误或缺失InstallLocation都报错；所有登记确实不存在后还检查固定ProgramFiles目标是否遗留文件，有遗留文件拒绝当首次安装。没有registry写入，没有SYSTEM用户桌面推测。

本轮真实BAT接线RED单项失败（无preflight时执行卸载标记）；加入guard后Win11 7项通过、0失败、0忽略。其中测试BAT只执行临时目录中的变更哨兵，没有调用旧产品卸载。宿主offline/jobs1的独立adapter非test lib check通过。主项目cargo check --offline --locked --lib未取得成功：首次lock_api/autocfg找不到编译程序，补PATH后工具执行异常结束，最终查无cargo/rustc后台进程。不能把adapter编译扩大成主项目标准lib已通过。

实际父进程权限（用户/UAC/SYSTEM）、产品安装、rollback、安装状态查询故障注入、跨旧卸载并发替换窗口尚未验收。prepare对非固定目标和重定向公共桌面保守失败。此次生产回归面仅install_me桌面计划及其错误提前返回；原startmenu/tray/联合更新BAT保持。尚需独立review及完整产品编译门禁。

新增实际模块 `src/platform/windows/exe_desktop_shortcut.rs`，未修改 windows.rs。宿主 offline/jobs=1 编译；Win11 实际 Rust adapter 生成的 EncodedCommand 执行：RED 5项中2失败（原desktopicon决策），修复后6项通过。测试 target 是独立Cargo harness，仅复用项目已有windows 0.61.1，不增加主项目依赖。不是旧PowerShell原型测试。

## 具体接线（由 windows.rs owner 应用）

模块声明：`mod exe_desktop_shortcut;`（windows.rs 的相邻目录解析到 windows/exe_desktop_shortcut.rs）。

在 install_me 获得已安装信息、生成新 shortcut bytes 后，且 run_cmds/卸载执行前：

```rust
let desktop_plan = exe_desktop_shortcut::prepare(installed_exe.as_deref(), options.contains("desktopicon"), shortcut_bytes(&exe, None, shortcut_icon_location.as_deref())?).map_err(|e| anyhow!(e))?;
let uninstall_str = format!("{}\r\n{}", desktop_plan.preflight_commands().map_err(|e| anyhow!(e))?, uninstall_str);
```

这里 `installed_exe: Option<PathBuf>` 必须由 owner 的成功安装状态查询产生：有已安装EXE为Some，确认首次安装为None；读取失败传播错误，不能用exists/metadata.is_ok吞错。当前 install_me 未保留这个变量，需保留 get_install_info 的旧exe和成功查询状态，不能无条件用默认路径推断首次安装。新目标只允许OS Known Folder ProgramFiles/ProgramFilesX86下RustDesk/RustDesk.exe。

替换现有 `if options.contains("desktopicon")` 的桌面copy分支（保留后续startmenu拼接）：

```rust
let mut shortcuts = desktop_plan.commands().map_err(|e| anyhow!(e))?;
let reg_value_desktop_shortcuts = if desktop_plan.creates_shortcut() { "1" } else { "0" }.to_owned();
```

移除原桌面无条件copy分支；否则会绕过新判定。`desktop_plan` 必须活到run_cmds完成，期间持有公共桌面祖先目录无delete-share句柄，阻止目录被替换。原子创建拒绝卸载后出现的不同文件。旧卸载负责移除原文件，已有则写入新版bytes，不承诺原字段保真。首次无现存快捷方式时遵循原选项；首次同名外来文件保守失败。

EXE update_me 无需接线。范围只有旧install_me实际创建的公共桌面RustDesk.lnk，不读取/写入个人Desktop，不从SYSTEM推断登录用户。公共桌面被重定向、与旧%PUBLIC%/Desktop不同则拒绝，防止原卸载删错位置。

## 验收及限制

- prepare阶段校验COM目标、文件类型、reparse及读取失败；snapshot仅保留在plan内，无长期全局状态。
- preflight在旧卸载之前复核原bytes/absence；不是跨整个旧卸载的文件锁事务。复核与legacy删除之间仍有文件被替换的竞争窗口，owner需保持安装串行门禁，不能宣称对恶意并发替换的完整防护。
- commands使用CreateNew，不能覆盖卸载后外来文件。过长编码脚本在调用run_cmds前报错，不能无限增长突破cmd行限制。
- 已验证fixture内COM/absence及实际脚本；未执行prepare的真实公共桌面入口、实际提权旧卸载、新安装或产品权限/回滚测试。不能宣称EXE生产接线完成。
- 未使用shim/测试开关改变产品KnownFolder；fixture仅调用模块私有prepare_at，将范围限定为新建临时目录。两个失败RED用例正常清理；中间preflight失败遗留空目录rd-exe-desktop-7472-1792118443900586300，已核对为空后按确切路径非递归删除。没有创建VM计划任务或产品安装。

运行结果：最终 `6 passed; 0 failed; 0 ignored; 0 filtered out`。负向PowerShell错误输出为预期；VirtualBox转码出现CLIXML解析噪声，但Rust最终测试摘要为通过。
