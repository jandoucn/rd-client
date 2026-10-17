# 通讯录连接后全离线：修复与 VM 验证交接

## 当前状态（唯一有效交接状态，2026-10-16 接管核验）

最新 P2 增量：独立复审发现同一 Ab 实例按名称复用、GUID 改变时会继承旧通讯录 online。已在 setSharedProfile 的 GUID 边界清理旧在线值，同 GUID 改名保持；pullAbImpl 在 peers/tags 的 await 后核对请求开始时 GUID，旧响应不提交。没有引入全局缓存或修改查询接口，独立复审已关闭该 P2。

- 原始 VM RED：`peer-profile-p2-original-red.jsonl`，10 项中 8 passed、2 failed、0 skipped；日志 SHA-256 `28b66ed0f1be00975212dcd7e86eb30ae70e87005a1ade9e6980f90dc081654d`。
- 对应 GREEN：`peer-profile-p2-original-green.jsonl`，四套件共 30 passed、0 failed、0 skipped；日志 SHA-256 `594eea2f6d52f3f1f4dcb96c7b9c916d65844cc5f3a246276159214d82de0a34`。
- 主 agent 独立复跑当前 Ab 套件：10 passed、0 failed、0 skipped，6.796 秒；`peer-profile-red-20261016a-tests.jsonl` 虽文件名带 red，实际为修复后通过记录，不算 RED。
- 当前模型 SHA-256 `c5d044798f9548cc31be7e6cc143c884863f8cbfade0a7fc92ec89a65b541280`；测试 `ab_online_reload_test.dart` SHA-256 `fd0549fbdcbc04bdf950077d9fc310183ff85f2f1198e1034bc0ea1fce3f34e5`。完整逐例统计见 TEMP `peer-profile-p2-audited.json`。
- 主 agent 收尾查询未见 Dart/flutter_tester/consent/msiexec，两项服务 Running；此次普通令牌查询不替代历史 SYSTEM 完整摘要。新批次日志及 SDK 保留用于验收追溯，没有重新安装或控制产品服务。

以下 25/7 项结果和对应模型摘要为 P2 修复前历史，不得代替上述最新代码身份。测试始终是 VM 中的模型/widget 层；真实连接、关闭会话及锁屏/注销可控性仍未获得完整 E2E 证明。

下列条目保留 P2 修复前的首次接管核验；其中“当前”仅指当时阶段，源码身份以本节开头的 P2 增量为准。后续主 agent 已独立重跑 Ab 十项。历史章节中的“未修改/未运行/待运行”不代表最新状态。

- 实现回归面仅 `flutter/lib/models/ab_model.dart`：非 legacy Ab 在 HTTP 返回后、替换共享 RxList 前，从本 Ab 当前 peers 按 ID 复制 online；失败保留旧 peers；HTTP/API/header 可选注入用于测试。没有全局状态缓存。生产默认 HTTP 路径沿用原调用。
- 有效 RED：`peer-reload-red2.jsonl` 4 项中 2 通过、2 失败；`peer-reload-ab-red2.jsonl` 真实 Ab 路径 5 项全部失败。
- GREEN：`peer-reload-all-related.jsonl` 25/25，通过且无 skipped，`done.success=true`；追加的 `peer-reload-isolation-green.jsonl` 7/7，通过且无 skipped，涵盖不同 Ab 同 ID 隔离、有效空列表后重新添加默认 false、权威 offline 回调。后者含前者的 5 个 Ab 用例，不是另增 7 个独立用例。初版四项 RED 是模拟列表替换，后改为真实 Ab.pullAb，不能宣称这四项测试原封不动由红转绿。
- 当前 host/guest 源码 SHA-256 均与 `peer-reload-final-evidence.json` 匹配：模型 `7ba21be531f129b36814cd11e88ee815669356739e0852e0aa6f8007f3555450`；peer 测试 `05ed5d9cff7585c72fadb69c2d7db5643e2827266c371f6a549a97a4442c33a5`；Ab 测试 `c08b06d7a2571782c1334097448df955af225c1a519153d78dc4edeb87450e4b`。25 项归档内 Ab 测试仍是 5 用例版 `b33b3bb26a4024c7f38dd02711a83cddf3ef12ef5b494feec0b0153e93619925`，当前 7 用例版由后续 isolation-green 覆盖。
- 历史 SYSTEM 健康记录时间为 `2026-10-15T14:14:50.9065415Z` / `14:57:52.4088206Z`：6 项摘要相同，两服务 Running、PID 2132/3940 相同。旧 0510 agent 不支持该 JSON 输出，`application=null`，不据此宣称 ready。
- 本次只读释放检查时间 `2026-10-15T20:21:11.4260750Z`，目标 `RDLAB-WIN10`：consent 为空，历史 PID 4300/4668 均不存在；Dart/flutter_tester/msiexec/helper 为空；当前身份可见的 RustDeskAgentLab* 任务为空。两服务 Running，当前 PID 2892/2952，不能将其写成与历史 PID 不变。本次未执行关闭提示、kill、注销任务或删除 SDK；旧提示现在不存在，无法追认其退出原因。VM 独占窗口已交回，下一锁屏测试可接手。
- 独立 SDK/cache、guest 批次日志和 host 原始证据均保留。当前测试只证明 MockClient 驱动真实 Ab 路径及 widget 行为；真实连接/关闭 session、完整桌面 E2E、全平台、80% 分支覆盖率未验证。新账号实例隔离通过，不等价于完整退出登录与在途 HTTP 取消验收。

Review 证据根目录：`C:\Users\Administrator\AppData\Local\Temp\opencode`。原始 JSON、脚本和健康记录保持原文件：

| 证据 | SHA-256 |
| --- | --- |
| peer-reload-red2.jsonl | 647856feff4b876e46f29982175652f7abecdb49d3d81b7fa2e116ded97a50fc |
| peer-reload-ab-red2.jsonl | 432e51451a5c6fdcd6a6e8014c0dd716fd1316ec38ec65c4cbc5b8020c3d0475 |
| peer-reload-all-related.jsonl | a5661303e7d91b69c560fc4ebc52125ceb8e968a02fec99ed935725486fed627 |
| peer-reload-isolation-green.jsonl | d1c64a8be3dccd5cdfbac848144b54cd52bbb20d0af7b91e7ace569f8e88d9ae |
| peer-reload-green-source.zip | 7d03b20d76c0cbb37ab78907162ee1d11216202d57a3615cbea35cc189439ed0 |
| peer-reload-run.ps1 | 273fc77a73591f6b810cbfdb12f385505ed09a403430dff306c7db4612bdfa76 |
| peer-reload-control.py | 5d4d8fc827fd86e4a15b65cb2bbc7451476df67d80224395d164d379ba6d9788 |
| peer-reload-green-pack.py | a9368e415c9fb8002d7f2852e8acbce904c9e8f30a561788037a9f9d929207a9 |
| peer-reload-green-stage.ps1 | 82d09cb88f21dacf52debf12a85062777274d7be7323649df1687abea2c09cc9 |
| peer-reload-final-run.ps1 | 6665c47a2d26f6cc30c09ce1dec0595f3853e6b0c5945f96612c0534bba591c2 |
| peer-reload-health.ps1 | 65112205e48b4ef1bd06cf80b1f99405a7080843e2511256daf875fe5792beac |

复核脚本 `peer-reload-review-audit.py` 只读比对上述日志、源码、归档及健康记录；`peer-reload-release-inspect.ps1` 为本次 VM 只读释放检查源码。`peer-reload-final-evidence.json` 保存逐例结果；`peer-reload-health-before.json` / `peer-reload-health-after.json` 保存历史 SYSTEM 六项摘要。

## 历史：2026-10-15 实际 VM 增量结果（以顶部当前状态为准）

Win10 guest 已运行 Flutter 3.24.5 / Dart 3.5.4，host 未运行客户端或 widget 测试。使用既有完整 SDK 缓存压缩包，guest 独立目录 C:\RustDeskAgentLab\peer-reload-kit，没有下载 SDK、安装或升级旧 0510 SOS 产品。

有效 RED：peer-reload-red2.jsonl 四项中 2 通过、2 业务失败；peer-reload-ab-red2.jsonl 五项全部业务失败。重载在线值由 [true,true,false] 变成全 false；503 将旧列表清空。首次 red/ab-red 因代理影响本地测试 WebSocket 而 suite 加载失败，不计业务 RED。

实现仅修改 flutter/lib/models/ab_model.dart：非 legacy Ab 成功获取 peers 后，在 await 之后按当前 ID 复制当前在线状态，再替换共享 RxList；获取失败保留旧 peer 对象。另增加 API 地址、headers、HTTP post 的可选测试注入点，默认仍使用原生产调用。不添加永久缓存，不修改 Peer/recent/favorite、Rust 查询、session、updater 或服务路径。

GREEN：peer-reload-all-related.jsonl 共 25 个非 hidden 业务用例通过，0 失败、0 skipped，done.success=true，runner exit=0；4 个 peer reload + 5 个实际 Ab HTTP/model/widget + 8 个 autocomplete + 8 个 address_book_customization。JSON 日志耗时 14.502 秒。初版四项测试直接模拟 RxList 替换，随后改为调用真实 Ab.pullAb；五项 Ab 测试在修复前后保持行为断言一致（仅格式化）。因此不能声称初版四项测试原封不动转绿，真正生产路径 RED/GREEN 由 Ab 五项证明。

证据位于 host C:\Users\Administrator\AppData\Local\Temp\opencode：peer-reload-red2.jsonl、peer-reload-ab-red2.jsonl、peer-reload-all-related.jsonl、peer-reload-health-before.json、peer-reload-health-after.json。guest 同名批次目录保留原始 tests.jsonl、stderr.log、result.json。

GREEN 归档 SHA-256：7d03b20d76c0cbb37ab78907162ee1d11216202d57a3615cbea35cc189439ed0。
已实测模型 SHA-256：7ba21be531f129b36814cd11e88ee815669356739e0852e0aa6f8007f3555450。
peer_online_reload_test.dart：05ed5d9cff7585c72fadb69c2d7db5643e2827266c371f6a549a97a4442c33a5。
ab_online_reload_test.dart：b33b3bb26a4024c7f38dd02711a83cddf3ef12ef5b494feec0b0153e93619925。

SYSTEM before/after SID=S-1-5-18；6 项程序/身份/配置摘要一致，两服务 Running，PID 2132/3940 保持。旧 0510 agent 的 status --json 仅输出根目录，不支持 JSON，故不能宣称应用 ready 已验证。测试后没有 Dart/flutter_tester/msiexec/helper 进程或运行中的 lab task。独立健康任务自行注销；两个早期 guestcontrol 触发的 PowerShell consent 提示（当时 PID4300/4668）仍可见于进程枚举，未仅凭 PID 强杀，UAC残留清理尚未闭合。原有 RustDeskAgentLabDriver 未动。

边界：HTTP fixture 使用 MockClient，执行真实 Ab 解码/分页/提交路径，不是 loopback socket/Rust HTTP 集成。没有新桌面程序构建或连接/关闭 session E2E；没有证明完整过期在线查询响应拒绝、poll恢复、显式三态 unknown/stale UI、全部平台或80%分支覆盖率。本补丁只保证元数据重载失败不制造 offline，并保留请求期间最新已接收在线回调。以上未闭项不能用25项通过替代。

## 历史：最初仅编写测试阶段（已被后续修复与 VM 结果取代）

当时仅增加 Flutter 行为测试和本说明，生产代码未修改。当时没有执行 host 客户端/widget、VM 命令、安装、服务操作或线上远控；尚未取得有效 RED/GREEN。以下至文末均为这一早期阶段的记录，不作为当前交付状态。

## 基线与最小候选链

HEAD：9d8d430c0（工作区含其他 owner 的大量修改，不能以 HEAD 替代源码摘要）。

旧修复 18287e819 仍存在：Rust 网络查询失败不发送全离线回调；不完整位图报错；Flutter 查询增加 in-flight 和 catch。当前另发现 FFI query_onlines 忽略 async_tasks::query_onlines 返回错误，因而队列满不会传给该 catch。

与连接触发吻合的源码链：

1. models/model.dart 收到 sync_peer_hash_password_to_personal_ab。
2. AbModel.changePersonalHashPassword 调 personalAb.pullAb(quiet: true)。
3. 非 legacy Ab.pullAbImpl 将 Peer.fromJson 创建的 tmpPeers 写入 peers.value，每个新对象默认 online=false。
4. Peers.getInitPeers 返回 currentAbPeers；Peers.peers 与原 RxList 共用容器。容器替换后 _updatePeers 才调用 _getOnlineStates，读到的是新对象的 false，旧在线证据已丢失。
5. 窗口 blur 会让 peers_view 的固定集合轮询暂停；focus/restore 才恢复。是否实际收到恢复事件尚无真实程序证据。

这是源码支持的候选链，尚未在真实连接中复现。legacy _deserialize 已恢复旧在线 ID，与非 legacy 路径不同；不能把此次问题归为旧 Rust 修复被撤销。

## 行为测试

文件 flutter/test/peer_online_reload_test.dart。经真实 platformFFI.tryHandle 分发回调，不初始化 Rust 动态库，不连接服务，不写用户配置。

- single peer response preserves other verified online peers：单 peer 回调只更新该 peer。
- empty callback does not turn verified online peers offline：空事件保持历史值；不代表网络失败集成已测试。
- address book replacement preserves verified state on load event：替换同一 RxList 后发送 load，验证旧在线状态和新元数据。
- metadata reload keeps unrelated peer visibly online：Obx 真正渲染模型列表，再替换并 pump，验证其他 peer 文本保持在线。

后两项预期暴露状态丢失，但未运行，不能计 RED。widget 为模型可见性测试，不是完整通讯录卡片或 Ab HTTP 端到端。实现前还需本地 HTTP/FFI fixture 实际驱动 Ab.pullAbImpl，覆盖成功提交、失败保留和异步期间新在线事件；不要只为让模拟替换通过而增加全局缓存或改写共享列表模型。

VM 独占测试窗口确认后，在隔离 guest 源码/SDK副本执行：

```powershell
flutter test test/peer_online_reload_test.dart --reporter expanded
```

应归档 stdout/stderr、退出码、实际用例数量、SDK版本、源码清单及摘要；基线 RED 必须是业务断言失败，不能是编译/初始化失败。当前只有历史 owner 文档，无法确认所有当前 agent 活动已结束；没有申请或占用 VM 窗口。主 agent owner 应先确认 Win10 当前任务和 0510/SOS 基线，再安排这组纯隔离 fixture。

## 协议与未闭验收

Rust 在线协议为请求 ID 顺序对应的 MSB-first 位图，不是返回 ID 列表。有效零位表示该请求 ID 离线；不足 ceil(ids/8) 字节报错；未请求 ID 不应更新。现有全局 callback 的 onlines/offlines 仅覆盖本次请求范围，不应把缺 ID 解释成离线。

尚未覆盖：超时/断连与 stale/unknown 表示、队列满重试、过期回调拒绝、完整 Ab HTTP fixture、真正卡片 widget、连接/关闭 session/焦点恢复的真实程序 driver、无需手动刷新的 E2E、Windows/macOS/Linux 运行与覆盖率。串行 Rust worker 是当前事实，但不能据此声称所有 UI/元数据异步响应都不会过期。暂无本轮 VM 日志、构建包或 GREEN。

## 源码身份

以下为 git hash-object 的 blob SHA-1，不是构建制品 SHA-256：

| 文件 | blob |
| --- | --- |
| flutter/lib/models/peer_model.dart | 3171b373a90f5c0e51a935f4a6f4e75bb0eb9dae |
| flutter/lib/models/ab_model.dart | a66aff810f4df407e1fa2d31a1f5382598c2ec97 |
| flutter/lib/common/widgets/peers_view.dart | 7d2f50c79cdfe0644cd732b7d79151d482a3b46b |
| src/client.rs | 833f7cd847001a442c1a66a8d5e64f6b17ae8207 |
| src/flutter.rs | 35fa8cc5ee1e9a4a43de269469fe7037902df056 |
| src/flutter_ffi.rs | 4ddab182ed22c31f5149856944c25a9ebbf849f4 |
| flutter/test/peer_online_reload_test.dart | a9dd3e5fccb5f088107ebed477375149cb61f47d |

回归面：本轮没有改变既有文件或运行路径。候选最小修复应优先限定于非 legacy Ab 成功替换列表时保留提交时刻的在线证据，失败不提交部分列表；须先有上述真实路径 RED 才落地。FFI 错误传播可能牵涉生成桥接，需要独立确认必要范围，不擅改共享接口。

AOCI：本仓库未发现 aoci.txt/.mcp.json，当前工具也未提供 aoci_rules/overview/maintain；没有伪造索引收据或写认知资产。
