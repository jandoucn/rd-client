# 通讯录真实会话 VM E2E 入口

## 当前入口（2026-10-17）

源码位于 `tests/fixtures/peer_online/`，正式 Flutter 包不包含这些依赖 integration_test 的入口。
从仓库根执行 `python tests/fixtures/peer_online/prepare.py <不存在的隔离目录>`，再进入该目录的 `flutter/` 执行 `flutter pub get --offline` 和 `flutter analyze --no-pub integration_test test_driver`。
下方 build/drive 的相对路径均指复制后的隔离目录；build 可在 host 执行，drive 和测试产物仅可在独占 VM 运行。复制脚本保留源码摘要，依赖注入只作用于副本。

## 依赖隔离纠正（优先于下方历史构建记录）

首次构建成功使用了正式目录临时加入的 integration_test / ffigen 9.0.1 及变更后的 lock，且未运行 VM。此记录仅证明测试入口可编译，不证明正式构建兼容。FRB v1.80.1 的 `frb_codegen/src/commands.rs` 明确检查 ffigen `>=8.0.0,<9.0.0`；`.github/workflows/flutter-tag.yml:137` 先 pub get 再执行该 codegen，没有 skip-deps-check。因此 ffigen 9 会被该链路拒绝，不能留在正式依赖。

已用 apply_patch 精确撤回正式 pubspec 中加入的 integration_test 和 ffigen 改动、逆转本轮工具生成 lock 差异；未整文件 checkout。正式两文件 git diff 为空，保留 `1.5.3+2026101501`、`ffigen ^8.0.2` 及其他原有内容。测试源码和 driver 留仓库，测试依赖仅在 TEMP 独立源码副本的 pubspec 注入；副本保留现有 generated_bridge 等真实源码，以逐文件 SHA-256 绑定。副本不重新生成桥接、不改变任何发布 workflow 或签名。下方原目录构建命令及依赖审计仅为历史，不应在正式目录重跑；后续测试 pub get/build/drive 均只能在隔离 fixture 中执行。

隔离 fixture 已实际构建成功：`C:\Users\Administrator\AppData\Local\Temp\opencode\peer-e2e-isolated-20261016`。首次长路径构建触发 MSBuild MAX_PATH 错误，保留 `long-path-build-failed/`；短盘符 T: 映射同一 TEMP 根后构建成功，70.9 秒、exit 0，未改变 CMake。使用完毕移除本轮 T: 映射；再次构建需重新映射同一目录，以匹配缓存路径。

该目录保存 `source-manifest.json`（217 个复制输入，摘要 `43136e5193d99dd787036e592b8dc57cd330a2793e814f8988f8cef0cf5e52ef`）、`fixture-build.log`、`fixture-build-exit.txt`、`artifact-manifest.json`（完整产物 91 文件及大小/SHA-256）。构建后重新验证 216 个源码/输入，README 因补写结果不作最终源码一致性断言；实际 app.main、全部 lib、runner、测试入口和旧 Rust DLL 均绑定原始摘要。fixture lock 由副本内 `pub get --offline` 生成，SHA-256 `4a8794d2a9b2d78275b9cefa7b1c68aa529da48861d8303d20b31c6450674524`。

正式 pubspec SHA-256 `e30a227d254a7481a77728c8f21e2403fc0a484eb2c230c9256ce979e7e7f18a`；正式 lock SHA-256 `c0100d3b0d22d22318a8bf58d8723270498cb2bc702890d42f3af7fd332233c8`。TEMP 根旁的 `prepare-peer-e2e-isolated.py`、`peer-e2e-short-path.py`、`verify-peer-e2e-fixture.py` 保留复制/注入、构建、复核步骤，准备脚本拒绝覆盖既有 fixture。副本中 ffigen 9 仅用于解决依赖，不用于 codegen；正式 FRB 1.80.1 继续使用 ffigen 8。源码依据：https://raw.githubusercontent.com/fzyzcjy/flutter_rust_bridge/v1.80.1/frb_codegen/src/commands.rs 。它通过 CLI `run ffigen --config` 生成 C 头对应 Dart FFI，检查声明和已安装版本；并非已证明 ffigen 9 API 兼容。

正式目录此前 pub get/build 留下的忽略项 `.dart_tool`、build 缓存仍是历史测试状态，不能视为正式发布验证；本轮没有在正式目录再次 pub get 以免重新改锁。现有正式流水线自身会 pub get 重建解析状态。本轮真实 E2E、跨 engine driver、VM 启动均未执行。

2026-10-16：入口 Windows Debug 编译成功，真实 E2E **未运行**。本轮没有启动 host 客户端、flutter test 或操作 Win10legacy/Win11shortcuts。既有 bugab_modelfix30green/profileP2closed 仅为模型层证据，不替代本测试。本目录是测试准备，外部 VM driver 尚未实现/验证，不能标记验收通过。

## 入口与断言

`peer_online_session_test.dart` 仅在主 engine 注册 IntegrationTestWidgetsFlutterBinding/testWidgets；`multi_window` 原样调用 `app.main(args)` 后返回，不注册测试、事件 handler 或替换 channel。主 engine 也通过原 `app.main(args)` 启动。

测试经现有通讯录 tooltip、AddressBookPeerCard 双击进入真实远控。只读真实个人非 legacy Ab，要求至少三个指定、互不相同 ID 全部出现在可见卡片中，至少两个实际 online。等待自然在线查询；连接、首帧握手、关闭期间持续核对指定 ID 状态。要求目标 Peer 对象经自然重载替换、hash 从旧值变为非空新值，并有对应服务端 hash 更新和后续 peers 请求审计。fixture 必须预置一个将发生 hash 变化的测试目标，已有相同 hash 会明确失败。

不注入 online 回调、不调用 pullAb/refresh、不拦截 sync_peer_hash_password_to_personal_ab。正常点击通讯录本身触发的产品加载路径保留。缺认证、第三设备、跨 engine driver、截图、审计或握手超时均失败，没有 skip 分支。

## 严格 VM 前置条件

外部启动器先核验 VM 身份、隔离 profile/配置、应用及 DLL 摘要、受保护 fixture ACL、测试账号授权、网络出口规则，再启动程序。**native runner 在 Dart 入口之前 LoadLibrary 和 rustdesk_core_main，因此 Dart 检查不能防止误启动 host 的 native 副作用。禁止在 host 启动此 EXE。**

测试读取运行时环境变量 `RUSTDESK_VM_E2E_MANIFEST` 指向的绝对 Windows 路径；不把 fixture 编入产物。代码检查 CIM manufacturer/model 的虚拟机特征及 manifest 的 name/manufacturer/model/uuid 精确匹配，检查 manifest ACL 禁继承且 Allow 仅当前 SID、SYSTEM、Administrators，检查有效期。VM 名称本身不是身份凭据。生产域名禁止访问必须由 VM 防火墙/隔离网络保证，不能把 prepare 回复当作防火墙证明。

manifest 结构（占位内容不能直接执行）：

```json
{
  "schema": 1,
  "expiresUtc": "<短期有效 UTC ISO8601>",
  "vm": {"name":"<CIM>","manufacturer":"<CIM>","model":"<CIM>","uuid":"<CIM>"},
  "api": "https://<隔离服务器>.test",
  "account": "<已登录的专用测试账号>",
  "abGuid": "<真实个人通讯录 GUID>",
  "peerIds": ["<连接目标>","<保持在线的第二设备>","<第三设备>"],
  "online": {"<连接目标>":true,"<保持在线的第二设备>":true,"<第三设备>":false},
  "isolationPolicyId": "<启动器已核验的网络隔离规则身份>",
  "evidenceDirectory": "C:\\<仅本轮主体可写的证据根目录>"
}
```

API 只允许 loopback 或 `.test` 域，当前客户端实际 API 必须完全相等。配置必须在 native 启动前由 VM fixture 预置；入口不写服务器地址或账号密码。证据目录和 fixture 父目录也应由启动器验证保护 ACL（入口只检查 manifest 本身）。测试账号凭据、远端密码留在 VM 受保护凭据源和原生认证 UI，禁止写入 manifest/仓库/握手/日志。请关闭其他测试客户端、过滤和排序干扰，保证三个卡片同时可见。

## 外部 VM driver 文件契约

主 engine 在证据根创建随机 128-bit run 子目录，写 `阶段.request.tmp` 后原子改名为 `阶段.request.json`。driver 用相同原子写法发布 `阶段.response.json`，所有响应必须回传 run，失败写 error。每阶段最长 45 秒，总测试 9 分钟，integrationDriver 10 分钟。旧目录响应不可复用。

1. `prepare`：请求含 run/pid；回复 capabilities 数组，必须包含 `native-window-auth`、`native-window-close`、`remote-pixel-challenge`、`native-client-capture`、`server-audit`、`isolated-egress`，以及与 manifest 一致的 isolationPolicyId。driver 此时建立从此客户端启动到会话关闭的独立服务端审计游标。
2. `connect`：请求含 peerId/startedUtc，测试随后双击卡片。driver 识别该 PID 的新增远控 HWND、与 desktop_multi_window windowId 映射、对应 peerId，在真实窗口处理密码对话框，回复 run/windowId。测试独立查询插件窗口列表确认窗口确实存在。不能通过 RPC 修改模型或代替点击连接。
3. `frame-0`、`frame-1`：各请求包含全新 64 随机 bits 和 windowId。driver 通过独立的被控端测试夹具在**远端桌面**显示 8×8 黑白矩阵；使用原生窗口捕获保存控制端该 HWND 的完整 PNG 为 `frame-N.png`，回复 run/windowId/x/y/cell。x/y 是矩阵左上角，cell 为整数像素边长 ≥4。测试解码 PNG，逐格中心像素检查 RGB 黑<35、白>220。两轮变化证明收到动态远端画面，不能以窗口出现、peer_info、Flutter 首 UI frame、授权成功或 `passed=true` 代替。
4. `close`：请求 windowId，driver 通过真实窗口关闭操作并恢复主窗口焦点，再回复 run；测试再次查询插件窗口列表确认 windowId 消失，继续观察主卡片状态 5 秒。如果产品保留窗口池导致该 ID 仍存在，本入口会失败，需提供真实 session 生命周期观测方案后另行评审，不能直接放松成成功。
5. `audit`：关闭后 driver 导出受保护测试 API 的只读审计为 audit.response.json，包含 run/api/records。更新记录要求 method=POST、path=`/api/ab/peer/update/<GUID>`、peerId、changedFields 包含 hash、status=200、utc；随后需 POST `/api/ab/peers`、ab=<GUID>、status=200、utc。必须筛选本客户端/测试账号/本轮请求关联，服务器与 VM 时钟同步；保留原始 request-id/关联 ID 和审计导出摘要供复核，不能用 driver 自编“成功记录”。不得导出 hash/token/password。

截图与审计的来源仍是受信任外部 driver 的责任：文件协议本身无法证明 HWND 捕获来源、服务端日志真实性。必须归档 PID/HWND/windowId/peerId 映射、远端挑战显示记录、原始截图、审计游标与 request-id；禁止根据 bits 在本地合成截图。当前未交付这个 driver，故本入口编译成功不是首帧或会话 E2E 成功。不增加生产 seam 或泛化框架。

## 构建及 VM 执行准备

本轮 host 使用 `C:\Users\Administrator\AppData\Local\Temp\opencode\flutter-3.24.5`（Dart 3.5.4）：

```powershell
flutter pub get
flutter build windows --debug --no-pub --target integration_test/peer_online_session_test.dart
```

最终构建成功耗时 18.6 秒；未执行 host analyze/test/drive/EXE。以下仅供取得 VM 独占窗口并完成上述启动器门禁后执行，本轮未执行：

```powershell
$env:RUSTDESK_VM_E2E_MANIFEST = '<VM受保护manifest绝对路径>'
flutter drive -d windows --debug --no-pub --driver test_driver/integration_test.dart --target integration_test/peer_online_session_test.dart
```

该命令会在 VM 构建/启动；若使用 host 产物，应由 VM 启动器启动已核验产物并让 driver 附着其 Dart VM service，明确选择主 isolate，不能重新把 production target 编译覆盖测试 target。附着链尚未验证。driver 入口参数已对照本机 SDK `integration_test_driver.dart` 的真实签名；未单独运行 driver。

## 本轮产物 manifest

目录 `flutter/build/windows/x64/runner/Debug/`，必须带完整 `data/`（含 icudtl.dat、flutter_assets/kernel_blob.bin、所有 assets/fonts/shader 等），不要只复制 EXE。以下为最终构建关键文件 SHA-256，重编译后需重新生成；完整目录级归档 manifest 尚待 VM owner 打包前生成，当前不是可发布包。

| 文件 | SHA-256 |
| --- | --- |
| rustdesk.exe | 28234279d439588ec8eca1f517db35a28627d4aa3aa94e4c7407553245892bee |
| data/flutter_assets/kernel_blob.bin | 4c373a531fd17bdd5bf74db1bca9700c70e3bceae01c27699cf6bfe16277f5fa |
| librustdesk.dll | ee8353c95caaf4aa71a7255d14045c9e13f5c53e2a2d7b10687b9bd22bb05702 |
| flutter_windows.dll | ad5faf227ea307562db2f95ddc62264c7714454d26b35cec975bbb2a7ed86b4b |
| desktop_drop_plugin.dll | 7b4e2d08a3d231ee6bf01963b7a4e4daed813f086301d2def840a48d4c30a180 |
| desktop_multi_window_plugin.dll | 6d6efd322c6882937f707e71b456e57310f8c07c617d55515b443b3be4f9774a |
| file_selector_windows_plugin.dll | 328700d03a172c0ea93fed6ff0412b919f0369369c1fa06cde27807958489747 |
| flutter_custom_cursor_plugin.dll | 326b55d557b62d8fc844f69a5a33d3d99ce78d51451304bbf6ebd920f9e1e5df |
| flutter_gpu_texture_renderer_plugin.dll | 2f597114bed8c310ae22073b6990249bf965d241c0725712f7d0beb20d24d3e2 |
| screen_retriever_plugin.dll | cae36523d4e77cac7f9fcb996c2aca442f61fc922b52cc07a5a7b6ffa2403863 |
| texture_rgba_renderer_plugin.dll | 811094766b4a357722734c54f0860773d434b0aeeb95d7b952d20c0b1bf5d9f8 |
| uni_links_desktop_plugin.dll | 16b2dd165d7a9690ad696ef3edf689f97e778d7446befdb89d2caa98c8e0c8f3 |
| url_launcher_windows_plugin.dll | 3e159104a01f4cbd00c49b63cb65ef03410221aba2bdaa9247c7f92c46ce3770 |
| window_manager_plugin.dll | 4c68ab6b0d2d6f65b4eec22f8d1db83d31315c9897e2b599c02125eabf67f86d |
| window_size_plugin.dll | 86d2bfd203366679bd7b54311bb59eca5e24d2be4f0424a8826045e71adee78e |

旧 Rust DLL 精确来源是 `target/debug/librustdesk.dll`，与输出文件摘要相同。本轮没有重建 Rust，不能声称此 DLL 对应当前修改中的 Rust 源码，也不能任意替换成 0510 安装版 DLL。其桥接 ABI、真实会话能力与 VM 运行兼容性待测。

`dumpbin /dependents` 已核对：EXE 静态依赖上述 11 个插件及 flutter_windows.dll；librustdesk.dll 由 runner 动态 LoadLibrary 加载。旧 Rust DLL 的导入为 Windows 系统库 KERNEL32/ADVAPI32/newdev/WTSAPI32/oleaut32/user32/shell32/ole32/api-ms-win-core-synch-l1-2-0/userenv/bcryptprimitives/gdi32/setupapi/cfgmgr32/iphlpapi/ntdll。Flutter 引擎另依赖 Windows 图形、网络、输入和 UIAutomation 系统库，含 api-ms-win-core-path-l1-1-0.dll，延迟加载 dwmapi.dll/dbghelp.dll。这些系统 DLL 不从 host 复制到 VM。插件传递导入和运行时动态依赖尚未全扫描，需在 VM 启动前补齐；不能宣称仅这些文件足以在所有 Windows 启动。

## 历史：首次共享依赖变更审计（已撤回）

仅新增 SDK integration_test dev dependency；ffigen 8.0.2 的 file ^6 与 SDK 固定 file 7 冲突，改为精确 9.0.1。锁文件由 SDK `pub get` 生成，没有 pub upgrade。新增 12 项：fake_async、flutter_driver、flutter_test、fuchsia_remote_debug_protocol、integration_test、leak_tracker、leak_tracker_flutter_testing、leak_tracker_testing、process、sync_http、vm_service、webdriver。SDK 固定降级 async、boolean_selector、matcher、platform、source_span、stack_trace、stream_channel、string_scanner、term_glyph、test_api；升级仅 ffigen/file。vector_math 版本未变，只修正既有 direct main 归属。flutter_test 原来已在 pubspec，旧 lock 漏项也由工具补齐。

上述共享依赖变更已撤回，仅存在于 TEMP fixture 的独立依赖图中。最终仓库交付仅两个测试入口和本说明，既有生产源码、正式 pubspec/lock、workflow、签名逻辑无本轮额外差异。未执行真实 E2E/负例矩阵、跨 engine 实测、Win10/Win11 启动、Linux/macOS、80% 覆盖率。AOCI 工具和项目 .mcp.json 不可用，未生成认知收据或改动托管资产。
