# Android 独立线当前候选与 RED 交接

本轮仅源码核验和测试准备，未运行客户端测试、VM、CI、构建或发布。Windows 主线与 T01–T12 全部保留。当前业务源码是修复前候选，不是 GREEN。以下修复须在主统一 Win10 窗口取得有效业务 RED 后实施并重冻。

## A01 双指垂直 wheel

当前问题：remote_input.dart 的 onTwoFingerScaleStart 参数是 ScaleStartDetails，却读取 d.scale；此编译问题必须先单独消除，不能计作业务 RED。最小编译候选为删除新增的 `_scale = d.scale`，保持原 start 路径。其余业务保持当前版本用于 RED。

`abs(d.scale - _scale) > 0.001` 会把逐指派发的平移中间帧当捏合；当进入画布路径后又更新 _scale，下一帧容易继续误判。不能只用更大的任意常数掩盖。最小行为候选限于 Android 非 camera、非 hold 的双指路径：利用当前指针移动/跨度证据判别纵向平移和明确捏合；边界帧只重置、不发 wheel；鼠标及触屏共享相同纵向 wheel 判定。具体判别须通过下列原始指针用例后确定，不新增计时器、缓存或手势超时状态机。

RED/回归清单：

- 两模式分别上/下滑，逐指派发 30px、20px；wheel 非空且方向正确，canvas scale/pan 均零调用。现有鼠标上下用例及触屏向下用例已具备；触屏向上仍待补。
- 两指间距100px，逐指纵向移动导致中间 scale 超过0.001仍应滚动；需补1px横向抖动版本。
- 明确捏合及水平平移仍走画布、不发 wheel；已有旧画布测试改为先水平移动，避免把合法纵向 wheel 当失败。
- cancel/up 后剩余单指不得追加 wheel；重新双指首帧不得携带旧积分；arena reject 后等待超过200ms不得恢复旧手势。后两项待补，尤其检查 debounce timer 生命周期，不借此重写共享识别器。
- camera 两模式保持画布；hold优先；iOS双指画布/三指wheel；桌面原路径。现有 camera 仅鼠标模式，触屏 camera 与桌面回归待补。
- helper旧 `touch` gate 用例改为水平输入，保留抑制/积分清空检查；已有触屏垂直正向用例保留。

## A02 前台恢复

当前 `_retry()` 在调用 onRetry 前清失败并消费资格，false 后永久丢失该次资格。最小候选：先调用同步 onRetry；仅 true 时消费并清失败，false 保留资格并返回 false，让当前错误继续展示。保持 manual retry 的 retryStarted 在实际重连入口消费资格，避免 resume/late DNS 再触发；不添加 in-flight 状态。

closeAndroidConnectionResume 开头增加 `_usesAndroidConnectionResume` gate，之后保持 controller/expected 身份核验与原定时器清理。不能只依赖调用点的 isAndroid。clear/mobileReset 现有 tombstone 策略不重构。

本轮新增待执行测试：

- policy：首次 onRetry=false，第二次=true，第三次不再尝试；返回值依次 false/true/false，attempts为1/2/2。
- model：非Android model调用close hook，即使存在controller也不关闭。此用例只验证平台gate，不足以证明真实timer保持。

保留既有 manual model reconnect 消费、close intent、旧listener/旧页面、peer变更、认证UI、healthy权限和重启路径测试。仍须补真实普通timer与restart timer触发/取消的行为验证；不能把替身reconnect计数称作native重连成功。

## 主处理需要修改的文档段落

- docs/agent-recovery-tdd-plan-20261016.zh.md 的 A01 标题/目标（当前约198–200行）改为“两模式双指垂直wheel；明确捏合/水平平移/camera保留画布”；202–203行勾选不能代表当前冻结通过，应注明历史证据及当前待VM验证。
- docs/agent-tdd-execution-ledger-20261016.zh.md 的 A02 当前汇总（当前约346行）不得把历史20、43或宿主45拼成新冻结GREEN。
- 统一纠正文案：“先前45 pass来自宿主，仅为历史宿主测试结果，不是Win10 VM证据，不能证明当前源码通过。本轮客户端测试未运行，等待主统一Win10窗口；Android产品E2E仍未执行。” 本次读取的docs未定位含45 pass的明确原句，不猜造行号；交接文档android_connection_resume_handoff.zh.md第13行已作正确声明，但下方历史20项仍需原件独立核验。

总账和上述docs由主修改，本owner未写入。SDK D:/project/51baoliao/tools/android-sdk；device0现有x86_64 AVD不能运行arm64-only包作为E2E。当前发布包不含本轮源码，不能用于新修复验收。

## 源码bundle

android_current_bundle.sources.txt列出精确覆盖文件。它是完整Flutter工程的源码覆盖层，主需冻结完整工程和依赖、检查hash后在独立副本运行。先记录编译故障，再仅移除非法d.scale赋值，记录手势业务RED；恢复测试可单独运行。所有测试实际数量/退出码由主保存，本文件不预填pass数。

VM命令（本轮未执行，工作目录flutter）：

```text
flutter --no-version-check test --no-pub --reporter=json --concurrency=1 test/mobile_connection_resume_test.dart test/android_connection_resume_policy_test.dart test/android_resume_model_flow_test.dart
flutter --no-version-check test --no-pub --reporter=json --concurrency=1 test/android_two_finger_wheel_test.dart test/android_touch_pointer_widget_test.dart
```

本轮修改仅四个Android测试文件（两个新增断言、两个需求冲突输入修正）及本交接资产，生产运行路径未变。未执行analyze：PATH未发现dart；未执行format或动态测试。AOCI工具未挂载，本轮未维护认知资产。
