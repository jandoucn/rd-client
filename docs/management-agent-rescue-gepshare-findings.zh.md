# Management Agent 救援就绪 GepShare 实机复现与结论

本文记录在测试机 GepShare（termark SSH 资产，`10.0.0.254`）上对 Management Agent「完整救援已就绪=False」的实机复现与诊断结论，作为 `docs/management-agent-rescue-readiness.zh.md`（生产机 `1338209581` 诊断路线）的实证补充。macOS 侧无法编译运行该 Agent（`#[cfg(windows)]`），GepShare 作为可自由升级/重装的真机靶场。

验收边界：本轮全部结论来自真机只读取证与标准安装/卸载操作，未改动客户端代码，未重启该机操作系统，未触碰 RustDesk 以外的数据目录。救援链路的单元/集成测试矩阵仍以 `docs/management-agent-rescue-readiness.zh.md` 第 8 节为准，本文不以「实机观察」替代那一层。

## 1. 救援就绪的前置门链（waiting_approval 的真实来源）

`status --json` 的 `rescue.error_code=waiting_approval` 不是最终状态，而是首装 fallback 暂存被更早的硬门挡下的回落显示。链路（客户端 `/Users/olly/github/rustdesk`）：

```
rescue.error_code=waiting_approval
└─ rescue_status.rs 读 rescue 包目录不存在 → bundle_missing
     回落读 first-install-status/last-check.json，其 waiting_approval 覆盖显示
└─ last-check.json 由 rescue_first_install.rs::prepare() 写
     第一道门：release-cache\first-install.json 这个 sidecar 不存在 → waiting()
└─ sidecar 仅当 coordinate_current_installation() 返回 Delivered 才写
└─ first_install_approval_delivery.rs::coordinate_current_installation()
     按序过硬门，任一失败都被 300s 节流日志
     「first installation approval unavailable: <DeliveryError>」吞掉，
     对外只留 waiting_approval
```

硬门顺序与对应 DeliveryError：ACL/installed-state（UntrustedPath）→ 身份/target_key（IdentityMismatch）→ Config 路径（IdentityMismatch）→ MSI product code / 已注册包名（RegisteredSourceMismatch）→ gates（LocallyBlocked）→ 健康管道 build 比对（BuildMismatch）→ 活动会话（LocallyBlocked）→ coordinate_approval。

结论：`waiting_approval` 只说明「还没过审批门」，不指明是哪一道门。必须读节流日志里的 DeliveryError 才能定位确切子原因。

## 2. 干净规范安装 → 完整 fallback 暂存（链路端到端可用）

GepShare 清空后用**规范文件名** MSI `rustdesk-1.5.3-2026101805-standard-windows-x86_64.msi` 全新安装（exit=0），随 Agent 后台周期观察到：

- `release-cache\first-install.json` sidecar 已写（coordinate 返回 Delivered，越过全部审批门）。
- `rescue\first-install-staging\` 出现**完整 fallback 负载**：`client\`（flutter 全量 DLL、`files\data|drivers|usbmmidd_v2`、`manifest.json`）、`agent.exe`、`approved-manifest.json`、规范名 MSI、`extract.log`、`image\`。
- 此时 `rescue.ready=False error_code=bundle_missing freshness=unverified`，`current` 包尚未生成。

这端到端证明：**前置门全部通过时，审批下发 → 下载批准包 → 解压客户端 → 暂存 fallback 一路成功**；`bundle_missing` 是「已暂存、待验证提升到 `current`」的中间态，提升由 Agent 自身的验证/轮换后台周期（worker 300s 节流）完成，不是链路缺陷。对照生产机 `1338209581` 卡在 `waiting_approval`（连 sidecar 都没写），GepShare 推进得远得多。

## 3. RegisteredSourceMismatch 是非规范文件名的复现伪影

首轮在 GepShare 升级复现时命中的是 `RegisteredSourceMismatch`（与生产机的门不同）。根因是我把 MSI 以非规范名 `rd-2026101805.msi` 安装，`deliver_bound()` 要求已注册 MSI 的 PackageName 等于规范 `cache_name`，故拒绝。

这不是生产问题：在线更新器 `src/updater.rs` 走 `primary_package_name(&target.primary, "msi")`，落地文件名必为规范名；生产机不会因此命中该门。改用规范名安装后此门即消失（PackageName 变回规范名）。

## 4. 生产机卡在 UntrustedPath（ACL），尚未定位

生产机 `1338209581` 的节流日志为 `first installation approval unavailable: UntrustedPath`——卡在最早的 ACL/installed-state 门（`preservation_state_read.rs` 对 `C:\`、`ProgramData`、`RustDesk`、`ManagementAgent` 及各 state 目录做严格 owner+ACE 白名单校验）。GepShare 的目录 ACL 干净、未复现该门，故生产机的具体 ACL 偏差仍需在那台真机上取证（dump 各级目录 owner/ACE 与白名单比对），不在本轮范围。

## 5. 卸载残留 RecoveryBootstrap → 全新安装 1603 的处置

本轮恢复 GepShare 时发现：**用 `msiexec /x` 等非脚本方式卸载后，`C:\ProgramData\RustDesk\RecoveryBootstrap` 可能残留**；随后全新安装时，MSI 自定义动作 `InspectBootstrapUpgrade`（`res/msi/CustomActions/Bootstrap.cpp`）的预检 `Test-RdBootstrapUpgrade`（`res/msi/bootstrap-preflight.ps1`）因 `RecoveryBootstrap` 存在而判定为 bootstrap 升级场景、要求主服务存在，而全新环境无主服务，导致安装 1603 失败。

处置：用仓库既有的 `scripts/clean-rustdesk-windows.bat` 做完整清理——它 `rmdir /s /q "%ProgramData%\RustDesk"`（连带 RecoveryBootstrap）与 `%ProgramFiles%\RustDesk`、服务、注册表项，清理后用规范名 MSI 全新安装即 exit=0。本轮即以此恢复 GepShare。

结论：这是「非完整卸载残留」触发的场景，不是产品自发残留；正确卸载/清理应始终走该脚本，不要只 `msiexec /x`。

## 6. GepShare 当前状态

- 客户端构建 2026101805，standard/msi，PackageName 规范名，`RecoveryBootstrap` 不存在。
- `RustDesk`、`RustDeskManagementAgent` 两服务 Running。
- 救援：sidecar 已写、`first-install-staging` 完整，`rescue.ready` 待 Agent 后台验证周期提升到 `current`（`freshness=unverified`→待提升）。
- 机器健康，可继续作为救援链路真机靶场。

## 7. 约束与不改项

- 未改客户端代码；未重启 GepShare 操作系统；未触碰 RustDesk 以外数据目录。
- 固定配置不变：密码 `asd123asd`、ID 服务器 `rd.yan.life`、API `https://rdapi.yan.life`、下载域 `download.yan.life`、更新公钥 `yan-release-2026`。
- 后续若改代码定位生产机 UntrustedPath，须按 TDD（先失败测试→最小实现→补边界），本地 disable-recovery/disable-agent/签名哈希校验/回滚门禁优先于远程命令，不借修复改协议/字段语义/默认权限。
