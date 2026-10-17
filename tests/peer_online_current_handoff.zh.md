# 当前 peer-online 基线待执行

本轮仅增加 peer-online 测试和源码打包器，生产模型保持接管时状态。没有运行 host 客户端测试，没有申请或占用 VM，没有 CI、提交或子代理。主控分配 Win10 后先执行本轮基线，再按真实失败断言实现最小修复。

## 精确数量

- `ab_online_reload_test.dart`：16 项（原有 10；新增 personal=true/false widget 2、无效 HTTP 200 响应 3、第二页失败 1）。
- `peer_online_reload_test.dart`：7 项（原有 4；新增切换同 ID 隔离、返回个人通讯录、recent metadata 各 1）。
- 合计 23 项模型/widget；本轮实际执行 0，RED/GREEN 均待 VM 回执。
- 既有 `tests/fixtures/peer_online/integration_test/peer_online_session_test.dart`：独立 1 项真实连接 E2E，不加入上述 23 项计数。
- 历史 30 green 属于此前四套件，不能作为新增九项的运行结果。

## 源码结论与待验证假设

1. Windows/macOS 主 engine 和 Android 都经过 `model.dart` 的 `sync_peer_hash_password_to_personal_ab`，调用 `AbModel.changePersonalHashPassword`，更新个人 Ab 后 reload。当前 `Ab.pullAbImpl` 已在 await 后按本实例 ID 合并 online。个人 `personal=true` 与共享 `false` 的获取路径相同，hash 写入权限不同。新增 widget 从真实 pull 开始观察等待期间和加载后的帧，不能宣称模拟了真实 connect/hash 写入。
2. `Peers._updatePeers` 在 supplier 返回不同 RxList 后仍把旧列表的 online 写入新列表。新测试通过真实 load handler 切换列表，预计暴露跨通讯录状态泄漏和返回已验证个人列表丢失 online；没有发送虚假的 offline 回调制造失败。需要 VM 验证后再改 supplier 路径，保留 recent/favorite 无 supplier 的按 ID 合并。
3. `_fetchPeers` 对 HTTP 200 的 `{}`、缺 data、data=null 返回成功，随后提交空 peers。三个新用例要求失败状态并保留旧对象和值；这不是有效的 `total=0,data=[]` 空列表。第二页 503 用例核验已有失败保留行为。
4. `loadCache` 校验缓存 access_token，但 await 后未重新读取当前 token；cache 反序列化创建新实例并 reset，不能把缓存 online 当作已验证证据。完整 logout/inflight/cache 竞态尚无测试 seam 和 VM 证明，不宣称账户隔离已闭。
5. `platform_model.dart` 只是 native/web 导出；三平台 native handler 共用。现有 online 回调只更新列出的 ID。`peers_view.dart` 的查询 catch 不写 offline；固定集合轮询直接 query 的路径及 Rust FFI 忽略入队错误仍需其他 owner 处理，不在本轮改动范围。
6. `syncFromRecent` 原位写 metadata；recent load 已有按 ID 保留。尚不能将所有 metadata/缓存失败路径都声明覆盖。

## VM 执行交接

使用既有 `peer-reload-kit` 的隔离 project/SDK。主控核验本轮 zip 中逐文件 SHA-256，覆盖对应测试副本；E2E 源码单独保留，不在普通 flutter test 中运行。复用已有 `peer-reload-run.ps1`，传入唯一 Phase 和两套件 Tests；它会创建独立日志目录并保留退出码，不要复用旧输出目录。

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\RustDeskAgentLab\peer-reload-run.ps1 -Phase current-baseline-20261017 -Tests 'test/ab_online_reload_test.dart test/peer_online_reload_test.dart'
```

归档 tests.jsonl、stderr.log、result.json 和源码摘要；排除 hidden suite-load 项后必须恰好 23 个非 skipped 用例。编译或初始化失败不算业务 RED。新增失败须保存完整 assertion；通过的已有修复路径记录 baseline pass，不回退生产修复来制造 RED。

真实 E2E 沿现有 `tests/fixtures/peer_online/README.zh.md` 的隔离 fixture、窗口握手、首帧挑战和服务端审计执行。旧构建不代表当前源码；当前入口只支持 Windows，Android/macOS 实场仍待各平台资源。既有文档记录外部 driver 未闭合，不能生成自述成功响应替代。要实现完整事件模型测试，需先向主控报告 `model.dart` 全局 gFFI/native binding 的 hook 边界；本轮未修改该文件。

回归面：本轮没有生产路径变更，仅新增九个模型/widget 用例。无 AOCI 索引、项目 .mcp.json 或可调用 AOCI 工具，未生成认知资产。
