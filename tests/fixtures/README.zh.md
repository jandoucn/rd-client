# 独立测试布局与编译记录

2026-10-17：host 仅编译和静态分析，测试执行只允许在对应独占 VM。没有运行测试、提交、触发 CI 或派生子代理。

## 入口

- peer：`peer_online/prepare.py <仓库外不存在的目录>` 复制真实 lib/assets/windows、现有 DLL 和两个 Dart 入口；仅副本注入 integration_test/ffigen 9.0.1。正式 pubspec/lock 不变，不运行 codegen。详细 VM 协议见 `peer_online/README.zh.md`。
- 快捷方式：`cargo test --offline --manifest-path tests/exe_desktop_adapter/Cargo.toml --tests --no-run`。9 项位于 `tests/exe_desktop_adapter/cases.rs`，仍由生产模块的 cfg(test) path 引入。
- publication：`cargo test --offline --manifest-path tests/fixtures/ordinary_publication/Cargo.toml --tests --no-run`。build.rs 只为独立 package 设置已有 t02_receipts；两个父测试及 ignored child 保留。VM 命令见 `tests/ordinary_receipt_publication_barrier.zh.md`。
- worker：独立 package 和必需提取步骤见 `tests/management_agent_first_install_worker_delivery.zh.md`。缺 T01_WORKER_EXTRACT 只影响这个显式入口，不影响根默认发现。
- legacy：`tests/management_agent_status_contract/legacy_unknown_fields.rs` 保留 4 项，移除固定 COMPUTERNAME 断言，文件级 cfg(windows) 防止 Linux 执行 Windows 路径。VM 运行约束由调度执行，不用主机名跳过测试。

## 本轮证据与限制

### 后续完成验证（以本节为最终状态）

- worker：`cargo test --offline --manifest-path tests/fixtures/worker_delivery/Cargo.toml --tests --no-run` 最终 exit 0，用时 3m31s，生成 `management_agent_first_install_worker_delivery-73c645b268810739.exe`。
- 根：`cargo test --offline --locked --tests --no-run` 最终 exit 0，用时 5m29s，日志列出 63 个测试可执行目标。全程 jobs=1；执行前及串行续编时核对构建进程。
- 初次根自然失败为 kcp-sys bindgen 缺 stddef.h（exit 101）；在验证子进程中补齐已有 MSVC/Windows SDK INCLUDE/LIB/PATH 后解决。后续根真实报错为 ordinary_update_terminal_contract 默认引入 runtime，缺 hbbs_http/EDITION 等 19 个符号（exit 101）。最小修复只给三处 persistence/module cfg 增加 t02_receipts，并在 ordinary_publication 独立清单注册完整 terminal target；独立 harness 两个 EXE 编译 exit 0（8.43s），没有删用例。
- 正式 `flutter/` 执行 `flutter analyze --no-pub --no-fatal-warnings --no-fatal-infos` exit 0，与 flutter-tag.yml 的警告策略一致；247 条 warning/info，无 error。另 `flutter analyze --no-pub test` exit 0、No issues found，覆盖新 Android/Ab/peer 测试解析，没有缺包/mock/生成文件错误。使用现有解析缓存，未在正式目录 pub get 或修改依赖。
- Linux 默认入口静态核对：新增 first-install/rescue、legacy 均保留 Windows cfg；package-name 默认仅纯 helper；特殊 t02 projection 不作为默认覆盖证明。本机只有 Windows rustlib，未做 Linux 编译或运行。
- 最终日志位于 TEMP/opencode：`layout-complete-worker.log`、`layout-complete-root-final.log`（19 个错误的 RED）、`layout-complete-terminal-fixed.log`、`layout-complete-root-fixed.log`、`layout-complete-results.json`。worker 原始成功日志独立保留；最后 results 为 terminal-fixed=0、root-fixed=0。

下方超时/未完成描述为前次记录，已由本节真实最终 exit 更新。没有执行任何测试二进制、客户端、VM、CI 或提交；生产逻辑没有本次增量修改。

完整读取了 git diff（1419 行含警告）及相关未跟踪测试源码。修复前 rustc --test --emit=metadata 直接编译根快捷方式 cases，复现 E0433 “too many leading super keywords”；其他缺父模块符号和未传外部依赖的错误不冒充完整 Cargo CI 回执。根 --tests --no-run 两次初始调用超时没有错误回执，不能声称捕获了 worker/publication 的修复前真实 Cargo 错误。

修复后 Windows 快捷方式 adapter no-run exit 0；publication no-run exit 0（存在生产模块未使用符号警告）。两个 target 均生成 EXE，没有运行。根 `cargo test --offline --locked --tests --no-run` 在 45 秒预算内仍编译依赖，未到测试 crate，未完成全根编译。

worker 独立 Cargo 首次离线解析因 confy 缓存缺 origin/HEAD 失败；准备脚本复制根锁后越过解析阶段，45 秒内依赖重编译未完成。尝试直接复用根 rlib 的 rustc metadata 检查遇到 E0463 缓存依赖不匹配，不能称 worker 编译通过。独立清单补齐 package_gate 直接依赖并保留根 tungstenite 固定补丁。worker 最终完整编译尚待完成，不以零测试或空 cfg 替代。

peer 新副本实际 pub get --offline 成功，Flutter 3.24.5 `analyze --no-pub integration_test test_driver` exit 0，No issues found；仅移除一个未使用 material import。最终源码副本为 TEMP/opencode/peer-layout-final-20261017，带 source-manifest.json。未做 Flutter 全包分析、VM 运行、Linux/macOS 编译或发布构建。

回归范围仅测试发现、测试依赖解析、测试 path、复制/打包入口及 legacy 测试主机名断言。唯一 src 文件改动是 exe_desktop_shortcut.rs 的 cfg(test) path 一行；生产运行路径没有本轮逻辑变化。AOCI 工具和项目 .mcp.json 不可用，未维护托管索引。

## 精确暂存候选

2026-10-17 格式增量：仅应用 `host1701-current/new-files-format.patch` 的 13 个已校验候选。原始字节均匹配 format-summary.json 的 source_sha256；最终字节均匹配 candidate_sha256。使用 Rust 1.90 rustfmt、edition=2021、skip_children=true，13 项 check exit 0；原始源码的 rustfmt stdout 与当前源码一致。未格式化其余共享文件或 hbb_common 基线，未改测试中的 1501 常量，未运行测试/VM/CI/提交。

以下 13 路径是本次格式增量的完整暂存候选，与下方布局清单取并集并去重；保留迁移后的 `tests/exe_desktop_adapter/cases.rs`，不得恢复旧根 cases 路径。每行末尾是当前 SHA-256：

```text
src/platform/windows/exe_desktop_shortcut.rs 221215b5cd905aa939aa56e171163bfcc2ebb5767bddcf088ae2881f5f040e45
src/updater/ordinary_update_receipts.rs 0fa2230031ab62afd4861dbf0949df3a6011f590332d39f6e377dcc3d7ab5138
src/updater/ordinary_update_receipts/admission.rs 26df1299a6693b74b1333032cd37a79cb4fb74eab290ac2da634cecf52f7e2b2
src/updater/ordinary_update_receipts/package_contract.rs 19a61da48987379597b6e63d4b9922460a1fe1948905c8caacaf1f6f02cb439c
src/updater/ordinary_update_receipts/package_name.rs 8980c5a156d423d6d248a45e36fc4c212ee08aac0cfcbe9dbbb50b128992c47e
src/updater/ordinary_update_receipts/runtime.rs 63256910ef7c8ee613d76b6a2ebe9e1fc9d6369e755b36e6de3233c4ca96c647
tests/exe_desktop_adapter/cases.rs 7fae1278902aa799d56802b4f5bbe6833469edaf778134ad77c51d40350437b1
tests/management_agent_first_install_approval_delivery.rs a0a5d3544ddec796b21b1e6a81a35f76997889bf053afc3993bbb02d7afb7871
tests/management_agent_first_install_delivery_contract.rs 9464c13f3fc2fa49a0539cf341f49ee4ac1154113bf5fd87e8a96ac931edd903
tests/management_agent_status_contract/legacy_unknown_fields.rs ad174b52683e15e241504db4980ca4f03f0f63414b0a86fb3037bbdd210528d3
tests/ordinary_msi_schema_route.rs f504f6db072167fedde82073a12b32a635da1525f937061a243df9204f2ae4a8
tests/ordinary_update_package_name_contract.rs 51d27f96395b3de2a92287442b96c26309cc2e6bab4a7f05eef8b0d2771835e7
tests/ordinary_update_terminal_contract.rs a7cf0c95a9200dcce4729ae65301acef0fce58f8791b7055965b3b90258265dc
```

这些文件原本未跟踪，整文件暂存包含原作者实现；格式检查只证明本次增量等于批准的 rustfmt 输出。`git diff --ignore-all-space --no-index` 对各文件均返回 1，因重排换行、可选尾逗号及 import/module 排序，不声明其为零差异。逐文件回执位于 TEMP/opencode/host1701-current/applied-format-verification.json。

以下相对路径按文件暂存；不要使用 `git add .` 或目录递归暂存：

```text
tests/fixtures/.gitignore
tests/fixtures/README.zh.md
tests/fixtures/peer_online/README.zh.md
tests/fixtures/peer_online/prepare.py
tests/fixtures/peer_online/integration_test/peer_online_session_test.dart
tests/fixtures/peer_online/test_driver/integration_test.dart
tests/fixtures/worker_delivery/Cargo.toml
tests/fixtures/worker_delivery/prepare.py
tests/fixtures/worker_delivery/cases.rs
tests/fixtures/ordinary_publication/Cargo.toml
tests/fixtures/ordinary_publication/build.rs
tests/fixtures/ordinary_publication/cases.rs
tests/exe_desktop_adapter/cases.rs
tests/shortcut_encoding_evidence.py
tests/peer_online_bundle.py
tests/peer_online_current_handoff.zh.md
tests/management_agent_first_install_worker_delivery.zh.md
tests/ordinary_receipt_publication_barrier.zh.md
tests/management_agent_status_contract/legacy_unknown_fields.rs
tests/ordinary_update_terminal_contract.rs
```

逐项命令为 `git add -- <上述精确文件路径>`，执行前由主控复核未跟踪文件的整文件内容。原路径本来未跟踪，Move 后无 tracked deletion 需要暂存。

另需合并一行 `src/platform/windows/exe_desktop_shortcut.rs` 的测试 path 变更。该生产文件原本未跟踪，不可用整文件 `git add` 声称只暂存布局补丁；应由拥有完整生产候选的主控合并这一行。现有 adapter/Cargo.toml 原本也未跟踪，其所有权由主控决定；本轮未改。
