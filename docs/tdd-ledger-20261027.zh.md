# 本轮交付 TDD 台账（2026-10-27）

> 范围：当前工作区未提交的四项改动（69 个已改文件 + 6 个新增文件）。
> 状态：⬜ 未开始 · 🟨 进行中 · ✅ 完成 · ⛔ 阻塞 · ⚠️ 已知限制
> 证据只写实际执行过的命令和结果，不写「应该通过」。

## 0. 本轮基线（2026-10-27 Mac 本机实测）

| 项 | 结果 | 命令 |
|---|---|---|
| Python：系统盘扫描 + MSI 固定目录 + Defender 安装 | 23 项，通过 21，跳过 2，退出码 0 | `python3 -m unittest tests.test_no_hardcoded_system_drive tests.test_msi_fixed_install_layout tests.test_defender_installer` |
| Python 全量 `tests/test_*.py` | 190 项，通过 188，跳过 2 | `python3 -m unittest discover -s tests -p 'test_*.py'`（Mac） |
| agent 在 Mac 上可跑的部分 | 28 项，通过 26，失败 2 | `cargo test --locked --bin rustdesk-management-agent`。失败的 `service_arguments_select_edition_and_normal_path`、`trailing_separator_stays_inside_normal_install_dir` 是旧问题：用例按 Windows 路径规则判断结尾的 `\`，Mac 上不成立，本轮没有改到相关函数，以 win10-gs 结果为准 |
| win10-gs agent 测试（2026-10-27 12:48，源码与本地按 215 个文件 SHA-256 一致） | 283 项：通过 196，失败 71，忽略 16 | `cargo test --locked --bin rustdesk-management-agent --no-fail-fast`。71 项失败全部是环境原因：66 项实验室门禁（未设 `RUSTDESK_LAB_HOST`，或要求 win11 实验机、`RUSTDESK_R1_RECONCILE_TEST`），4 项缺冻结的 API 夹具（`frozen API required`），1 项缺 php；没有逻辑断言失败。旧基线为 70 项门禁失败 |
| win10-gs lib 测试 | 459 项：通过 445，失败 11，忽略 3 | `cargo test --locked -p rustdesk --lib --no-fail-fast`。9 项是 `exe_desktop_shortcut` 实验室门禁；2 项是光标测试（`test_cursor_data` 拒绝访问、`test_get_cursor_pos`），原因是计划任务以 SYSTEM 身份在无交互桌面的会话里运行 |
| win10-gs clippy | 本轮新增代码 0 处 | 整个工作区的 clippy 先在旧的构建脚本 `libs/virtual_display/dylib/build.rs`、`libs/portable/build.rs` 上失败。改为对 agent 和 lib 加 `--no-deps` 检查，共约 780 条旧诊断；按本轮未提交新增行逐行比对命中 3 条，都是旧问题：`request_restart` 在 HEAD 里就从未被调用（新抽出的 `fixed_client_exes` 跟着报未使用），`windows.rs:1671` 的多余借用原样存在于 HEAD |
| 四道提交门禁 | 本轮未执行 | `cargo fmt` / `cargo test` / `cargo clippy` / `flutter analyze && flutter test` |

注意：Python 用例只证明源码和 wxs 的静态结构，不能代替 MSI 实装和真机验收。

## 1. U：安装与 Agent 关闭 UAC

设计见 `docs/windows-disable-uac.zh.md`。目标四个键 `EnableLUA`、`ConsentPromptBehaviorAdmin`、`PromptOnSecureDesktop`、`FilterAdministratorToken` 全部写 0，不自动重启，卸载不恢复。

| # | 步骤 | 层 | 落点 | 状态 |
|---|---|---|---|---|
| U.1 | 四键目标值与 `scripts/disable-uac/u.ps1` 一致 | 单元 | `src/uac_policy.rs` `desired_values_match_disable_uac_script` | ✅ Mac 通过 |
| U.2 | 四键全为 0 才算已关闭；缺键、部分非 0 都算未关闭 | 单元 | `fully_disabled_requires_all_four_zeros` | ✅ Mac 通过 |
| U.3 | `reboot_required` 判定：`EnableLUA` 原来非 0 或缺失时为 true | 单元 | `reboot_required_when_enable_lua_was_not_zero` | ✅ Mac 通过 |
| U.4 | after-install 命令串包含四条 `reg add` 且不含重启 | 单元 | `after_install_commands_write_all_four_keys_without_reboot` | ✅ Mac 通过 |
| U.5 | 命令结果 JSON 字段稳定（before / after / reboot_required） | 单元 | `command_result_json_is_stable` | ✅ Mac 通过 |
| U.6 | status 能力清单在 Windows 上带 `system.disable_uac@1`，MSI 下能力数 = 基础 9 + UAC + 包三件 | 单元/契约 | `src/management_agent/status.rs`、`tests/management_agent_status_contract` | ✅ win10-gs：`status_snapshot_declares_stable_agent_capabilities` 通过；`installed_snapshot_declares_msi_package_capabilities` 属于 Win11 只读快照，被忽略 |
| U.7 | **缺**：MSI 侧 CA `DisableUacPolicy` 的导出、`deferred` + `Impersonate="no"`、`Before="InstallFinalize"`、纯卸载不执行、不请求重启 | 静态（Python） | 新增到 `tests/test_msi_fixed_install_layout.py` 或单独文件 | ✅ 新增 `tests/test_msi_disable_uac.py` 7 项通过；实现已在前，改用变异验证代替 RED：去掉导出、改为用户身份、卸载也执行、漏写一个键，4 种破坏都会变红 |
| U.8 | **发现缺口并修复**：原实现执行 `system.disable_uac` 时不检查本地门禁，直接写注册表。新增 `uac_policy::local_gate_refusal`，`disable-recovery` / `disable-agent` / `maintenance.json` 任一存在就拒绝，命令以 `local_gate_*` 失败落入既有审计日志 | 单元 + 接入 | `src/uac_policy.rs` `local_gates_refuse_the_remote_write`；`operations_runtime.rs` 执行分支 | ✅ 先 RED（空实现 `None` 时断言失败），后 GREEN；win10-gs 上 agent 编译通过，`local_gates_refuse_the_remote_write` 通过 |
| U.9 | 写注册表失败时命令失败并返回 `uac_policy_apply_failed:*`，不进入整机维护态 | 真机注入 | `apply_disabled` 直接调用 winreg，无注入点；为测这一条加接口会改动现有路径，改为真机验收（临时去掉键的写权限，测完恢复 ACL） | ⚠️ 并入 U.10 真机（需暂时改 ACL，win10-gs 下次执行） |
| U.10 | 真机：**重点核对 MSI 写入是否真的生效**——同一文件里的 `AddRegSoftwareSASGeneration` 有注释说 `RegSetValueExW` 总返回 998，所以 SAS 改为先用 `reg add` 写；`DisableUacPolicy` 只用 `RegSetValueExW`，没有这层兜底，失败也只记日志。静默 MSI `/qn /norestart` 装完四键为 0、不重启；手动重启后 UAC 关闭；Agent 再下发一次仍成功；卸载后四键仍为 0 | win10-gs | — | ✅ 2026-10-27 13:17 win10-gs：测试包用正式版 2026102701 的安装内容加本轮安装器逻辑打成（版本 1.5.4.29884310）。原值四键皆为 0，先备份再临时改成 1/5/1/1，静默安装 `/qn /norestart` 退出码 0（不是 3010），日志逐条写出 `set EnableLUA=0` 等四行，四键全部回到 0，`RegSetValueExW` 在此机正常、998 问题未复现；开机时间 11:36:19 不变；ID、`installation_id`、`key_pair` 哈希不变，两个服务运行。随后装回正式包，四键仍为 0（升级移除旧产品不会恢复 UAC）。Agent 下发补写依赖新 Agent 上机，未测 |
| U.11 | `release-notes.zh.md` 写明「安装默认关闭 UAC，需自行重启后完全生效；卸载不恢复」 | 文档 | — | ✅ 2026-10-28 已写入 `release-notes.zh.md` 20261028.1 节 |

## 2. D：不再写死系统盘

所有 `C:\` 路径改由 `src/platform/windows_system_dirs.rs` 向 Windows 查询（Known Folder → 环境变量 → 旧固定布局回退）。

| # | 步骤 | 层 | 落点 | 状态 |
|---|---|---|---|---|
| D.1 | 生产 Rust 代码（跳过注释和测试项）不再出现 `C:\` 字面量，只允许系统目录模块自身 | 静态（Python） | `tests/test_no_hardcoded_system_drive.py` | ✅ 本轮通过 |
| D.2 | 词法器正确跳过注释和 `#[cfg(test)]` 项 | 静态 | `test_lexer_skips_comments_and_test_items` | ✅ 本轮通过 |
| D.3 | 系统盘在 D 盘时，所有派生目录（Program Files、ProgramData、Agent、救援、Installer 缓存、LocalService/SystemProfile Roaming）都跟随 | 单元 | `every_derived_directory_follows_the_system_drive` | ✅ win10-gs agent 与 lib 各 5/5 |
| D.4 | 查询失败时回退旧固定布局 | 单元 | `an_unavailable_folder_falls_back_to_the_previous_fixed_layout` | ✅ win10-gs agent 与 lib 各 5/5 |
| D.5 | 安装包内固定标记映射到本机实际目录；其它目录不算固定布局 | 单元 | `the_packaged_fixed_install_marker_maps_to_this_machine`、`any_other_install_directory_is_not_the_fixed_layout` | ✅ win10-gs agent 与 lib 各 5/5 |
| D.6 | status 上报 `paths` 快照：系统盘、各救援目录、缺失记 null、无盘符 Windows 目录不报系统盘 | 单元 | `status.rs` 三个新用例 | ✅ win10-gs `paths_tests` 3/3 |
| D.7 | 回归：救援、本地修复、total-loss、EXE 迁移/升级、purge、普通更新回执在 C 盘机器上行为不变 | 单元（已有用例） | `cargo test --workspace` 全量 | 🟨 win10-gs agent/lib 全量中除环境门禁外没有失败；66+9 项实验室门禁用例需要破坏性真机环境才能验证，未执行 |
| D.8 | 真机：win10-gs（C 盘）装、升级、修复、救援全链不回退；无 D 盘系统盘真机，记 ⚠️ 只由单元覆盖 | win10-gs | — | ⚠️ C 盘机安装/升级已在本轮真机通过（U.10/M.7）；`system_dirs` 路径逻辑单元覆盖 5/5；无 D 盘系统盘真机，D 盘分支只由单元测试覆盖，属已知限制 |

## 3. M：MSI 安装目录锁定、Defender 默认执行

| # | 步骤 | 层 | 状态 |
|---|---|---|---|
| M.1 | 目录页没有「更改」按钮和可编辑路径，只读显示 `[INSTALLFOLDER_INNER]` | 静态 | ✅ 本轮通过 |
| M.2 | 注册表记忆值与命令行 `INSTALLFOLDER` 都被忽略，costing 前强制回到系统 Program Files | 静态 | ✅ |
| M.3 | Agent 恢复安装落到固定回退目录；升级和修复不会命中回退条件；Agent 收到同一个锁定目录 | 静态 | ✅ |
| M.4 | 界面不显示 Defender 选项；交互与静默都默认执行；命令行 `DEFENDER_EXCLUSIONS=0` 仍可关闭；在复制文件前执行 | 静态 | ✅ |
| M.5 | 快捷方式勾选框显示且默认勾选，升级保留上次选择 | 静态 | ✅ |
| M.6 | CA（`Admission.cpp`、`Bootstrap.cpp`、`bootstrap-preflight.ps1`）不写死系统盘 | 静态 | ✅ |
| M.7 | 真机：交互安装界面截图核对（无改目录、无 Defender 勾选）；命令行传 `INSTALLFOLDER=D:\x` 仍装到 Program Files；从旧自定义目录升级的行为 | win10-gs | ✅ 不截图，改读 MSI 数据库：目录页「安装到」是只读文本 `[INSTALLFOLDER_INNER]`，没有更改按钮和 Defender 勾选框，没有入口能打开 BrowseDlg；真装时传 `INSTALLFOLDER=<其他目录>` 仍装到 `C:\Program Files\RustDesk\`，其他目录未创建，`DEFENDER_EXCLUSIONS = 1`。**发现**：目录页说明文字仍是「…or click Change to choose another」，提到已删除的更改按钮，待修 |

## 4. T：桌面管理员终端 / SYSTEM 会话（连接认证 + 服务进程身份）

说明文档：`docs/desktop-terminal-admin-system.zh.md`。  
控制端提交会话标记；被控端用户名字段非空时使用服务进程令牌（已安装 = LocalSystem）；连接密码与控屏共用。

| # | 步骤 | 层 | 状态 |
|---|---|---|---|
| T.1 | 标记非空 | 单元 | ✅ Mac 通过 |
| T.2 | 控制端：管理员终端只发标记和空 OS 密码字段；`handle_hash` 经 `terminal_admin_after_hash` 只可能 `input-password` 或自动 marker 登录，永不 `terminal-admin-login*` | 单元 | ✅ `admin_terminal_sends_only_the_marker…`、`admin_terminal_after_hash_never_opens_an_os_account_dialog`、`non_admin_logins_keep…`（`uac_policy` 15/15） |
| T.2b | Flutter：旧 `terminal-admin-login` → 空 OS 字段自动 `login`；`terminal-admin-login-password` → 只弹连接密码框；新增 SYSTEM 菜单/URI | UI | ✅ 源码已改；**正式菜单展示等 Tag Build**（本机缺 Xcode，未重编 Flutter 资源） |
| T.3 | 被控端用户名字段非空 → `SelfUser`；空则原用户令牌 | 代码 | ✅ `connection.rs` |
| T.4 | 跨机真机：控制与被控均换主库后，管理员终端无 Windows 交互登录框；会话 `whoami` = `nt authority\system`；本机连本机不作连接密码结论 | 真机 | ✅ **2026-10-28**：win10-gs（控制）→ GepShare `83077683`（被控）；两边 `librustdesk.dll` sha `35cec6e31840ff6a…`；无 OS 登录框；`whoami` = SYSTEM；用户确认连接密码因已记住未再输入（符合共用记住逻辑）。详见 `desktop-terminal-admin-system.zh.md` §7 |
| T.5 | 产品边界：门禁为连接密码 + 终端权限；已安装服务上会话身份为 LocalSystem；与网页 Agent 终端通道分离 | 文档 | ✅ 已写入 `desktop-terminal-admin-system.zh.md` §6 |

## 5. 收尾门禁（全部完成前不得声称可发布）

| # | 项 | 状态 |
|---|---|---|
| G.1 | 补齐 U.7、U.8、U.9、T.2、T.3，逐项记录 RED（先失败）→ GREEN | ✅ U.7、U.8、T.2 已补；U.9、T.3 转真机 |
| G.2 | `cargo fmt --all -- --check` | 🟨 更正：先前写「改动文件没有差异」有误（`cargo fmt` 输出的路径是 `tests/../src/...`，按文件名比对漏了）。实际 `operations.rs`、`operations_runtime.rs`、`status.rs` 的格式差异数与 HEAD 完全相同（6/83/18），属旧问题，本轮没有新增 |
| G.3 | win10-gs 按哈希同步源码后执行 `cargo test`，记录用例数与退出码 | 🟨 agent 196/283、lib 445/459 通过，失败全部是环境门禁（见 §0）；`--workspace` 其余成员没有单独跑 |
| G.4 | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | 🟨 写法照原样会在旧构建脚本上失败，永远过不了；本轮新增代码 0 处诊断（见 §0）。lib 里 `uac_policy` 的 8 条未使用告警已用 `#[allow(dead_code)]` 消除 |
| G.5 | `flutter analyze && flutter test`（本轮未改 `flutter/`，仍按门禁跑） | ✅ win10-gs：`flutter analyze --no-fatal-warnings --no-fatal-infos` 0 个错误；CI 指定的 4 个测试文件 45 项全部通过。首次运行失败是环境问题：pub 缓存里 16 个 Windows 插件包和 `window_size` 的目录在 10-21 被清空，删掉空目录后重新获取即恢复 |
| G.6 | Python 全量（`tests/test_*.py`，含 `test_macos_signing.py`） | ✅ 190 项，跳过 2 |
| G.7 | 生成 MSI（standard、sos）并完成 U.10、D.8、M.7、T.4 真机项 | 🟨 standard 测试包已生成并完成 U.10、M.7；sos 包、D.8、T.4 未测 |
| G.8 | 回归面说明：列出 69 个改动文件中运行时行为变化的路径及必要性 | ✅ 见本台账 §9（2026-10-28 补） |
| G.9 | `release-notes.zh.md`（U.11）与 AOCI 对齐（`aoci_maintain` 后 `aoci check` 非 blocked） | 🟨 U.11 / `release-notes` 已写入；AOCI 对齐待 MCP 恢复后执行 |
| G.10 | code-reviewer / security-reviewer 复审（重点 T、U.8） | 🟨 code-reviewer 已完成（审查结论见 §6）；security-reviewer 未做（待排期） |


## 6. 待决事项（2026-10-27）

1. **本地文件又被别的会话改过**：13:06 有会话在本地给 `uac_policy.rs`、`operations*.rs`、`status.rs` 加了 `system.query_uac` 和心跳 UAC 字段，保留了本轮的门禁和测试。Mac 上 agent 测试 `uac_policy` 10/10 通过；但 win10-gs 上本次 agent/lib/clippy 结果对应的是加入前的版本，需要重新同步再跑一次。
2. **代码审查需要用户决定的项**：
   - 桌面管理员 / SYSTEM 终端（T）：**2026-10-28 用户确认**——连接认证（固定/一次性 + 记住逻辑与控屏相同）即可；已安装服务上会话为 LocalSystem；跨机验收通过。文档见 `desktop-terminal-admin-system.zh.md`。
   - 新客户端对旧版本设备：控制端发会话标记与空 OS 密码字段，旧被控端按真实账户校验会失败并提示升级；机群需逐步换主程序。
   - Agent 修改整机 UAC 已写入 AGENTS 与 `windows-disable-uac.zh.md`；门禁与 `local_gate_refused` 区分见 U.8。
3. **审查中已核实不触发的项**：bootstrap 路径比较不对称，只在服务参数带结尾 `\` 或大小写不同时出错；win10-gs 上服务参数与 `bootstrap.json` 都是 `C:\Program Files\RustDesk`，标准 MSI 布局不会触发。

## 7. 第二轮（2026-10-27 13:20 之后）

| # | 项 | 结果 |
|---|---|---|
| R2.1 | 合并别的会话加的 `system.query_uac` 与心跳 UAC 字段 | 叠加在本轮改动上，门禁与测试保留；同步 win10-gs 后复跑：agent 200 通过 / 71 环境失败（与上轮同一批），lib 449 通过 / 11 环境失败，`uac_policy` 两处各 13/13；clippy 本轮新增行仍只有上轮那 3 条旧问题 |
| R2.2 | 用户决定 1：管理员终端固定密码和一次性密码都可以 | 写入 `docs/windows-disable-uac.zh.md` |
| R2.3 | 用户决定 2（A 方案）：旧版被控端回「Incorrect username or password.」时退回系统账号对话框 | `uac_policy::terminal_admin_os_login_fallback` 先 RED 后 GREEN；`client.rs` 新增 `terminal_admin_os_login_fallback` 状态，`handle_login_error` 首次遇到该错误弹原对话框（未保存连接密码时弹带密码的版本），之后发用户输入的账号；手输仍错则保持旧的通用报错。真机（新控制端连旧设备）未测 |
| R2.4 | 用户决定 3：UAC 能力写进规则、门禁补「用户明确停止意图」、区分错误码 | AGENTS.md 加入 UAC 例外条款；`local_gate_refusal` 增加停止意图（`stop-service=Y` 或读不出时拒绝）；新增 `command_error_code`，门禁拒绝上报 `local_gate_refused`、写入失败 `uac_policy_apply_failed`，其他命令仍为 `service_query_failed`；均先 RED 后 GREEN，Windows 编译通过 |
| R2.5 | MSI 目录页说明文字仍提「Change」 | 新增 `MyInstallDirDlgDescription` 字符串并改引用，测试先 RED 后 GREEN；win10-gs 重打 MSI 成功，MSI 表中文字为「Click Next to install [ProductName] to the folder below.」 |
| R2.6 | 补测试环境 | 原环境未丢：`D:\build-env\php`、`D:\github\rd-api` 都在，是本轮脚本没设 `AGENT_TEST_*` 变量。设上后原来缺环境的 5 项连同同组 2 项共 7 项全部通过。环境说明已写入 AGENTS.md（含迁移后的测试目录、SYSTEM 下的缓存路径、32 位 MSBuild 重定向） |
| R2.7 | 实验室门禁用例（用户 2026-10-27 14:00 同意执行） | 14:04 备份（Program Files 105、ProgramData 707、配置 5 个文件哈希一致，日志未备份）→ 停生产 Agent 并改手动 → 看门狗 15:35 → 14:09–14:15 运行。后端用冻结副本 `D:\github\rd-api`。结果见下表 |
| R2.8 | 恢复 | 生产 Agent 改回自动并运行，失败动作未变；ID `1338209581`、`installation_id`、`key_pair` 哈希与测试前一致；未重启（开机 11:36:19）；UAC 四键仍为 0；Agent 恢复后策略检查成功、无错误；全部计划任务已删除 |

### 实验室用例结果（2026-10-27 14:09）

| 批 | 范围 | 结果 |
|---|---|---|
| A | `local_event_runtime::reconciliation_tests` | 4/4 |
| B | `signed_rust_command_roundtrip_uses_real_php_sqlite_and_journal` | 1/1 |
| C | `status::tests::delivery_contract` | 6/6 |
| D | `local_repair::r4_finalize_tests` | 19/19 |
| E | `r4_poll_tests::r4_poll_` | 9/9 |
| F | `r4_poll_tests::full_loss` | 18/20，2 项阻塞 |
| G | `local_recovery_events::tests` | 9/9 |
| H | `bootstrap::tests` | 15/15（2 项按设计忽略） |
| I | `recovery_windows::tests` | 12/12 |
| J | `local_event_runtime::tests` | 0/1，阻塞 |
| L | lib `exe_desktop_shortcut` | 17/17 |

阻塞的 3 项都是旧隔离 VM 时代的测试数据，与本轮代码无关：
- `full_loss_actual_worker_terminal_php_ack`、`full_loss_actual_pending_upload_only`：要读 `C:\RustDeskAgentLab\total-loss-worker-events`、`event-upload-only` 下旧 VM 整机丢失演练采集的证据文件，win10-gs 上没有；要补需在本机实做一次整机丢失演练。
- `native_r1_stopped_service_records_before_ensure_start_without_installation_changes`：除需 `LOCAL_EVENTS_NATIVE_R1=authorized` 外，`installation_id` 写死为旧 VM 的 `743e15c5…`，本机为 `c3c0f2de…`，必然失败；要跑需改测试。

另记：较新的后端副本（`D:\tmp\rdtest\rd-api-head`、`rd-api-wt`）下 B、C 两批会失败，原因是新后端的 `agent_binding_authorize` 调用了定义在别处的 `fail()`，而测试夹具只 require 了 `sqlite/lib.php` 和 `agent_operations.php`。属测试夹具与后端的耦合问题，不影响生产，待单独处理。
lib 另外 2 项光标测试（`test_cursor_data`、`test_get_cursor_pos`）需要交互桌面，计划任务以 SYSTEM 运行时取不到光标，未覆盖。

## 8. 集成测试目标（26 个 tests/*.rs，2026-10-27 15:xx）

下午把 26 个集成测试目标也按 win10-gs 真机跑了一遍。首轮多数因 `system_dirs` 改动后测试入口缺模块声明而编译失败；在 21 个入口（含 `modules.rs`）补 `mod system_dirs` + `management_agent`/`platform` 同名路径 shim 后，分两类：

- **已通过**：`msi_metadata`、`msi_assets`、`package_download`、`local_repair_source`、`rescue_projection`、`first_install_*`、`health_windows`、`msi_admission`、`preparation`、`native_install`、`purge_discovery`、`purge_preflight`、`purge_execution`、`purge_native_service`、`windows_installer_busy` 等。
- **`registration`、`exe_migration`、`r4_total_loss_red`**：首轮上传被截断成损坏版本，重传校验哈希一致后重编。`registration` 21/21、`exe_migration` 21/21 全通过；`r4_total_loss_red` 编译通过、95 项通过，16 项失败全是环境/夹具依赖（ACL 夹具 `agent_acl_bytes_fixture`、救援包 `fixed_source_public_fixture`、`RUSTDESK_COORDINATION_ROOT` 环境变量、VM 驱动协调），与本轮代码无关。涉及 `crate::platform::windows::system_dirs`（平台 shim）与嵌套 `mod local_repair` 里的 `AGENT`→`system_dirs` 改写。

**结论（系统盘不写死）：26 个集成测试目标的 system_dirs 编译断裂已全部修复，无一例因本轮代码逻辑失败。**

- **仍失败，属测试环境/驱动，与本轮代码无关**：
  - 只需预建基目录：`exe_assets`（`C:\RustDeskAgentLab\exe-v215`）、`exe_install` 等，`workspace::create` 会逐级校验祖先目录存在，基目录不存在即报 `workspace path unavailable`。
  - 需预置演练证据：`rescue_assets` 的 `actual_installed_layout_prepares_complete_lab_bundle…` 等，要本机实做一次安装/整机丢失演练生成可解密原件。
  - VM/guest driver 门禁：`local_events`（`only the main agent's scheduled VM driver`）、`purge_marker`（`PURGE_MARKER_RENDEZVOUS` guest driver rendezvous）。按用户要求，这类门禁要从 VM 驱动改造成 win10-gs 真机门禁，属独立一块，待排期。

> 结论：本轮新增/改动代码在集成测试层未发现逻辑缺陷；剩余失败是「VM→win10-gs 测试架构改造」与「预置演练证据」两项遗留，已登记，未纳入本轮必过门禁。

## 9. 回归面说明（G.8，2026-10-28 补）

本轮改动文件中运行时行为有实质变化的路径如下，其余改动（文档、lang、测试）不影响运行时。

| 文件 | 行为变化 | 必要性 |
|---|---|---|
| `src/uac_policy.rs`（新增） | 写四键注册表、读取当前值、校验、错误码区分、标记常量 | 核心功能，全新文件 |
| `src/local_terminal.rs`（新增） | 本机 `_terminal` IPC 协议编解码 | 桌面管理员终端通道 |
| `src/ipc/terminal.rs`（新增） | 本机终端 IPC 路径、帧结构 | 同上 |
| `src/platform/windows_system_dirs.rs`（新增） | 运行时查询系统盘路径，替代写死 `C:\` | 系统盘路径抽象 |
| `src/client.rs` | `terminal_admin_after_hash`：管理员终端路径不走 OS 账号对话框；`handle_login_error` 新增 fallback；新增 `connect_terminal`/`terminal_ipc_path`；`IS_TERMINAL_ADMIN` 环境变量 | 管理员终端免 OS 密码 |
| `src/ipc.rs` | `LocalTerminal`/`LocalTerminalResponse` Data 变体；`connect_terminal` 函数 | 本机终端 IPC 接入 |
| `src/server/connection.rs` | `fill_terminal_user_token`：用户名非空直接 `SelfUser`，删除旧 `LogonUser` 路径 | 被控端用服务进程令牌 |
| `src/server/terminal_helper.rs` | `get_default_shell`：从 `system_dirs` 查 PowerShell 路径 | 非 C 盘兼容 |
| `src/server/terminal_service.rs` | 新增 `ensure_service`/`drop_service` 公开入口 | 本机终端复用 |
| `src/management_agent/operations.rs` | 新增 `system.query_uac`/`system.disable_uac`/`system.run` 三条命令解析与分派 | Agent 远程能力 |
| `src/management_agent/install_plan.rs` | 用 `system_dirs::get()` 验证正常安装路径 | 非 C 盘兼容 |
| `src/management_agent/installer_process.rs` | 用 `system_dirs::get()` 替换多处写死路径 | 非 C 盘兼容 |
| `src/management_agent/workspace.rs` | 用 `system_dirs::get()` 替换 Agent 目录 | 非 C 盘兼容 |
| `src/management_agent/file_restore.rs` | 用 `system_dirs::get()` 替换路径 | 非 C 盘兼容 |
| `src/management_agent/helper_runtime.rs` | 用 `system_dirs::get()` 替换路径 | 非 C 盘兼容 |
| `src/management_agent/resume_task.rs` | 用 `system_dirs::get()` 替换路径 | 非 C 盘兼容 |
| `src/updater/ordinary_update_receipts/runtime.rs` | 用 `system_dirs::get()` 替换 `INSTALLED` 常量和 ProgramData 路径 | 非 C 盘兼容 |
| `src/platform/windows.rs` | `get_default_install_path` 去掉 `ProgramFiles` 环境变量覆盖，直接用 `system_dirs` 的 64 位查询结果 | 修正路径查询逻辑 |
| `res/msi/CustomActions/CustomActions.cpp` | 新增 `DisableUacPolicy` CA 函数，写四键注册表 | UAC 安装时关闭 |
| `res/msi/CustomActions/Bootstrap.cpp` | 用 `SHGetKnownFolderPath` 替换写死路径 | 非 C 盘兼容 |
| `res/msi/CustomActions/Admission.cpp` | 用 `SHGetKnownFolderPath`/PS `GetFolderPath` 替换写死路径 | 非 C 盘兼容 |
| `res/msi/Package/Components/Folders.wxs` | 安装目录改为固定 `ProgramFiles64Folder`，不再读注册表或命令行 | 目录锁定 |
| `res/msi/Package/Components/RustDesk.wxs` | 调度 `DisableUacPolicy` CA | UAC 安装时关闭 |
| `res/msi/Package/Fragments/Defender.wxs` | 移除 Defender 勾选框，`DEFENDER_EXCLUSIONS` 默认 1 | Defender 默认执行 |
| `res/msi/Package/UI/MyInstallDirDlg.wxs` | 目录页路径改为只读文本 | 目录锁定 |
| `res/msi/bootstrap-preflight.ps1` | `state.normal_install_path` 比较加旧版 `C:\Program Files\RustDesk` 兼容 | 非 C 盘老设备升级兼容 |
| `flutter/lib/desktop/pages/connection_page.dart` | 新增 `_terminalAdminApplies`，将两个终端菜单项限于 Windows peer 或未知平台 | 平台门禁 |
| `flutter/lib/common/widgets/peer_card.dart` | peer 卡片加「终端（SYSTEM）」入口 | 新菜单入口 |
| `flutter/lib/models/model.dart` | 收到旧 `terminal-admin-login*` 时自动切换登录模式 | 兼容旧被控端交互 |
| `flutter/lib/common.dart`、`flutter/lib/mobile/pages/home_page.dart` | URI 与移动端加终端 SYSTEM 入口 | 新入口 |
