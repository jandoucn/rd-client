# Management Agent 完整救援就绪诊断与修复路线

本文记录 Windows `standard/msi` 客户端在线自升级到 `2026101805` 后，Management Agent 报告 `完整救援已就绪=False` / `救援资源提示=waiting_approval` / `救援核验=unverified` / `恢复目录程序存在=False` 的诊断过程、已证明的事实、代码级根因链，以及在 Windows 侧继续排查/修复的操作路线。接手任务者按本文执行，不必重复已完成的调查。

现场状态（真机 `status --json` 关键字段）：
- 客户端 ID 1338209581，安装类型 standard / msi，Agent 构建号/主程序构建号均 2026101805，主程序 ready、已注册，服务路径可信。
- 本地启用 enabled、服务恢复 effective_enabled=True、当前任务 idle、服务恢复错误为空。
- `完整救援已就绪=False`、`救援资源提示=waiting_approval`、`救援核验=unverified`、`恢复目录程序存在=False`。
- capabilities 全量齐备（agent.status/service.*/recovery.reconcile/package.verify/diagnostics.collect/network.probe/package.reinstall/package.update/agent.update）。

## 1. 已证明健康，不要重复调查

以下在 macOS 侧只读验证通过，结论是远程源与后端都不是阻塞点：

- 生产 `https://rdapi.yan.life/rd/update/v1/manifest/stable.json` 正确：version 1.5.3、build_seq 2026101805、product rustdesk-yan、schema 2。
- 其中 `windows-x86_64-msi-standard` 完全合规：build_seq 2026101805(int)、signature_key_id `yan-release-2026`、签名 base64 解码 64 字节、size 29588717、sha256 64hex、primary `https://download.yan.life/rustdesk/stable/v1.5.3-build-2026.10.18-05/rustdesk-1.5.3-2026101805-standard-windows-x86_64.msi`（主机+路径均在白名单）、含 GitHub 镜像。
- 该 MSI 下载地址 HEAD 返回 HTTP 200（OSS 对象存在可达）。
- 后端 agent resolve/批准/命令链黑盒回归全绿：仓库自带 `tests/agent_operations_test.php`(76 断言)、`tests/agent_status_test.php`(30 断言) 通过；自定义"任意构建号"场景 10 断言通过，证明后端在任意 build 下只放行正好匹配的"已批准包"（`update_releases` 中 active=1 且 channel=stable 且 manifest 通过 `agent_package_reference` 校验者），并对未知 build/撤销/篡改 package_id(409)、身份不符/非白名单源/签名非 64 字节/key 错(422) 正确拒绝。

结论：**manifest、签名包、OSS、后端批准/选包逻辑都正确。阻塞点在 Windows 侧 Agent 本地那份 fallback 救援包尚未下载并暂存核验完成。**

## 2. 代码级根因链（客户端 `/Users/olly/github/rustdesk`）

```
status 显示 waiting_approval
└─ src/management_agent/rescue_status.rs::snapshot（约 303 行）
     读 rescue 包目录不存在 → error_code=bundle_missing
     回落读 first-install-status/last-check.json（约 316-320 行）
       若其 error_code=waiting_approval → 覆盖为 waiting_approval
└─ last-check.json 由 src/management_agent/rescue_first_install.rs::prepare()（约 630 行）写入
     prepare() 第一道门（644-647 行）：
       release-cache\first-install.json 这个 sidecar 不存在 → return waiting()（264-265 行，{error_code:waiting_approval,ready:false}）
└─ sidecar 只有 coordinate_current_installation() 返回 Delivered 才写
     调用点：rescue_first_install.rs:927（DeliveryError 会被 throttled_log 吞掉并打日志）
└─ src/management_agent/first_install_approval_delivery.rs
     coordinate_current_installation()（约 248-312 行）按序过门，最后调 coordinate_approval()
     coordinate_approval()（64-101 行）：
       target_key 必须是 windows-x86_64-msi-(standard|sos)
       fetch_fixed_manifest() 取 rdapi.yan.life stable.json（>256KiB 或失败 → WaitingSource）
       解析 UpdateManifest 失败 / package_gate::verify 失败 → WaitingSignature
       build(manifest 的 target build) != current.current_build → WaitingExactBuild
       全过 → deliver 写 first-install.json → Delivered
```

关键排除项：`current.current_build` 不是运行时读的，而是 `include_str!("../../version.json")` 的 `build_seq` **编译期焊进 Agent 二进制**（first_install_approval_delivery.rs:266-269）。现场 Agent 构建号 2026101805，manifest build 也是 2026101805 → **build 比对必过**，`WaitingExactBuild` 基本排除。因此卡点在 `coordinate_approval` 之前的更早硬门。

`coordinate_current_installation` 的硬门（按代码顺序，附对应 DeliveryError）：
1. ACL / installed-state 校验 → UntrustedPath
2. state/agent.json 读 agent_installation_id、target_key → IdentityMismatch / TargetMismatch
3. Config::file() 必须是 LocalService 或 systemprofile 的 RustDesk.toml → IdentityMismatch
4. current_product_code() 从注册表读 `C:\Program Files\RustDesk` 的 MSI product code → RegisteredSourceMismatch
5. gates(...)
6. 健康管道 `\\.\pipe\RustDesk\management-health-v2` 探测并要求回报 build=2026101805 → 失败即 BuildMismatch（292-298 行）
7. `health.no_active_sessions != Some(true)` → LocallyBlocked（约 300 行）；rescue_first_install.rs 的 first_install_healthy 亦有同款"会话活动即拒绝"检查
8. coordinate_approval（见上）

## 3. 待判定的确切子原因 + 取证

DeliveryError 类硬门会被这行日志吞掉（rescue_first_install.rs:927，300s 节流）：`first installation approval unavailable: <DeliveryError>`；而非 Delivered 的 CoordinationOutcome(WaitingSource/Signature/ExactBuild) 静默返回 Ok、不打日志。Agent 日志经 `hbb_common::init_log(false, "management-agent")` 输出，文件名 `management-agent*.log`。

在真机上取证：
```powershell
Get-ChildItem -Recurse C:\Windows\ServiceProfiles, C:\ProgramData\RustDesk -Filter 'management-agent*.log' -EA SilentlyContinue |
  ForEach-Object { Select-String -Path $_.FullName -Pattern 'first installation (approval|preparation) unavailable' } |
  Select-Object -Last 20
```
决策树：
- `LocallyBlocked` → 有活动会话，救援暂存被故意推迟（最可能，尤其你正连着机器排查时）。
- `RegisteredSourceMismatch` → 在线升级改了 MSI 注册信息（真代码/流程问题，需进一步定位）。
- `BuildMismatch` → 健康管道没回报 2026101805（重启 RustDesk 服务）。
- 一行都没有 → 静默 WaitingSource：那台机器自己取不到 manifest，在该机执行 `Invoke-WebRequest -UseBasicParsing https://rdapi.yan.life/rd/update/v1/manifest/stable.json`。

领头假设：**活动会话 LocallyBlocked**，其次 **本机取不到 manifest(WaitingSource)**。很可能根本不是代码 bug。

## 4. Windows 侧诊断与修复步骤（先在真机做，非破坏性）

1. 先确认该机能下包：
   `Invoke-WebRequest -Method Head "https://download.yan.life/rustdesk/stable/v1.5.3-build-2026.10.18-05/rustdesk-1.5.3-2026101805-standard-windows-x86_64.msi"`（期望 200）。
2. 断开所有到该机的远程会话，`Restart-Service RustDeskManagementAgent`，等 5–10 分钟（worker 300s 节流 + 下载），再看 `status --json` 完整输出（上面那段 PowerShell 的 pretty 版只是子集）。
3. 若仍 False，从 ops-console 对该设备发 `recovery.reconcile` 或 `package.reinstall` 命令，成功执行会顺带把 fallback 暂存好。

## 5. 若确需改代码：本地快循环（用 VirtualBox VM 当靶场）

要点：agent 是纯 Rust、`#[cfg(windows)]`，**不需要 Flutter**。在 x86_64 Windows 上原生编译/测试最简单、最保真。生产机只用于读日志；编译、集成、复现都在 **VirtualBox Win10/Win11 VM** 里做（先 snapshot，破坏性测试后回滚）。

一次性环境（版本以仓库为准）：
- Rust ≥ 1.75，默认 `stable-x86_64-pc-windows-msvc`；Visual Studio C++ Build Tools。
- vcpkg：提交 `9e593bb18ea69cc5095e012465dcd675a822ed0d`，triplet `x64-windows-static`，参照 `.github/scripts/bootstrap-vcpkg.sh` 与 `vcpkg.json`（aom/ffmpeg/libvpx/libyuv/opus/mfx-dispatch + `res/vcpkg` overlay）。
- `git clone --recursive`（含 hbb_common 子模块）。
- 不装 Flutter（`flutter_rust_bridge` 为 optional feature，agent 不需要）。

循环：
- 首次 `cargo build --bin rustdesk-management-agent` 慢（vcpkg 编 ffmpeg 等 native 库，30–60 分钟+），仅一次；保持 `target/` 温热、绝不 `cargo clean`。
- 之后改 `src/management_agent/*.rs` → 增量秒级到 2 分钟。
- 单元测试：`cargo test -p rustdesk management_agent::`（fixture 覆盖 coordinate_approval/rescue_status/prepare/package_gate 等，纯逻辑、不碰真实服务，安全）。
- 集成：只 `cargo build --release --bin rustdesk-management-agent`（分钟级，不编 Flutter/MSI），把单个 exe 放进 VM 验收真实服务行为。
- 本地全绿后，**只在发版时**手动打 tag 触发那 1 次签名发布；仓库已配置 push 不触发任何构建。
- 可选优化（改构建结构，先不做）：把 agent 抽成只依赖 hbb_common/base/windows 的独立 workspace crate，可去掉 vcpkg、连首次都快。

## 6. Agent 救援子系统文件地图

- `src/management_agent_bin.rs`：入口，子命令 `status`/`status --json`/`check-now`/`--service`/`install-bootstrap`/`bootstrap`/`uninstall --purge`/`--cleanup`；`init_log(false,"management-agent")`。
- `src/management_agent.rs`：服务主循环、状态快照装配、recovery 调度。
- `src/management_agent/rescue_status.rs`：对外 `rescue` 字段（freshness/ready/error_code）装配。
- `src/management_agent/rescue_first_install.rs`：首装 fallback 暂存 worker、`prepare()`、`waiting()`。
- `src/management_agent/first_install_approval_delivery.rs`：`coordinate_current_installation`/`coordinate_approval`/各硬门与 DeliveryError。
- `src/management_agent/rescue_assets.rs` / `rescue_recovery.rs` / `rescue_rotation.rs`：救援资产准备、恢复周期、轮换。
- `package_gate.rs`：签名/目标/尺寸校验。`health_windows.rs`：健康管道探测。
- 后端对接：`~/github/rustdesk-api` 的 `sqlite/index.php` + `sqlite/agent_operations.php`（resolve/批准/命令/回滚）、Go `stream-broker`（命令/策略 SSE）。

## 7. 硬约束（AGENTS.md）

- 只在 `master` 提交；提交说明用中文（类型前缀可英文），不写英文句子、不加署名、不用 emoji；只暂存任务相关文件。
- 固定配置不可改：密码 `asd123asd`、ID 服务器 `rd.yan.life`、API `https://rdapi.yan.life`、下载域 `download.yan.life`、更新公钥 `yan-release-2026`、连接公钥 `WAVL+YUYZ6EXOqLCGZEq56VD2LalZc121gtVeXxpp78=`。
- 最小侵入 + TDD：先写失败测试→最小实现→重构补边界；不借修复改协议/字段语义/认证/默认权限；本地 disable-recovery/disable-agent/签名哈希校验/回滚门禁优先于远程命令。
- 安装包只由手动 Flutter Tag Build 产出；不给提交/定时/tag 加自动构建。

## 8. 验收

- 单元：`cargo test -p rustdesk management_agent::` 全绿，新增分支覆盖正常/边界/失败路径。
- 集成（VM）：干净安装 standard/msi 2026101805 后、以及模拟在线自升级后，分别观察 fallback 是否暂存、`status --json` 的 `rescue.ready` 是否转 True，断言失败路径（无会话/有会话、可下/不可下）各自的 error_code。
- 不得把"能编译"当作功能验收。
