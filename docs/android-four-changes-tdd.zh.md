# Android 四项修改 TDD

状态：四项源码修改完成；TDD 1/3 手势及 TDD 2 文案已在 Win10 VM 完成 RED→GREEN，合并回归 69/69 通过；TDD 4 资源配置静态校验通过。独立源码复审关闭；Android APK 构建、实际设备和覆盖安装尚未验收。

本文件按用户最新要求取代原 TDD 执行队列。原 Windows、macOS、API、Agent、DNS 恢复等任务停止推进，已有代码及证据保留，不据此回滚。此次仅修改以下四项 Android 行为，不自动提交、发布或触发构建。

## 范围及现状

1. `flutter/lib/common/widgets/remote_input.dart` 在双指未进入 wheel 分支后，会调用 `canvasModel.updateScale/panX/panY`，Android 鼠标模式因而仍会平移画布。
2. `flutter/lib/mobile/widgets/gesture_help.dart` 的鼠标模式仍显示“三指垂直滑动”和“双指移动／移动画布”。
3. `flutter/lib/common/widgets/gestures.dart` 的 Android 双指 wheel 按 `delta.dy / 4` 累积，严格超过 ±1 后发一步并清零。
4. Android Manifest application label 和 `res/values/strings.xml` 的 app_name 均为 `RustDesk Yan`；applicationId 为 `com.carriez.flutter_hbb`。

共享 Dart 文件中的变更必须只在本地控制端 Android 生效。摄像头页面、特殊按键按住拖动、单指鼠标移动、物理鼠标、iOS 和桌面保持原行为。用户仅要求禁用画布平移，没有要求禁用捏合缩放；因此 Android 鼠标模式保留缩放，但双指输入不能再调用画布 panX/panY。触屏模式沿用当前已实现的两指滚轮及原画布分流，仅应用本轮减速调整。

## TDD-1：鼠标模式禁用双指画布平移

- [ ] RED：用真实 `RawTouchGestureDetectorRegion` 和两枚 `TestPointer`，Android mouse mode 水平/斜向滑动应不调用 panX/panY，现实现可复现平移。
- [ ] 最小实现：Android、非 camera、mouse mode 的双指画布分支阻止 panX/panY；不能直接禁用所有双指回调。
- [ ] GREEN：垂直双指仍产生 wheel；水平双指不滚动、不平移；pinch 仍缩放且不平移。两指横排、竖排、斜排都覆盖。
- [ ] 回归：触屏模式原画布分流、camera、hold-drag、单指、iOS/desktop、pointer cancel 与指针数变化。

## TDD-2：手势提示与行为一致

- [ ] RED：Android 鼠标模式帮助卡应显示“两指垂直滑动 → 鼠标滚轮”，不应出现“三指垂直滑动”或“双指移动 → 移动画布”。
- [ ] 最小实现：修正文案及对应双指图标，移除被禁用的鼠标模式平移提示；保留缩放提示。
- [ ] GREEN：通过 widget 文本和卡片配对断言验证，而非源码 contains 检查。
- [ ] Android 触屏模式中若仍显示三指 wheel 提示，按既有双指行为同步校正，不改变触屏手势；非 Android 帮助保持原样。
- [ ] 文案走已有翻译机制；若需新增 key，遵守语言文件规则，不能改已有非空翻译或意大利语手工翻译。

## TDD-3：两指滚轮减速

初始调节方案：累积除数从 4 改为 8，约将单位滑动距离的滚动灵敏度减半；这是待验证的默认方案，不保证所有帧率下 tick 数恰好减半。保持原方向、严格阈值和单次更新最多一步规则，不修改物理鼠标滚轮。

- [ ] RED：同一小步位移序列在旧实现过早发 tick；新预期下绝对累计位移到 8 不发送，超过 8 才发送一步。
- [ ] GREEN：两模式正/反方向、连续小步、反向抵消、大位移最多一步、边界清零；逐帧累计不能在手势取消或下一次手势中泄漏。
- [ ] 比较固定距离及相同事件序列的新旧 tick 数，确认减速而非丢失全部 wheel。
- [ ] Android 实际设备中确认滚动手感；若需再调，仅改该局部常量，不引入新加速策略。

## TDD-4：应用显示名 RustDesk

- [ ] 修改 Android application label 与 app_name，使用 `RustDesk`；优先统一由资源声明，避免两处显示不同。
- [ ] 校验合并 Manifest 和实际 APK 的 application/launcher label 为 RustDesk，桌面和系统应用信息均不带 Yan。
- [ ] applicationId 继续为 `com.carriez.flutter_hbb`，provider authorities 和其他组件身份不改；不改任何签名、keystore、macOS 配置。
- [ ] 新 APK 使用固定长期证书，SHA-256 为 `71:11:D7:30:EC:A2:C7:98:47:41:53:04:59:DD:38:97:20:16:2F:7B:FE:B7:39:FA:96:8F:2E:D7:4E:6F:0B:D8`。
- [ ] 后续构建使用更高 versionCode；在已有正式 APK 上覆盖安装，检查包名、证书、配置和应用数据保留，不先卸载。

显示名本身不是 Android 覆盖升级身份；能否覆盖取决于 package/applicationId、签名及系统版本/安装策略。名称变更不会自动更换上述身份，但仍需真实覆盖安装确认。

## 验证与交付

- 行为用例先 RED 后最小修复再 GREEN；名称/文案静态校验仅证明配置，不代替手势 widget 或 APK 验收。
- 客户端测试按项目约束在 VM 执行，使用 `--no-pub` 和独立测试副本；禁止为了本轮变更正式依赖锁。
- 每轮记录实际命令、用例数、退出码、源码摘要；零测试或前置失败不计 RED/GREEN。
- 只运行四项相关测试及必要回归；不恢复旧全量 TDD 队列，不累计不同版本的通过数。
- 提交前检查最小 diff 和 Android-only 门禁。APK/设备或覆盖升级未验证时明确列出，不把编译或模拟平台测试当作真机通过。

## 实际执行回执

| 阶段 | 实际执行 | 通过/失败/跳过 | 原始退出码 |
| --- | ---: | --- | ---: |
| 手势 RED | 63 | 45 / 18 / 0 | 1 |
| 手势 GREEN | 63 | 63 / 0 / 0 | 0 |
| 文案 RED | 6 | 3 / 3 / 0 | 1 |
| 文案 GREEN | 6 | 6 / 0 / 0 | 0 |
| 合并回归 | 69 | 69 / 0 / 0 | 0 |

测试运行于 RDLAB-WIN10、Flutter 3.24.5，使用 `--no-pub --concurrency=1`。63 和 6 已包含于最终 69，不重复累计。源码与依赖文件运行前后摘要一致。原始 JSON 事件、逐 case 结果、源码摘要及 raw exit 保存在宿主 TEMP/opencode 的 `android-four-13-20261018-red4`、`android-four-13-20261018-green1`、`android-four-13-20261018-help-red1`、`android-four-13-20261018-help-green1`、`android-four-13-20261018-combined-green1`。

首次两次尝试分别为命令行长度超限和脚本执行策略阻塞，实际测试数为零，不计业务 RED；后续使用正常进程级脚本参数执行，没有修改系统执行策略。

实际改动：wheel 积分 `/4 → /8`；Android 非 camera 鼠标模式完成缩放后阻止 panX/panY；帮助卡片改为双指垂直说明，Android 鼠标模式隐藏 Canvas Move；新翻译 key 按规则追加到 53 个语言文件，原翻译保留；application label 引用 `@string/app_name`，值为 RustDesk。

应用名已在宿主及 VM 对源 XML 做只读解析检查：launcher 无覆盖标签，applicationId/manifest package 保持 `com.carriez.flutter_hbb`，provider authorities 和签名配置未改。没有安装 APK，因此不声称已确认桌面最终 label、实体签名或覆盖升级成功。当前无在线 Android 设备，SDK 缺少 NDK 及项目所需 Android 36 平台；本轮未安装依赖或触发发布。

复审结论：指定四点无阻塞性源码 finding。测试中的原始指针经过真实 Flutter widget/recognizer，但 wheel 接收端为 fake InputModel，不等于原生传输或远端实际滚动。真机手感、APK/覆盖安装及分支覆盖率仍待验证。
