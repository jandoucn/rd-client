# 双指移动画布开关验收（2026-10-18）

基线 HEAD：e3d517e4637a0c0e74c776f1adb54af1d4aebfa0，master。唯一实施及 Win10 VM 执行负责人；未派生子代理、提交、推送或触发发布构建。

## 行为与回归面

- `flutter/lib/mobile/widgets/gesture_help.dart`：Android 两种输入模式的“显示虚拟鼠标”紧邻下方增加“双指移动画布”；帮助卡片读取同一会话选项，关闭时不显示画布移动卡。平台/翻译参数默认沿用生产行为，用于真实 widget 测试注入。
- `flutter/lib/models/model.dart`：仅 VirtualMouseMode 新增此选项及加载/切换方法。与虚拟鼠标一致采用 local option，本机持久化、会话加载；两模式共用，初始及缺失/非法值均 false，只有 Y 开启。切模式不修改它；已开启后切模式保持开启。写入失败保留原状态和旧存储值。
- `flutter/lib/common/widgets/remote_input.dart`：仅 Android 非 camera 的既有 canvas pan 门禁读取新选项。wheel 优先及 updateScale 保留；hold、camera、iOS、desktop 分支不改。
- `flutter/lib/consts.dart`、`libs/base/src/config/keys.rs`：新增 two-finger-canvas-movement，并加入客户端 local settings keys；未修改 hbb_common。
- `src/lang/`：新 key `Two-finger canvas movement`，cn/tw 有翻译，其余语言及 template 空值，en 使用 key fallback。既有译文未改。
- 测试：扩展 android_touch_pointer_widget_test.dart，新增 android_canvas_option_widget_test.dart；保留本轮接手的 help 测试更新及全部既有 71 项。

## VM 证据

VM：RDLAB-WIN10（a6c9edff-6159-468f-bdba-08720a153424）。普通 rdlab guestcontrol，经既有 peer-reload-control.py；没有 UAC 或产品 service 操作。执行前未发现 dart/flutter 进程；结束后仍无残留。遇到 guest session 数量上限后，仅关闭已查询为 successfully terminated 的会话 27–31。

SDK：`C:/RustDeskAgentLab/peer-reload-kit/sdk/bin/flutter.bat`。
独立测试副本：`C:/RustDeskAgentLab/peer-reload-kit/project`。
参数：`--no-version-check test --no-pub --reporter=json --concurrency=1`。

| run（VM 目录前缀 peer-reload-） | 通过 | 失败 | 原始退出码 |
| --- | ---: | ---: | ---: |
| canvas-option-red-20261018b | 42 | 2 | 1 |
| canvas-ui-red-20261018c | 0 | 2 | 1 |
| canvas-full-green-20261018c | 85 | 0 | 0 |

raw-pointer RED 为两模式开启后预期 pan 非空、实际为空；UI RED 为两模式预期新 checkbox 文案存在、实际不存在。此前编译失败的 run 不计业务 RED。最终 GREEN 包含 pointer 46、wheel 23、help 6、option 10；旧 71 项加新增 14 项。

最后完成时间 UTC：2026-10-18T00:45:32.4142059Z。JSONL、原始退出码及摘要汇总存于宿主 `C:/Users/Administrator/AppData/Local/Temp/opencode/` 的 `canvas-final-evidence.json`、`canvas-option-red-20261018b.jsonl`、`canvas-ui-red-20261018c.jsonl`、`canvas-full-green-20261018c.jsonl`；VM 原始各 run 目录保留 result.json/tests.jsonl/stderr.log。

最终九个 Dart 源码/测试文件的宿主和 VM SHA-256 全部匹配。主要源码摘要：

| 文件（flutter/ 下） | SHA-256 |
| --- | --- |
| lib/common/widgets/remote_input.dart | a396ef0643d15486e0c32660832020a8211954ee4f3393c9c33a0ff4f74d3755 |
| lib/mobile/widgets/gesture_help.dart | 84e430392d44fc779db13203db973d16e4f31181200983a2f805f73e947eb2ef |
| lib/models/model.dart | 7a997c58a1692f15f4bf97cb7869e24cef5a0ea696196e9f409724b7ba3c4379 |
| lib/consts.dart | 9e862595677f366a2a99ce20aad2eca93cee13fa389a5973847d10f266709b7c |
| test/android_canvas_option_widget_test.dart | eb0485528cf53b2cc12cb4f31d309d04dcb530de4a87d154295f26be2f7f5d97 |

## 已知边界与未验收项

关闭只控制 panX/Y。跨帧逐指 move 引发 scale 改变时，updateScale 焦点仍可能移动；不能声称误缩放已修复。原 OPEN 证据 `C:/Users/Administrator/AppData/Local/Temp/opencode/android-pointer-open-20261018.zh.md` 保留。gestures.dart 经 git diff 确认与 HEAD 无内容差异，没有引入计时器或新识别状态。

本轮是 Win10 VM 中实际 Flutter widget/raw-pointer 测试，不是 Android 真机或 Windows 产品窗口 E2E。尚无本轮构建产物，且本轮未获新构建指令，未执行桌面 integration_test/产品驱动、Android 实机、iOS 或 Linux/macOS 实机验收；非 Android 路径仅沿用既有测试覆盖。持久化测试替换存储边界，验证实际模型读写方法的状态语义，未验证 Rust FFI 落盘。未测分支覆盖率，不能宣称达到 80%。未运行宿主 Flutter 测试、全工作区 cargo 测试/clippy、全 Flutter analyze。

最小 diff 复审：共享 model 只在 VirtualMouseMode 添加选项；remote_input 只改门禁；其他 Windows/API/Agent 既有未提交 diff 未改。git diff --check 通过。AOCI 工具未挂载，无法执行维护，未创建认知收据。
