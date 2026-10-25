# RustDesk Guide

## 子代理并行协作

- 用户长期授权主代理根据任务需要，自主安排最多 **15 个子代理同时工作**，无需每次另行询问或等待用户重复授权。
- 客户端与 API 后端协同任务共用这 15 个并发名额，不按仓库分别叠加；子代理不得自行再派生子代理扩大并发。
- AOCI 对齐、索引生成与条目作者化任务最多同时使用 **2 个子代理**，不得按文件分组持续派生起草代理；负责 AOCI 的子代理一律不得再派生子代理或启动 Workflow，主代理派发时须在指令中写明此限制并在派发后核对代理列表。
- 对可独立的需求梳理、实现、TDD、E2E、构建检查和代码审查主动并行，数量按任务依赖与机器资源合理分配，不要求凑满。
- 分派前明确文件写入范围、测试资源与验收责任，避免多个代理同时修改同一文件；Windows 真机测试机 `win10-gs` 同一时间只允许一个执行负责人，安装、升级、重启、备份恢复和故障注入必须串行调度。
- 主代理负责汇总、复审与收尾；中断或失败的调用不算完成，恢复前先核对源码、进程和测试回执，避免重复执行有副作用的操作。并行授权不改变既有提交、发布、生产操作和测试约束。

## Git 主分支约束

- 永远不创建或切换功能分支。
- 所有提交直接在默认主分支 `master` 上完成。
- 允许推送时只推送 `origin/master`；不得推送其他分支。
- 版本发布可以正常创建版本 tag；Release action 和 ACR workflow 按项目配置通过 tag 触发。

## 定制仓库

这个仓库是私人定制客户端。以后按下面做，不要再改回自动构建，也不要写成英文说明。

* 安装包只由手动触发的 **Flutter Tag Build** 产出。不要给提交、定时任务或打版本号 tag 加自动构建。
* 发版 tag 的构建号规则固定为：
  * 日期段取**当天北京时间**，形如 `20261025`；北京时间以 UTC+8 计算，跨过北京时间零点即算新的一天，不得沿用 `version.json` 里上一次的日期。
  * 序号为两位，同一自然日内从 `01` 起递增，得到 `build_seq = 日期 * 100 + 序号`，例如 `2026102501`。
  * **失败的构建同样占用序号**：每次打 tag 前先提交一次「构建号推进到 2026xxxxNN」，即使本次构建失败，下一个序号也不会复用前一个。
  * **没有明确说要打新版本 tag 时，版本号沿用上一版**（`version.json` 的 `version` 不动），只有日期和序号推进。
* 推送到 GitHub 的 commit 说明用中文。类型前缀可以留英文，主题和正文用中文，例如 `fix: 隐藏被控端连接窗口`。
* Release 标题只使用对应 tag 名称；Release 正文和 tag 说明必须保持为空。版本变化只写在仓库内的 `release-notes.zh.md`，不得自动或手动灌入 GitHub Release。
* 不要在提交说明里写英文句子。代码、路径、命令和 `RustDesk`、`SOS` 这类名称保持原样。
* 不要改这些固定配置：固定密码 `asd123asd`，ID 服务器 `rd.yan.life`，API `https://rdapi.yan.life`，公钥 `WAVL+YUYZ6EXOqLCGZEq56VD2LalZc121gtVeXxpp78=`。standard 和 SOS 都同时启用固定密码和一次性密码，并默认打开全部远程权限。
* Management Agent 属于本私人项目的远程维护通道。客户端默认声明并启用已实现的全部 agent 运维能力，包含状态上报、固定 RustDesk 服务检查/启动/重启/修复、恢复协调、固定对象校验、脱敏诊断和固定 API/下载/ID 网络探测；后续已实现的批准包修复、主程序升级和 agent 升级能力也默认纳入能力清单。API 绑定默认授予全部 agent 运维权限，不默认降级为仅观测；权限变更、实例绑定、命令、事务和事件必须保留审计。任何能力仍只能作用于固定 RustDesk 对象、批准且签名的发布包和固定域名，禁止任意脚本、任意路径、任意端口扫描、任意进程或任意环境变量执行。本地 `disable-recovery`、`disable-agent`、维护状态、用户明确停止意图、安装互斥、签名/哈希/身份校验和回滚门禁优先于远程命令，远程权限不得绕过这些本地约束。
* OSS 发布采用分离链路：GitHub Actions 上传必须使用 `oss-accelerate.aliyuncs.com`；客户端 `DOWNLOAD_BASE` 固定为 `https://download.yan.life`，其 CNAME 指向 `rustdesk-release.oss-cn-shanghai.aliyuncs.com`，不要因上传加速而改动客户端下载域名。
* Android 发布必须始终使用固定长期签名证书。证书 SHA-256 指纹为 `71:11:D7:30:EC:A2:C7:98:47:41:53:04:59:DD:38:97:20:16:2F:7B:FE:B7:39:FA:96:8F:2E:D7:4E:6F:0B:D8`，GitHub Actions 使用 `ANDROID_SIGNING_KEY`、`ANDROID_ALIAS`、`ANDROID_KEY_STORE_PASSWORD`、`ANDROID_KEY_PASSWORD`。缺少任一签名配置时必须终止 Android 发布，禁止把 debug 或 unsigned APK 上传到 Release 或 OSS。不得更换证书；确需更换时必须先得到明确确认并制定已安装客户端迁移方案。
* Android 永久只发布 standard APK，不存在 Android SOS 发布目标。Android 必须安装后使用；禁止构建、上传、写入 Release/OSS manifest 或恢复 `android-aarch64-apk-sos`。Windows 和 macOS 的 SOS 发布保持不变。
* macOS 包签名必须保持成功构建 run `36898758124` 和 `36859858472` 使用的既有方法，不得借构建修复重构或替换签名链：
  * 无证书构建依次执行 `codesign --force --sign - "$app/Contents/MacOS/service"`、`codesign --force --sign - "$app"`、`codesign --verify --deep --strict --verbose=2 "$app"`，不得启用 hardened runtime；
  * 正式证书构建继续使用 `.github/scripts/sign-macos-app.sh` 原有 `sign_args`，仅真实 identity 添加 `--options runtime --timestamp`，保持现有 entitlements、DMG 签名和公证流程；
  * 禁止增加 `--requirements`、designated requirement 校验、`PlistBuddy` 依赖或其他会改变现有应用签名身份和可运行性的参数；
  * 修改 macOS 签名相关 workflow 或脚本时，必须通过 `tests/test_macos_signing.py` 回归测试，并与上述成功 run 对应提交的签名命令逐项核对。

## 构建修复约束

构建失败时只修改导致当前构建错误的最小代码范围。不得借修复构建错误改变既有功能、协议、字段语义、固定密码、服务器地址、公钥、认证配置或默认权限；修复后必须进行代码审查并重新构建验证。

## Windows 本地编译环境（win10-gs）

本地编译/真机验收只用 `win10-gs`（termark `01a15c96-d93e-7ecc-ab71-6d5f1d90323d`，10.0.0.100）。仓库在 `D:\github\rd-client`（remote `jandoucn/rd-client.git`）。工具链已从 C 盘 `%TEMP%\opencode`（temp 会被清理）迁移到 **`D:\build-env`**，机器级环境变量已设。以下配置是实测编译通过（530+ crate，`cargo build --locked --bin rustdesk-management-agent` 退出码 0）的版本，不要再改回去：

* **Rust 工具链**：`D:\build-env\rustup\toolchains\1.90.0-x86_64-pc-windows-msvc\bin`（rustc/cargo 1.90.0），`RUSTUP_HOME=D:\build-env\rustup`，`CARGO_HOME=D:\build-env\cargo`。
* **关键坑：不要用 `D:\build-env\cargo\bin` 里那套 rustup 代理 shim，也不要把它在 PATH 中排在前面。** 复制过来的 rustup 代理会自触发「重装 rust-src 组件」→ 缓存校验失败 → `rustc` 被回滚 → 报 `failed to extract package: premature eof` / `could not compile proc-macro2`；ghost 情况下代理还会挑到 gnu 工具链报 `dlltool.exe: program not found`。正确做法是显式指定 `RUSTC=<toolchain>\bin\rustc.exe`、`RUSTUP_TOOLCHAIN=1.90.0-x86_64-pc-windows-msvc`，并把 toolchain `bin` 放 PATH 最前、**不放** `cargo\bin` proxies。
* **MSVC**：`C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat`（保留在 C 盘，不要迁）。编译前必须先 `call` 它，否则链接器/spawn 失败。
* **vcpkg 依赖**：`VCPKG_ROOT=D:\build-env\vcpkg-agent`（已装好 `x64-windows-static` 全套 deps：aom/avcodec/avformat/avutil/jpeg/libmfx/opus/turbojpeg/vpx/yuv，无需重装）。
* **libclang**：`LIBCLANG_PATH=D:\build-env\libclang-agent\clang\native`。
* **Flutter**：`D:\build-env\flutter-3.24.5\bin\flutter.bat`（`FLUTTER_ROOT`）。GitHub CI 用 3.24.5 且需 `flutter_3.24.4_dropdown_menu_enableFilter.diff` 补丁与 rustdesk 自定义 engine，本地 UI 打包需照 `.github/workflows/flutter-build.yml` 的 windows job 处理。
* **清理 target 时**：`target\debug` 若被上一次失败构建污染，必须彻底删干净（长路径用 `robocopy <空目录> <target\debug> /MIR` 再 `Remove-Item`），且在构建进程全部退出（`taskkill /f /im rustc.exe`、`cargo.exe`）后再删；脏的 proc-macro2 产物会导致 `syn` 报 `proc_macro2::extra::DelimSpan` 未解析。
* **长任务**：SSH 非交互，脱离进程用 `schtasks /run` 跑 `.bat`/`.ps1`（普通进程写入日志会因 SSH 会话结束而中断）。中文输出经 SSH 会乱码，用 base64 传递输出。
* **代码同步（重要）**：改动在 Mac 上完成并提交中文说明 → 推 `origin/master`，但 **win10-gs 上 `git pull` 不可用**：SSH 是非交互会话，Git 凭据管理器（wincredman）无法持久化，拉取会报 `could not read Username for 'https://github.com'`。正确做法是 **用 `termark upload` 把改动的源文件逐个覆盖到 `D:\github\rd-client` 对应路径，然后直接编译**。上传后 win10-gs 工作树相对其 HEAD 会显示为已修改，这是预期状态，不要试图在远端修复。推 master 不触发构建，只有打 tag 才触发 Flutter Tag Build。

## 测试与验收约束

所有功能开发、缺陷修复、协议调整、版本/更新逻辑和 UI 行为变更都必须按测试驱动开发（TDD）执行：

1. 先把需求拆成可验证的行为和失败条件；
2. 先新增或修改测试，使测试先失败；
3. 再实现最小代码使测试通过；
4. 最后重构和补齐边界测试，并保留回归测试。

测试按以下层级执行，低层测试不能替代高层验收：

* **单元测试**：纯函数、状态机、版本比较、序列化、哈希/签名校验、策略判断等逻辑必须使用 Rust `#[test]`/`#[tokio::test]` 或 Flutter `flutter_test` 覆盖正常、边界和失败路径。
* **集成测试**：跨模块、HTTP 请求/响应、配置读写、下载回退、安装事务、事件上报和 Rust/Flutter 桥接必须使用本地 fixture、mock 服务或测试服务器验证，禁止依赖线上服务才能通过。
* **Windows 真机测试统一在 win10-gs**：原隔离 VM（`RDLAB-WIN10`/`RDLAB-WIN11` 等）已全部停用。凡文档、测试、脚本或手册中写明在 VM、隔离 VM、guest 上执行的安装、升级、故障注入、E2E、静默验证和 Defender 真实写入，一律改在 win10-gs 执行（用户 2026-10-23 授权在 win10-gs 开启 Defender 并做实机测试；执行前导出 Defender 偏好与策略，测完按记录恢复，不改防篡改；开不起来时如实报告，不得以 mock 代替真实项）；破坏性测试以 `RUSTDESK_LAB_HOST=<win10-gs 的 COMPUTERNAME>` 显式声明测试机，不再按 VM 机器名判断。
  * win10-gs 没有快照。破坏性测试前必须先做逐文件哈希、属性和 ACL 的完整备份，并空跑一次恢复确认可用；执行中用有截止时间和次数上限的看门狗兜底；结束后恢复并核对 `installation_id`、`--get-id`、`key_pair` 和后台绑定。
  * 带外入口只有 termark SSH。可能切断 SSH 或网络的注入（断网、停 sshd、关机不自动回来）必须带限时自动复原，否则记 BLOCKED。
  * win10-gs 同时是编译机：测试不得删除或改动 `D:\build-env`、`D:\github` 下的工具链与仓库，不得按进程名结束进程（只能按安装路径或 PID 匹配）。
  * 测试构建与生产 agent 的安装互斥名不同，真机运行会改服务或安装目录的测试构建前，必须先停用生产 agent 的自动修复或确认不会并发修复，否则结论无效。
  * 缺少可靠恢复手段的破坏性用例记 BLOCKED，不得以未执行代替通过。
* **端到端测试（E2E）**：桌面端用户流程必须覆盖启动、首页栏目、连接入口、更新检查、下载校验、安装失败回滚和旧版本继续可用。E2E 必须使用构建出的实际程序或安装包，不得只调用内部函数代替。
* **Flutter 桌面验收**：涉及 `flutter/lib/` 的改动必须至少运行对应 widget 测试；涉及桌面导航、栏目、窗口或平台桥接时，必须增加 `integration_test` 或等价桌面驱动测试，并在 Windows、Linux 或 macOS 至少一个真实桌面目标上运行。
* **安装包验收**：涉及版本、构建元数据、更新器或打包脚本时，必须验证 Windows EXE、Windows MSI、Linux AppImage 中至少一个可生成包；更新器协议改动必须覆盖三种包的 manifest 选择逻辑，能运行的平台要做实际启动冒烟测试。

测试覆盖要求：

* 新增或修改的业务逻辑分支覆盖率目标不低于 80%；
* 不能用提高排除项、删除测试或只跑空过滤器来满足覆盖率；
* 测试命令必须确认实际执行了测试，过滤器匹配零测试视为失败；
* 任何失败路径都必须断言错误类型/状态和旧数据保留结果，不能只断言程序没有崩溃。

### 构建后的重测义务

**每次构建出新安装包后，此前所有「已修未真机验」的修复项都必须在新构建上重新测试一遍，不得沿用旧构建的结论。**

* 每个修复项必须在同样的环境、同样的版本上复测，跨构建、跨版本、跨机器的结果不能互相替代。
* 复测环境必须与上次验收完全一致：同一个版本号与构建号（`VERSION`、`BUILD_SEQ`）、同一平台与安装包类型、同一台测试机、同一套后台版本与更新策略。任何一项不一致都不能与前一轮结果对比，必须说明差异或重新建立基线。
* 复测前先清点当前待测清单（修复提交 → 对应验收用例），逐项标注「通过」「失败」「阻塞」；不得只抽样测「看起来相关」的几项。
* 任何一项失败即回到修复，修完重新构建，然后**整个待测清单重跑**，不能只重跑失败项。
* 复测未全部完成前，不得声称该构建可发布；交付说明必须列出仍未通过或未执行的项及原因。
* 已通过的项目在后续构建中若被同一区域改动波及，同样要重测，不能因为「上次通过了」而跳过。

提交前的最低检查集：

```bash
cargo fmt --all -- --check
cargo test --locked --workspace --no-fail-fast
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cd flutter && flutter analyze && flutter test
```

平台相关改动还必须运行对应目标的交叉编译或实际构建。更新器、安装器和发布脚本必须使用本地 mock/fixture 验证 `disabled`、`notify`、`download`、`auto_install`、primary/mirror 回退、大小校验、SHA-256 错误、签名错误、安装失败回滚和成功事件上报。

禁止把“能编译”当作功能验收；没有完成适用层级的测试时，交付说明必须明确列出未覆盖的平台、流程和环境阻塞原因。

## Project Layout

### Directory Structure
* `src/` Rust app
* `src/server/` audio / clipboard / input / video / network
* `src/platform/` platform-specific code
* `src/ui/` legacy Sciter UI (deprecated)
* `flutter/` current UI
* `libs/hbb_common/` shared with the server: rendezvous proto, sockets, `Config` core
* `libs/base/` (crate `base`) client-only: option keys, message proto, file transfer, platform code
* `libs/scrap/` screen capture
* `libs/enigo/` input control
* `libs/clipboard/` clipboard
* `libs/base/src/config/keys.rs` the single import path for all options

### Key Components
- **Remote Desktop Protocol**: Custom protocol implemented in `src/rendezvous_mediator.rs` for communicating with rustdesk-server
- **Screen Capture**: Platform-specific screen capture in `libs/scrap/`
- **Input Handling**: Cross-platform input simulation in `libs/enigo/`
- **Audio/Video Services**: Real-time audio/video streaming in `src/server/`
- **File Transfer**: Secure file transfer implementation in `libs/base/src/fs.rs`

`hbb_common` is a git submodule shared with the server, so changing it costs a
round-trip. Put client-only code in `libs/base` instead; it is a normal
workspace member. `base::config::keys` re-exports the handful of keys
`hbb_common` still reads, so callers get the whole set from that one path.

### UI Architecture
- **Legacy UI**: Sciter-based (deprecated) - files in `src/ui/`
- **Modern UI**: Flutter-based - files in `flutter/`
  - Desktop: `flutter/lib/desktop/`
  - Mobile: `flutter/lib/mobile/`
  - Shared: `flutter/lib/common/` and `flutter/lib/models/`

## Rust Rules

* Avoid `unwrap()` / `expect()` in production code.
* Exceptions:

  * tests;
  * lock acquisition where failure means poisoning, not normal control flow.
* Otherwise prefer `Result` + `?` or explicit handling.
* Do not ignore errors silently.
* Avoid unnecessary `.clone()`.
* Prefer borrowing when practical.
* Do not add dependencies unless needed.
* Keep code simple and idiomatic.

### Logging

* `debug` and above are written to the log file. A log call that can fire
  repeatedly (per packet, frame, input event, or loop iteration, or at a rate a
  peer controls) must not use `debug` or higher unthrottled.
* For such a site, pick one:

  * `log::trace!` when the event is expected and the line only helps while
    actively debugging;
  * `hbb_common::throttled_log!(interval, level, ...)` when it signals a fault
    that should still show up in a user's log. It keeps one line per interval
    with a count of the rest. Use `hbb_common::log_throttle::LogThrottle`
    directly only when the decision drives more than one log call.

## Tokio Rules

* Assume a Tokio runtime already exists.
* Never create nested runtimes.
* Never call `Runtime::block_on()` inside Tokio / async code.
* Do not hide runtime creation inside helpers or libraries.
* Do not hold locks across `.await`.
* Prefer `.await`, `tokio::spawn`, channels.
* Use `spawn_blocking` or dedicated threads for blocking work.
* Do not use `std::thread::sleep()` in async code.

## Editing Hygiene

* Change only what is required.
* Prefer the smallest valid diff.
* Do not refactor unrelated code.
* Do not make formatting-only changes.
* Keep naming/style consistent with nearby code.

### Imports

* One `use` per crate. Everything a file takes from the same crate goes in a
  single braced block, not one statement per item:

  ```rust
  // no
  use base::fs;
  use base::message_proto::*;

  // yes
  use base::{fs, message_proto::*};
  ```

* The only reason to split is a `#[cfg(...)]` that does not apply to the whole
  block -- an attribute binds to one item, so a differently-gated import has to
  stand on its own. A `pub use` re-export likewise cannot join a plain `use`.

  ```rust
  #[cfg(not(feature = "flutter"))]
  use base::fs;
  use base::message_proto::*;
  ```

* When splitting an existing `use` because some of its items moved to another
  crate, fold each side into that crate's existing block rather than leaving a
  second statement behind.

### Comments

* Avoid comments unless they explain a non-obvious reason, constraint, or workaround.
* Never restate what the code does; prefer clearer code instead.
* If the code is self-explanatory, add no comment.

### Be minimally invasive

* Prefer purely additive changes: layer new (`#[cfg]`-gated) blocks or new functions around existing code instead of restructuring it. The ideal diff for a fix adds lines and modifies/deletes none.
* Do not extract or reshape existing code just to enable your new code; look for a mechanism that leaves existing lines untouched (e.g. hide/show an existing object instead of refactoring its construction into a helper for rebuilding).
* Accept a little duplication over a restructure. A new function that repeats a few lines of an existing one is a better diff than reshaping the original so both can share it.
* Put new logic in self-contained functions in the module it belongs to (platform-specific logic in `src/platform/`, with `use` inside the function body to avoid churning shared import blocks). Call sites in shared files (`src/tray.rs`, `src/core_main.rs`, `src/server/connection.rs`, …) should be thin one-line hooks.

### Scope check before touching shared code

* Before changing a shared trait, a shared struct, or the signature of a widely used function, check whether the bug or feature is specific to one path. If it is, keep the change inside that path unless that is impossible, and say in the PR why it was.
* If an unrelated caller needs `Default::default()`, `None`, or another placeholder solely to satisfy a signature you changed, the diff is too broad: stop and redesign.
* The expected shape of a fix is a new function in the feature's own module, plus at most a new field or a thin hook in the shared code it needs. Feature-specific state belongs beside the feature's existing state, not in a new abstraction every caller has to learn.

### Mandatory regression-surface check

Before considering any implementation complete, perform a minimization pass over the final diff.

* Inspect every modified existing file and every modified existing code path. Each must be strictly necessary for the requested change. Revert changes that are merely cleanup, refactoring, consistency improvements, or fixes for pre-existing issues.
* For new features, preserve the existing implementation path when the feature is disabled or unsupported whenever practical. `feature off` should run the old code, not a rewritten equivalent.
* Do not route existing behavior through a new abstraction merely to share code with the new feature. Prefer a parallel new function or a small amount of duplication over changing a proven existing path.
* Keep new implementation logic in new or feature-specific modules. Changes to shared/core files should normally be thin hooks, capability checks, or protocol plumbing.
* Do not fix unrelated pre-existing bugs in the same PR. Put them in a separate change unless they directly block correctness or security of the requested work.
* For submodule bumps, inspect the exact commit range and ensure unrelated changes are not being pulled into the parent PR.
* Before finalizing, explicitly report the regression surface: list the existing files and existing runtime paths whose behavior changed, and explain why each change is unavoidable.
* During review, treat an unnecessarily modified legacy path as a review finding even if tests pass and the rewritten behavior appears equivalent.

### Corner cases raised in review

A refactor added to cover a corner case rarely converges. Each new counter, timestamp, cache or eviction/expiry rule interacts with state that existing code relies on, and the next review round finds the problems it introduced.

* A corner case is still worth fixing when the fix is easy and low-risk: a local change of a few lines that adds no state and changes no existing lookup, such as moving a check or refusing bad input earlier.
* When the only fix needs new state, a new lifecycle rule or a restructure, and the code already fails cleanly there or behaves as master does, document it as a known limit in the PR instead. Anything beyond the easy fix needs the maintainer's explicit go-ahead first.
* Before adding state that reorders, expires or reuses existing data, list every lookup that reads that data and check each one still holds.
* Prefer a clean failure, where the operation reports an error, over machinery that tries to make a rare case succeed.

## Reviewing a PR

* Review only what the diff introduces. Verify ownership with `gh pr diff` before reporting a finding — if the offending lines are untouched context, it is a pre-existing problem, not this PR's.
* List pre-existing problems in a separate section at the end, or leave out the ones that are not fatal. Never mix them into the findings the author has to fix.
* Before re-reviewing, read the author's reply comments. Do not re-raise items they declined on scope grounds.
* State a finding's consequence exactly: distinguish "the value is lost" from "the shortcut is inert but the value still saves".

## Localization (`src/lang/*.rs`)

Each file is a `HashMap<key, translation>`. Layout:

* `template.rs` is the master list of every key. **Never edit it** as part of translation work.
* `en.rs` holds only the keys whose English display text differs from the key itself.
* Every other file (`de.rs`, `fr.rs`, …) carries the full key set; an untranslated entry has an empty value: `("key", "")`.
* `it.rs` is maintained by hand by its translator. Never fill or change its entries; when adding new keys, append them to it with `""` and leave the translation to the maintainer.

### Finding the English source for a key

When filling an empty entry, determine the source English text with this rule:

* If `key` exists in `en.rs` **with a non-empty value**, that value is the source text (look it up in `en.rs`).
* Otherwise the **key string itself is the source text** (the key is already plain English).

Then translate that source into the file's target language (infer the language from the file's existing non-empty entries / filename).

### Translation hygiene

* Only fill empty values. Never change keys, and never touch existing non-empty translations.
* Preserve placeholders (`{}`) and escape sequences (`\n`, `\"`) exactly as in the source.
* Do not translate brand or technical tokens: `RustDesk`, `Socks5`, `TLS`, `UAC`, `Wayland`, `X11`, `TCP`, `UDP`, `2FA`, `RDP`, `D3D`, etc.
* Copy URL values (e.g. `doc_*` keys) verbatim from `en.rs`.

### Adding new keys (feature work)

* New English-text keys use sentence case, not Title Case: `Use ID whitelisting`, **not** `Use ID Whitelisting`. Acronyms (ID, IP, 2FA…) stay uppercase. Legacy Title-Case keys (e.g. `Use IP Whitelisting`) stay as-is — do not rename them.
* Since the key itself is the English display text, a sentence-case key usually needs **no** `en.rs` entry; add one only when the display text must differ from the key (e.g. `*_tip` keys).
* Append each new key to `template.rs` (with `""`) and to every `src/lang/*.rs` file (translated, or `""` if unsure; always `""` for `it.rs`), at the end of the list.

<!-- aoci:begin -->
## AOCI 仓库认知

AOCI 为本仓库维护一个稳定、可版本化、可增量更新的仓库级认知层，供模型跨任务复用对系统的理解。

`aoci.txt` 是面向模型的结构化认知索引。它以每个受管理文件、数据库表或其他受管理对象一条独立 Entry 的方式，用符号标签与 F/R/A/S 语义表达对象的核心职责、重要关系、对外契约，以及理解或修改系统时必须知道的非显然约束和设计决策。

Header、目录段和全部 Entry 共同组成完整仓库索引，可以覆盖前端、后端、配置、数据库结构及其他受管理内容。受管理内容发生变化时，通常只需维护受影响的认知条目，不需要重新生成整个索引。

AOCI 提供系统架构、对象职责、重要关系、对外契约和关键约束的高密度视图。

### 工作原理

AOCI 采用“模型生成、模型读取”的认知闭环。

Header、Entry 和 Curation 语义的创作只按当前机器签发的 Plan 与实时 Guide 执行；由 Host 模型基于当前绑定证据独立完成。

Entry 的语义必须来自模型对真实证据的理解。不得仅依据路径、文件名、扩展名、AST、符号列表、依赖扫描、正则、固定模板或规则引擎推导、预填、拼接或改写索引语义。

对 Fresh Bootstrap，只按当前机器签发的 Plan 和实时 Guide 执行。当它们要求创作时，Host 模型创作 Root、Meta、Tag 和 F/R/A/S，提供 authoring-run 声明，并把它绑定到 Plan、Evidence 与完整 Candidate。不得要求 AOCI 填写 `origin=host_model`、制造 Receipt 或把程序生成的 Framework 当作语义。本文件不自行重建 Onboarding 流程。内部批次不是用户决策；只有遇到既有批准边界或真实的安全、漂移、CAS、Recovery 条件才停止。

### 最小使用入口

- `aoci_rules`：取得当前AOCI版本的会话运行合同。
- `aoci_overview`：建立或恢复本仓库的完整认知。
- `aoci_maintain`：受管理对象达到最终稳定状态后检查认知是否需要维护。
- `aoci_update_entry`：提交与当前证据和源码摘要绑定的完整语义更新批次。
- `aoci_report`：仅当当前布局和工具状态支持时，在证据不足、无法可靠生成语义时登记待办，不猜写。

其他MCP工具、CLI命令、参数和专项流程，以当前工具说明、Guide和 `--help` 返回内容为准，不在本文件中重复完整手册。

本区块只规定仓库接入、认知使用和收尾原则。`aoci_rules` 承载当前会话合同，Guide实时输出承载当前Plan的执行顺序与停点，工具Schema、Spec和Validator承载机器结构与判据；Prompt、Description、README和静态文档不能覆盖这些机器事实。

### 建立、生成和恢复认知

1. 每个新的 Agent Run 开始时，应先判断：

   - 本仓库是否已经存在可用的完整AOCI索引；
   - 当前上下文中是否已有与本仓库根、当前索引版本和当前AOCI服务相匹配，并且模型仍可可靠使用的完整仓库认知。

2. 仓库已经存在可用的完整索引，但当前Run没有可靠完整认知时，先调用 `aoci_rules`，再调用 `aoci_overview`。

   完整认知仍可靠时直接复用。局部不确定本身不要求机械重读系统全貌。

   本Run从已知Host上下文压缩恢复时（包括宿主注入的压缩摘要），必须把此前模型认知视为不可靠。压缩handoff不得保留或摘要正式Whole-Index，也不得保留或摘要任何Overview Header、Entry、Chunk、Challenge或Attestation正文；只能保留安全续接所需的receipt身份、未完成write或Recovery状态，以及立即重载指令。复制进handoff的Whole-Index语义或receipt不能证明恢复后模型的当前认知可靠。若当前上下文已无法可靠保留运行合同，先调用 `aoci_rules`。继续业务任务前，使用 `refresh_reasons=["context_compaction"]` 和新的 `refresh_event_id` 调用普通完整Whole-Index `aoci_overview`（不设置 `check_only` 或设为false）；不得使用 `check_only` 或认知probe。原样跟随每个 `next_cursor` 直到 `completed=true`，确认交付，并且只基于新交付正文提交一次Attestation。完成这次新的完整传输后，即使Attestation为partial或fail也消费该generation，并按既有合同继续source-bound任务，不再自动调用第二次Overview。

   AOCI可以针对 `context_compaction`、项目 `cognition_refresh_threshold` 下的机器 `semantic_threshold` 或主要 `phase_transition` 提供checkpoint与认知状态事实。只需要这些紧凑事实时使用 `check_only=true`；这些事实只向Agent提供建议，不替模型决定是否需要系统全貌。

   Agent显式调用普通 `aoci_overview`（未设置 `check_only` 或为false）时，只要能形成一致的CognitionSet，AOCI必须完整交付请求scope。不得因为已有receipt、阈值未达到或没有待处理刷新原因而抑制正文。正式认知Dirty或Stale时仍交付正文，但必须标记不可靠。存在未决恢复或无法形成一致snapshot时失败关闭，不返回混合正文。

   普通Overview返回 `continuation_required=true` 时，必须原样提交 `next_cursor` 并自动继续到 `completed=true`。不得询问用户、开始业务任务或给出阶段性系统结论。Host截断、缺块、重复、乱序、cursor失败、Index变化或`chunk_tokens`变化时停止本次认知链。Attestation完成前不得用Memory、源码、Spec、`aoci.txt`、历史会话、scope、search或Entry读取修补或补充Whole-Index认知。Challenge ordinal是正式Entry序列中的1-based位置；Header内容、注释、空行、Section/Overview/Chunk Marker、Receipt与Metadata均不计数，Chunk Receipt ordinal使用同一序列。Attestation必须原样回绑本次Challenge发布的当前`index_sha256`、`entry_sequence_sha256`与`entry_count`；旧Index、旧Entry序列、旧数量或旧Attestation均无效。完整链结束后只正式提交一次既有模型认知Attestation；同一响应只允许一次不改变语义答案的JSON Schema或字段格式修正。对象、Tag或F不匹配即失败且认知吸收不确定，不得语义重试或旁路补答。首次认知失败时还不得执行Root/Meta、Migration、全局布局或其他未重新绑定的系统级决策。上下文压缩刷新若传输完整、认知身份不变、治理对齐且没有Recovery或第三方冲突，即使Attestation为partial或fail也消耗该refresh generation，并继续原任务，不再自动重读Overview。`system_mastery_percent`只自评系统框架——架构、职责、强关系、稳定外部契约以及高熵安全和维护约束——不表示完整实现或运行实况知识；机器索引覆盖率必须分开。默认只向用户输出由本次真实覆盖率、Challenge、块数、Token和掌握度生成的规定成功或失败一句话。Host截断时提示用户把 `overview_delivery.chunk_tokens` 设置为更小的合法值后重新开始，不得自动修改。

   加法认知等级必须与严格证明字段分开解释。`delivery_verified`表示已加载Index且Host交付已确认，但完整认知验证仍未完成；应表达为“已加载且交付已验证”，不得描述为“没有认知”或“没有理解系统”。`cognition_verified`要求Attestation通过（Challenge至少80%的ordinal完全正确且对象身份至多失手一处），`cognition_governed`还要求治理对齐。通用完整读取失败句只用于真实交付故障。

   当Overview响应包含可选`cognition-state/v2`投影时，必须分别解释各维度。其Level止于`model_cognition_usable`；`strict_attestation_verified`、`governance_aligned`与`current_system_cognition_reliable`都是独立状态，绝不参与该Level。ordinal、对象身份、Tag或核心F不匹配可以导致严格Attestation失败，而模型认知仍然可用；不得仅凭这种不匹配就宣称模型没有理解系统。只有`current_system_cognition_reliable=true`允许无保留地声称当前完整系统认知可靠。投影缺失时继续使用上述Legacy解释。

   普通的只读审计、分析、检查、不修改代码或不提交、不push，不自动等于严格零写入，也不改变上述认知有效性判断。Codex Memory和历史Skill只能辅助恢复经验、用户偏好与调查方向，不能替代与当前仓库根、索引摘要、AOCI服务身份和认知范围匹配的当前认知收据；项目AGENTS和当前AOCI身份在AOCI状态上优先于历史Memory。

   只有用户明确禁止Ledger、元数据、`.aoci`运行资产及任何文件写入时，才按严格零写入处理。若必要的认知建立与该边界冲突，必须报告冲突并请求用户裁决或建议使用隔离副本，不得静默以Memory替代当前仓库认知。

3. 仓库没有可用的完整索引，或当前只有最小骨架、Header不完整、Entries未完成、必要Curation尚未裁决时，如果需要建立正式完整AOCI索引，先取得 `aoci_rules`，然后进入当前AOCI Guide。由Guide依据仓库真实状态决定下一阶段并完成必要安全步骤。

   `aoci_maintain` 不替代索引建立流程。

   不在本文件中自行重建或硬编码完整索引生成状态机。

4. 在长程任务中，模型负责保留当前认知收据并正确使用刷新门禁：

   - Host报告上下文压缩或模型已知系统全貌丢失时，执行上述强制 `context_compaction` 重载规则；AOCI不能自行推断Host事件；
   - 进入真正的主要阶段时声明 `phase_transition`，不得把函数、测试运行或小步骤当作阶段；
   - 在有用的稳定检查点通过 `check_only=true` 取得机器语义计数；
   - 除已知压缩的强制重载外，由Agent判断当前任务是否需要再次显式获取指定scope或完整Overview；
   - 在维护和对齐完成前，保留AOCI报告的Dirty或Stale可靠性状态。

### 任务收尾与认知维护

5. 纯只读问答、分析、版本核验，或没有产生受AOCI管理对象变化的任务，不需要调用维护工具。当前AOCI版本是任意`aoci_overview` check_only或`aoci_maintain`响应里的`cognition_receipt.mcp_service_version`；二进制路径是项目`.mcp.json`里的`command`，CLI不必在PATH上。

6. 发生受AOCI管理对象变化时，待其达到本次任务的最终稳定状态后，只调用一次 `aoci_maintain`。不要在每次中间修改后逐文件维护。

   本仓库的项目约束：**每次修改内容都必须在收尾时完成一次 AOCI 对齐**，不得把漂移留到下一次任务。具体做法：改动达到最终稳定状态（测试通过、不再继续编辑）后调用 `aoci_maintain`，按第 7 条把返回的候选整批作者化写回，`remaining` 非零时继续，直到 `aoci verify` 的 `missing`、`stale`、`unbaselined`、`orphan` 与本轮改动相关的项清零。新增文件、删除文件、改动被索引文件时必须走这一步；纯只读任务与只改 `docs/` 以外未受管内容的例外见第 5 条。提交前用 `aoci check` 确认治理状态不是 `blocked`，并把 AOCI 资产变化与业务文件分开说明。若当次会话的 MCP 连接仍指向旧仓库导致工具不可用，须在报告中如实写明未对齐的部分与原因，不能默认视为已对齐。

7. 若维护结果返回真实语义候选，Host 模型必须基于每个候选绑定的对象和必要证据，独立创作完整标签与F/R/A/S更新。通过 `aoci_update_entry` 一次提交当前机器签发批次的完整候选集合，同时原样保留每项 `source_sha256`、`candidate_id` 与对应domain批次身份。`max_entries`只限制单次请求和原子事务，不限制logical plan、Whole-Index或Managed Scope。`remaining`非零时，在当前批次成功Apply后重新调用Maintain并从新preimage继续；绝不能为满足transport上限缩减Index覆盖或自行截取返回批次。

   没有足够证据且当前布局支持 `aoci_report` 时，使用它而不猜测、套用模板或为消除待办而生成缺乏证据的认知。

8. 必须遵守工具返回的结构化状态和安全边界：

   - `repair_required`：只修复明确命中的候选，再重新提交当前机器签发的完整批次；
   - `stopped`：结束当前写入尝试并检查 `failed_step`、错误、正式写入证据与Recovery。auto模式下，已证明零写入则记录closure并重新Plan；完整Intent和可证明postimage则Resume；策略要求Rollback且preimage可证明则精确恢复后重新Plan。只有证据不足、第三方正式字节冲突、需要审批或外部动作，或命中其他真实安全边界时，才停止整个用户任务；
   - 冲突、审批、人工裁决、权限和安全信号不得忽略；
   - 已经对齐后不得重复维护或重复写入；`refresh_ready_for_overview` 是checkpoint事实，由Agent决定是否为下一阶段请求普通完整Overview。

   维护完成后如果又修改了任何受管理对象，之前的维护结果失效，应在新的最终稳定状态重新完成收尾。

9. 用户只限制业务文件范围，但没有明确禁止仓库托管资产时，AOCI托管资产可以在收尾阶段为保持认知一致而更新，并应在审计和提交中与业务文件区分。

   用户明确禁止修改 `aoci.txt`、`.aoci`、元数据或任何额外文件时，以用户限制为准，不得写入，并如实报告剩余不一致。

### 专项流程

初始化、完整索引生成、Header生成、Entries生成、数据库结构索引、Curation、人工评审和故障恢复，只按当前AOCI Guide或工具在对应阶段返回的指令、命令和安全停点执行。

不预加载、不猜测，也不自行重建这些专项流程。平台调用方式、请求格式、批次上限、审批规则、索引格式细节和恢复步骤由对应Guide、工具说明、模型Prompt和CLI帮助按需提供。
<!-- aoci:end -->
