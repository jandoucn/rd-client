# 真实 MSI 同域镜像回退 E2E

仅在 Win10 `RDLAB-WIN10` / UUID `a6c9edff-6159-468f-bdba-08720a153424` 的 SYSTEM 任务中运行 `guest-agent-acceptance-mirror-e2e.ps1`。脚本在任何写入和执行之前校验名单与 SID；宿主只构建、传输、读取证据。

## 固定测试前提

- standard v139 build0502 健康安装，installation `76b922ac75fb14f19783bb6d48c45653`。
- `C:\RustDeskAgentLab\agent-cross-v153.msi`：SHA256 `af62de5a7edf73b8f8a515513b7680f8e48ed1c03bfe73a653efbc9f78b97782`，build0503，67,335,762 字节。
- `C:\RustDeskAgentLab\agent-auto-bootstrap-v139.msi`：SHA256 `5c4e4508ee116d64d4314d289cb7b0ad0d6d1e6c213d3607b3722a697ac05979`，build0502，67,343,954 字节。原包名不能改。
- 已存在本地 `api-tests/api`、`api-tests/php/php.exe`、`api-tests/php-test.ini`。
- `C:\RustDeskAgentLab\rustdesk_management_agent-mirror-e2e.exe` 为 `cargo test --locked --bin rustdesk-management-agent --no-run` 产物；必须保留 `rustdesk_management_agent-` 前缀以便既有 helper fixture 选择正确子入口。构建清空 `RUSTDESK_SOS`，记录源码/产物摘要，禁止修改正式版本文件。

## 执行

先核对并导出现有现场；任何恢复快照、正常关机或重启须有本轮明确授权。Upgrade/Rollback 各自要求从健康0502开始，不能直接在升级后的0503运行第二项。

```powershell
# 在动作/主体已经核对的唯一 SYSTEM 任务内调用；每次使用新 RunId。
& C:\RustDeskAgentLab\guest-agent-acceptance-mirror-e2e.ps1 -RunId mirror-<唯一标识>-upgrade -Mode Upgrade -TestSha256 <批准测试程序摘要>
& C:\RustDeskAgentLab\guest-agent-acceptance-mirror-e2e.ps1 -RunId mirror-<唯一标识>-rollback -Mode Rollback -TestSha256 <批准测试程序摘要>
```

任务名必须为 `RustDeskAgentLabMirror-<RunId>`；通过后驱动删除该唯一任务。失败时保留任务/事务/安装源/错误，禁止自动恢复快照覆盖失败现场。探测和测试等待使用 Stopwatch，超时保留 PID 与创建 FILETIME，不杀死安装器或 helper。

`CaptureSos` 可导出指定全新SOS0502安装身份 `075c943c979685b89ff9f4515560cbda`；`CaptureStandard` 核对 standard0502。Capture 的产品测试数为0。

## 断言范围

新增用例均在 `operations_runtime.rs` 的 `#[cfg(test)]` 模块中；正式下载、签名、helper、服务和安装实现没有测试专用变更。

1. 用临时密钥真实 Ed25519 签名目标和回滚清单，正式信任公钥仍拒绝该 fixture；PHP/SQLite 创建、签名领取和补报复用原链。
2. 目标/回滚各有合法同域不同 basename 镜像。回环 HTTP 四次请求精确为 primary503、target镜像200、rollback primary503、rollback镜像200；完整实际包仅由镜像返回。
3. 缓存以 primary 原文件名发布，重新核对真实 SHA/size 和文件锁，不能用镜像 basename 成为 MSI 源。
4. 独立 helper 实际 MSI 安装。升级用例覆盖领取进程退出、新进程补报与第三次重复补报；回滚用例在真实新包安装且健康后注入验证失败，执行原 MSI 回滚。
5. SQL 精确断言唯一命令终态、三条 receipt_persisted/running/succeeded 或 rolled_back 事件、sequence3、原主心跳123保持；重复轮询不改变安装回执。
6. build、agent/DLL/bootstrap、ID/UUID/installation、本地ready、原恢复任务和维护/接续任务收尾分别核对。真实远控始终 not_tested。

下载行为红测复用 `v198-mirror-red.result.json` / `v198-mirror-extended-red.result.json`，是实现前真实下载回退失败。新增实际 MSI E2E 首次即绿时应如实报告，禁止把脚本加载、权限或证据导出失败称为功能红测。

## 证据与清理

`<RunId>.result.json` 与 `<RunId>.evidence.zip` 保存在实验室根。ZIP 包含隐藏目录内计划/事务/安装结果、镜像请求/缓存证据、SQL数据库、安装和helper日志，并保留相对目录层级；不导出私钥 poll-fixture.json。导出后使用只读 SQLite 再核验命令、事件、审计和文件完整性。

`local-command-<测试PID>` 中事务的 MSI 可能已经成为 Windows Installer 注册源。该目录及包不能作为普通fixture清除。测试服务、PHP/helper进程与临时接续任务必须无残留；纯临时API源码可在确认进程退出并完成导出后精确删除，不能扩大到其他历史目录。

两个场景使用恢复快照重新准备基线时，这个动作只属于测试准备。只有场景内部 NativeInstaller 的真实原包安装、字节恢复、健康和 SQL rolled_back 才计产品回滚通过。健康检查通过后才允许保存新唯一快照。
