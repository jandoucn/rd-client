# 双指画布缩放独立开关验收（2026-10-18）

基线：master，6c24fe4f3fa6ed226ec2fa5a0e60144ac889a2c6。上一轮移动开关的发布记录不改。本轮单一实施及 Win10 VM 测试负责人，无子代理、commit/push/tag/build。

## 行为

Android 鼠标/触屏模式均增加紧邻“双指移动画布”下方的“双指缩放画布”。两个选项采用同一既有 VirtualMouseMode/local option 模式，但分别存储：two-finger-canvas-movement、two-finger-canvas-scaling。初始/缺失/非法值均 false，仅 Y 开启；模式切换保留共享状态。已有 move=Y 不因新 scale 默认值改变。

| move | scale | Android 普通双指画布路径 |
| --- | --- | --- |
| false | false | 不调用 panX/Y 或 updateScale |
| true | false | 允许 pan，不调用 updateScale |
| false | true | 允许原 pinch/updateScale，不调用 pan |
| true | true | 保留原缩放及移动 |

wheel 优先：scale=false 时跨度变化不再阻断纵向 wheel；原 dy/8 积分规则未改。scale=true 保留原 5% 跨度判断。hold、camera、非 Android 和 desktop 原路径保持。Android Canvas Zoom 卡随会话 scale 状态，非 Android 卡不受影响。

## 修改与回归面

- flutter/lib/common/widgets/remote_input.dart：仅跨度判断和移动端 updateScale 调用增加 scale 门禁；pan 独立。
- flutter/lib/models/model.dart：仅 VirtualMouseMode 增加独立读写、加载和切换方法，成功写入后读取状态，失败保留旧值。
- flutter/lib/mobile/widgets/gesture_help.dart：相邻 checkbox 及帮助卡门禁。
- flutter/lib/consts.dart、libs/base/src/config/keys.rs：新增 key 和客户端 local settings 注册，未触 hbb_common。
- src/lang/ 下 53 个文件各追加一条；cn 为“双指缩放画布”，tw 为“雙指縮放畫布”，其余空，en 使用 key fallback；既有翻译不变。
- 保留原 85 项：android_touch_pointer_widget_test.dart 原 pinch 用例明确开启 scale；android_gesture_help_widget_test.dart 原 zoom 卡用例明确开启。新增 android_canvas_scale_widget_test.dart 21 项。
- gestures.dart、wheel helper、旧 Windows/API/Agent/AGENTS diff 未修改；git diff --check 通过。

## 实际 VM TDD

VM RDLAB-WIN10（a6c9edff-6159-468f-bdba-08720a153424）从 saved 恢复后预检无活动 dart/flutter 测试进程。使用既有 peer-reload-control.py、普通 rdlab、process ExecutionPolicy Bypass；无 UAC/产品 service 操作。

SDK C:/RustDeskAgentLab/peer-reload-kit/sdk/bin/flutter.bat，版本 3.24.5；独立 project 副本。参数 --no-version-check test --no-pub --reporter=json --concurrency=1。未在宿主运行 Flutter test。

| run（VM 目录前缀 peer-reload-） | pass | fail | skip | raw exit |
| --- | ---: | ---: | ---: | ---: |
| scale-red-20261018a | 0 | 4 | 0 | 1 |
| scale-green-20261018d | 106 | 0 | 0 | 0 |

RED 两模式实测 updateScale 值 [0.6,1.0,1.0]，违反默认关闭；另两项缺少 checkbox。GREEN 完成时间 2026-10-18T01:46:55.6587744Z，包含原85及新增21。新用例覆盖四组合、默认关闭、跨帧逐指 wheel/scale、真实 CanvasModel x/y/scale 逐帧不变、开启后真实缩放正向、UI位置/帮助/切模式、缺失非法配置、持久化重载和写入失败保留两项旧值。

中间 scale-green-a 为测试 SessionID const 编译错误；b 为测试 UUID 格式错误；修正夹具后 c/d 通过。不把这些失败计作业务 RED。

最终回执取证时外部 VM online snapshotting 导致查询超时，未重跑测试；恢复 running 后取回原 run JSONL，并独立短脚本提取源码 SHA。未宣称超时汇总脚本成功。宿主证据目录 C:/Users/Administrator/AppData/Local/Temp/opencode/：scale-red-20261018a.jsonl、scale-green-20261018d.jsonl、scale-hashes.json；VM 原目录保留 tests.jsonl/result.json/stderr.log。

## SHA-256

8 个相关 Dart 文件宿主与 VM 全部一致；完整集合见 scale-hashes.json。

| 对象 | SHA-256 |
| --- | --- |
| remote_input.dart | 16fda8f362347834ba19612c496ccb242576477fc96abbbbcf8d2ec20aeeeeb6 |
| gesture_help.dart | 7e0f219e9542b79c23bb807ad38db67ce1e656a089f015764321c3ef612b8bb1 |
| model.dart | 521c2902d2feedffefe8beaa325bf085ccb45600fe4a9a66f6399bfbb685d68d |
| consts.dart | a821596234db6f742ba176415814ab6c54840cb1c4ca27ac2ba7d0e8be884dd5 |
| android_canvas_scale_widget_test.dart | 2900799cf91d77468b86dce0c124151ba52e40caf95ae3b75bbc8a6db65fe0ea |
| VM pubspec.lock | 6e7b177de0ece988c85888666330526d4dc60883076853366062ccb191bed501 |
| VM .dart_tool/package_config.json | 44e6df58253032524f26fe15fe5778b21a2dfdb979014cfb32ce5f39ec2034da |
| SDK version 文件 | 99b7c4bf3cf005b4f726740dbe3b616d7326cd9bac1ac4ac88216ae9c714441a |

## 边界

关闭缩放阻断了本轮测试中的 updateScale 焦点漂移；显式开启缩放仍使用原识别，跨帧误缩放 OPEN 不宣称全修，原 android-pointer-open-20261018.zh.md 保留。

本轮为实际 Win10 VM Flutter widget/raw-pointer/真实 CanvasModel 测试；未执行 Android/iOS 真机、桌面产品驱动或新包验收，无本轮构建产物，且用户明确不 build。持久化通过存储边界替身验证，未验证 Rust FFI 实际磁盘写入；未运行完整 cargo/clippy/Flutter analyze 或统计分支覆盖率，不能宣称80%。AOCI 工具未挂载，未维护或伪造认知收据。独立审查由主后续安排。
