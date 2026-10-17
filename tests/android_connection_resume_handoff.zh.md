# Android 后台连接恢复接线检查点（2026-10-17）

当前状态：最小修复候选已交接，待独立审查与主代理统一运行测试；本轮没有使用 VM、CI、测试运行器或提交。不得标记第二个 bug 完成。

## 本轮候选与测试 bundle（优先于下方历史记录）

源码清单：`tests/android_resume_test_bundle.sources.txt`。按清单携带七个文件，并使用主代理统一冻结的完整 Flutter 工作树及已有依赖；清单不是可单独运行的完整工程。本轮没有改动 pubspec.yaml、pubspec.lock 或 package_config.json。

- 首批测试先于实现写入：late DNS 的 handled 返回值、manual retry 消费资格、close + clear 后迟到 restart、旧 listener 不得操作新 controller、非 Android 保留 restart 路径。原 mobile_connection_resume_test.dart 已直接 import helper。
- 后续补充：拒绝的恢复回调返回 false；前台 transport/restart 不进入额外恢复；旧页面 dispose 不关闭新 controller；peer 变更拒绝旧恢复；明确 close 后手动重连不得进入原生调用；manual reconnect 在原生调用前消费资格。
- 删除仅供测试读取的 failure map，原 pending 断言替换为“未展示错误 + resume 后重连次数/ready 后不重连/关闭后迟到消息无重连”的行为断言，没有跳过或删除测试场景。
- 本轮没有执行 RED 或 GREEN。以上是已编写的预期失败用例，不是已观测业务 RED；主代理需在隔离基线与本轮候选上统一运行并记录结果。bool 回调签名迁移造成的编译失败不能算业务 RED。
- 用户指出的先前宿主 **45 pass 仅为宿主测试结果，不是 VM 证据**，也不证明本轮修改通过。下方旧记录的 20 pass 属于历史源码版本，不能套用到当前候选；本轮没有重新核验其 raw 回执。

建议主代理执行（本轮未执行）：

```text
flutter --no-version-check test --no-pub --reporter=json --concurrency=1 test/mobile_connection_resume_test.dart test/android_connection_resume_policy_test.dart test/android_resume_model_flow_test.dart
```

运行前冻结源码和依赖摘要，保存机器身份、命令、stdout/stderr、退出码与实际测试数量。主代理统一安排 Win10，当前 owner 不访问该 VM。helper/model 替身测试不能替代 Android 产品 E2E。

宿主静态检查：使用既有 Flutter 3.24.5 中的 Dart，对清单全部七个 Dart 文件执行 analyze；最终无 error，1 个既有 warning（common.dart 未用 dart:math）和 28 个 info。未修这些范围外诊断，因此不宣称 analyze 全绿。未做产品构建、覆盖率或动态测试。

本轮回归面与必要性：

- android_connection_resume.dart：去掉两个 in-flight bool；deferFailure 在后台存故障返回 true，恢复后仅首次 DNS 尝试返回实际回调结果；retryStarted 消费资格并清待恢复故障，connected 才重置资格。
- models/model.dart：删除 failure map；close 仅有 controller 才取消旧 timer；clear 保留已关闭 controller，mobileReset 延续既有清 tombstone 路径；FFI.close 使用 isAndroid gate。reconnect 和原 restarting 原生调用入口消费资格，关闭 tombstone 阻重连；普通认证错误保留展示。事件 listener 捕获 controller 实例，回调核验 session/peer，避免旧事件改变替换后的恢复状态。
- mobile/pages/remote_page.dart：dispose 传入原 controller 身份，避免该恢复关闭 hook 关闭替换后的 controller。未重构既有异步页面销毁流程。
- common.dart：本轮无需修改；bundle 必须携带已有的创建/传递同一 controller 与 closeConnection 同步关闭接线。

独立审查重点/仍待验收：真实通用 timer 与 restart timer 的竞争及无 controller 时 close 不取消 timer；真实 FFI.close→clear→mobileReset 生命周期；Android 后台 DNS 与首个 late DNS 的错误 UI；原生 reconnect、权限/画面恢复、普通认证 UI、非 Android 回归、widget/设备 E2E、80% 分支覆盖率。manual model 测试在 InputModel 边界主动抛出确定异常，验证进入原生调用前的资格状态，不能声称覆盖原生重连成功。

当前环境未发现正式 aoci 索引或项目 .mcp.json，也未挂载 AOCI 工具；没有建立或改写认知资产。

## 原九测与本轮证据

未找到原任务九项 RED/GREEN 的 raw 日志和机器身份，因此原 GREEN 不采信。本轮重新在 VirtualBox 客体 RDLAB-WIN10（Windows 10 企业版 LTSC）执行原九测，Flutter 3.24.5、Dart 3.5.4。宿主 YAN 为 Lenovo 物理机，没有在宿主运行这些测试。

证据根：`C:\Users\Administrator\AppData\Local\Temp\opencode\android-resume-evidence-20261017`。

- `red-tests.jsonl` / `red-stderr.log`：缺少新接口及测试替身返回类型错误，编译失败，不算业务 RED。
- `red2-tests.jsonl`：有业务断言失败但伴随未初始化 FFI 错误，不作为有效 RED。
- `red3-tests.jsonl`：有效 RED，5 项中 3 失败/2 通过，期望重连1次实际0次。
- `final2-tests.jsonl`：原 helper 9项＋真实 model 事件流11项，20通过、0失败、0跳过，done.success=true，8510ms。
- `final2-result.json`：exit=0，UTC `2026-10-17T00:33:07.4061266Z`。
- `final2-environment.json`：VM硬件身份、SDK版本、五个源码SHA-256及运行后无Dart/flutter_tester/msiexec进程。
- `working.diff`：三个已跟踪业务文件相对HEAD的完整diff，包括此前已有helper。新增测试全文在 `flutter/test/android_resume_model_flow_test.dart`。

执行命令：`flutter --no-version-check test --no-pub --reporter=json --concurrency=1 test/mobile_connection_resume_test.dart test/android_resume_model_flow_test.dart`。

## 接线与回归面

- `flutter/lib/common.dart`：唯一移动端 RemotePage 构造前，由同一 gFFI.ffiModel 创建controller，builder只传递同一实例，避免builder重建丢失暂停状态。用户close入口在首个await前同步关闭controller并取消两类定时器。
- `flutter/lib/models/model.dart`：model内session map关联controller；恢复闭包核验owner、closed、peer和controller实例。handleMsgBox仅对已注册且paused的session识别网络故障；connection_ready清除待恢复状态；关闭标记拒绝迟到msgbox。FFI.close同步关闭，mobileReset清理旧关闭标记以免影响后续文件传输等复用gFFI的会话。
- `flutter/lib/mobile/pages/remote_page.dart`：保留原helper/生命周期hook，增加状态读取；dispose在异步清理前同步取消model重连。恢复先消费失败状态再调用model闭包，闭包取消 `_timer` 和 `_restartReconnectDelayTimer` 后调用原reconnect。

## 原生错误与健康语义

`src/flutter.rs` 的msgbox事件只有type/title/text/link/hasRetry，没有DNS稳定错误码。`src/client.rs::on_establish_connection_error` 将原错误字符串转发；`Connection closed: {err}({errno})` 只是日志，不能据此前缀匹配Flutter事件。当前DNS分类匹配翻译前 `failed to lookup address information:` 和 `Failed to look up host for `；前者为resolver错误前缀，后者可在hbb_common socket_client中确认。未声称覆盖所有resolver/系统语言。

io_loop发送的EOF为 `Reset by the peer`，超时为 `Timeout`；重启走restarting/restarting-show。普通 `Connection closed`、密码/权限及Incoming only mode不进入恢复分类，保留原处理。健康session仅paused/resumed不会重连或清空权限。

## 尚未完成的验收

测试调用真实startEventListener/handleMsgBox，但showMsgBox和reconnect是替身；没有初始化原生库、读取生产凭据或连接线上账号。不能把这些测试称作原生socket、真实重连或Android页面E2E。

尚缺：真实 `_timer`/restart定时器竞争执行验证、RemotePage完整widget/桌面driver、Android后台DNS实际复现、连接成功后真实恢复画面、完整健康配置前后摘要与产品ready验证、80%分支覆盖率、全量analyze及产品构建。本轮未生成/发送/安装APK。VM现有SDK/project复用，五文件摘要有记录，未对整个guest源码树做本轮冻结证明。
