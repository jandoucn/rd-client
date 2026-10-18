# Android 虚拟鼠标默认与选项换行验收

基线 master 76ace22d02f1edc2af515ec62adc08d11d7f6167，2026-10-18。本轮唯一实施/Win10 VM runner，无子代理，无 commit/push/tag/build；原 Windows Agent、AGENTS 及历史 docs diff 保留。

## 行为及回归面

- flutter/lib/models/model.dart：showVirtualMouseFromStoredOption 仅 Y 为 true，移除 Android 空配置自动开启；已有 Y/N 尊重，非 Android 本来就是仅 Y true。VirtualMouseMode 初始 false，未发现模式切换强制开启调用。floating_mouse.dart 和 floating_mouse_widgets.dart 均读取此 showVirtualMouse，false 时不显示；本轮不改变这些组件。
- flutter/lib/mobile/widgets/gesture_help.dart：仅 Android 三项以 Wrap 排列，顺序虚拟鼠标→移动→缩放。LayoutBuilder 读取父逻辑约束，IntrinsicWidth 保持每项自然宽度，最大宽度+Flexible 允许长标签折行；Checkbox 和完整标签可点击，MergeSemantics 保留选中、启用、聚焦与 tap 语义。非 Android 保留旧虚拟鼠标布局。没有新翻译 key、runtime 算法或 move/scale 默认变化。
- flutter/test/yan_standard_ui_policy_test.dart：已有默认开启测试改为默认关闭，保留显式 Y/N 与其他平台断言。
- 新 flutter/test/android_canvas_options_layout_test.dart：10 项，Android/非 Android 空/Y/N、两模式宽1600逻辑像素同排及窄320逻辑像素换行；窄测试长翻译、textScale2，MediaQuery 宽1600而实际父约束320；以真实文字矩形验证顺序和边界。帮助页采用与 remote_page.dart 相同的 SingleChildScrollView，验证无 overflow、真实点击和 checkbox 语义。

## VM TDD

预检 Win10 无 dart/flutter 活动进程。既有普通 rdlab guestcontrol / peer-reload-control.py；无 UAC、产品服务操作。Flutter 3.24.5，C:/RustDeskAgentLab/peer-reload-kit/sdk/bin/flutter.bat，独立 project 副本，--no-version-check test --no-pub --reporter=json --concurrency=1，无宿主 Flutter test/pub upgrade。

| run（目录前缀 peer-reload-） | pass | fail | skip | raw exit |
| --- | ---: | ---: | ---: | ---: |
| layout-red-20261018a | 5 | 5 | 0 | 1 |
| layout-green-20261018d | 137 | 0 | 0 | 0 |

RED：Android 空配置 true、两模式宽屏未同排、窄屏长标签 overflow。中间迭代纠正自然宽度、测试滚动容器和 SemanticsHandle 释放时机，不作通过证据。最终完成 UTC 2026-10-18T02:50:01.8857145Z。

完整七个 suite：android_canvas_options_layout_test、android_canvas_scale_widget_test、android_canvas_option_widget_test、android_touch_pointer_widget_test、android_two_finger_wheel_test、android_gesture_help_widget_test、yan_standard_ui_policy_test（均 .dart）。保留原106项+新增10项+既有策略21项=137，包含原持久化失败测试。

宿主证据目录 C:/Users/Administrator/AppData/Local/Temp/opencode：layout-red-a.jsonl、layout-green-d.jsonl、layout-hashes.json。VM 原 run 目录保留 result.json、tests.jsonl、stderr.log。

## SHA-256

以下四个修改文件宿主与 VM 全部匹配，依赖未更新：

| flutter/ 下文件 | SHA-256 |
| --- | --- |
| lib/models/model.dart | 6848826ec9ba80a60f0a419f3087a5a3be2c9f049d5500b2cbfc8402753e8c11 |
| lib/mobile/widgets/gesture_help.dart | 9dd13eb213c5f6ca87799d1b9f8212d3dd49416f221ccce505c6b16f49950f19 |
| test/yan_standard_ui_policy_test.dart | 2b36c7546b57f8b3c789cc64393f2fec27a201962d53528ce696cb6347dbc525 |
| test/android_canvas_options_layout_test.dart | da45f783ea0b18119a0c2a68adb414f9a16f638b924e8973af2207bcf3912dc9 |
| VM pubspec.lock | 6e7b177de0ece988c85888666330526d4dc60883076853366062ccb191bed501 |
| VM package_config.json | 44e6df58253032524f26fe15fe5778b21a2dfdb979014cfb32ce5f39ec2034da |

## 验收边界

实际 Win10 VM widget/state/raw-pointer 回归通过；没有 Android 真机、旋转设备实测、产品桌面驱动、新包或 FFI 落盘验收。可聚焦/语义经测试，键盘真实按键切换未单独实测。浮动鼠标与状态关联经源码核对，未声称真机 cursor E2E。未测分支覆盖率或全 cargo/clippy/Flutter analyze。已有 scale 开启时误缩放 OPEN 不受本轮布局修改影响。

git diff --check 通过。AOCI 工具未挂载，未伪造认知收据或执行维护。主后续独立审查。
