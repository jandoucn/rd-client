# T01-Z 生产 snapshot 投影测试交接

仅构建，host 未运行测试；Win10 owner 统一执行。测试要求提升管理员、`RUSTDESK_LAB_HOST` 等于本机 `COMPUTERNAME`（可选 `RUSTDESK_LAB_OS=win10`），且 `C:\RustDeskAgentLab` 已存在。每例使用独立 UUID 子目录，使用真实 workspace ACL、文件打开和生产 snapshot；不需要 MSI 包、服务操作或网络。

## 构建制品（2026-10-16）

- EXE：`C:\Users\Administrator\AppData\Local\Temp\opencode\exe-transaction-v218-target\debug\deps\management_agent_rescue_projection-d239a689a3b8c2ed.exe`
- EXE SHA-256：`D19F4E118BBCC64A8A2175B97DD5F5AA3CF90E5244D86AA2C68AC75A1609EF7E`
- 构建 driver：`C:\Users\Administrator\AppData\Local\Temp\opencode\rescue-projection-build-20261016.cmd`
- 原始 Cargo JSON：`C:\Users\Administrator\AppData\Local\Temp\opencode\rescue-projection-compile.jsonl`，SHA-256 `40F1CFD8C2AFBFDC643FB5B893407F68DA7F8833A6A49B3D02AB0BDC71966E8B`
- 原始 stderr：`C:\Users\Administrator\AppData\Local\Temp\opencode\rescue-projection-compile.stderr.log`，SHA-256 `A0CA49E94732C4670ABD83C352B4F3D4F1927FD9AEE88E602CE1DB05ED602245`
- 最终编译：`cargo test --offline --locked -j1 --test management_agent_rescue_projection --features flutter --no-run --message-format=json`，`build-finished success=true`，7.92 秒。无 compiler error，存在引用模块的未使用代码及既有 cfg 警告。
- `src/management_agent/rescue_status.rs` SHA-256：`E0EABD32902A72E2641C2E3D1B31EC0A68438003034E56FAA53A5BE4B2144DE9`
- `tests/management_agent_rescue_projection.rs` SHA-256：`1CD01D08110B69B4F0D05F7D4D8395D6C0CC628C4D6C4F5F7B1CF5F05B4A612D`
- `tests/management_agent_rescue_projection/modules.rs` SHA-256：`44D8048B236A92B4C70EF3D21C6FB1C7E1C400C16984165B52958749842437A4`

新 Rust 测试文件 rustfmt check 通过。上述摘要不表示对其他 owner 并发修改的全仓源码冻结。

## 运行参数

将 Cargo JSON `compiler-artifact` 中 target.name 为 `management_agent_rescue_projection` 的 executable 复制至 Win10。提升管理员 PowerShell 中：

```powershell
$projection = 'C:\RustDeskAgentLab\management_agent_rescue_projection.exe'
& $projection t01z_ --list
& $projection t01z_ --test-threads=1 --nocapture
$baselineExit = $LASTEXITCODE
```

必须核对非零测试数量与 exit code。预期 13 个 baseline 实际执行、1 个待评审候选 ignored；引用的生产模块自带测试通过 `t01z_` 过滤排除。此处预期不是实测结论。

候选需单独评审、单独运行并保留失败原文：

```powershell
& $projection t01z_zr06_review_candidate_unbound_waiting_must_not_override_current_instance --ignored --exact --test-threads=1 --nocapture
$candidateExit = $LASTEXITCODE
```

候选使用当前真实 waiting 报告形状，预期失败点是实际 `waiting_approval` 对比期望 `bundle_missing`，不使用新错误码、不预造绑定字段。未运行前不能登记为真实 RED。

## 覆盖

- Z-R01：bundle 缺失，无首次报告。
- Z-R02：受保护 waiting 报告；fallback incompatible 覆盖顺序与 legacy route baseline。
- Z-R03：坏 JSON、null、数组、错误字段类型；32768/32769 字节边界；独占文件锁；真实 ACL 信任失败。
- Z-R04：分别缺 bundle.json、approved-manifest.json，waiting/fallback 报告不得覆盖 bundle_incomplete。
- Z-R05：真实非 protected 目录拒绝；查询不修复 ACL。
- Z-R06：旧实例、旧 target、未知 schema、陈旧时间字段被忽略的现行为 baseline；另有未绑定报告拒绝候选。
- current cache：缺报告、坏 JSON、坏 schema/state/checked_at、实例/target/authority 不匹配、24 小时过期、时钟回退、source digest 变化、fresh degraded 缓存。

查询前后比较全部文件 bytes 和文件路径清单。独占锁案例在开锁前冻结、释放锁后复核。所有投影断言非 ready；不声称验证完整资产。bundle 中的 manifest 是明确无效测试字节，测试不调用批准、refresh 或准备入口；缓存是隔离 root 的历史状态输入。

## 实例绑定契约缺口及最小候选

`rescue_first_install::waiting()` 当前输出只有 state/error_code/ready；worker 原样持久化。失败报告也未统一携带 schema、installation_id、target_key、checked_at。它与 rescue_status 的完整校验缓存 Record 不同：后者已有 schema/installation/target_key/checked_at/source_digest/authority，并由 snapshot_at 检查绑定与 freshness。

生产 snapshot 仅在 bundle_missing 时读取首次报告，按 error_code 和 blocked_reason 覆盖投影。故旧实例的残留 waiting 报告仍可改变当前实例提示；旧 fallback incompatible 报告还可改变 state/blocked_reason 和 legacy_route_preserved。这不会把 ready 改为 true，但会误归因当前阻塞或历史路由。

如果要求“当前实例绑定”，最小修复应在首次报告生产者统一添加并原子写入版本化 envelope，至少明确安装实例字段、target 字段、schema 和采样时间；snapshot 在相等绑定、认可 schema 和约定有效期通过后才消费提示，缺字段/不匹配保持原 bundle_missing。字段名、TTL、旧报告迁移和 fallback 提示是否共用规则需先由契约 owner 确认。本任务未修改 schema 或生产者。

回归面：仅新增测试及模块声明，未修改任何既有生产文件/运行路径。生产 snapshot 已接受 agent root，无需测试 seam。未覆盖 Z-R05 symlink_metadata 真实 AccessDenied/source_unreadable、全资产验证、服务、安装、升级和恢复 E2E；由对应 owner 后续验收。覆盖率未测量。
