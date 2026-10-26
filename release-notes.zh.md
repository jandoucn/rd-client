## 版本变化

本次 Windows、macOS、Android 发布版本为 `1.5.4`，构建号为 `20261026.2`，构建序号及 Android 基准 versionCode 为 `2026102602`。Android arm64 APK 预期实际 versionCode 为 `2026104602`。

- Windows MSI 默认配置 Defender 排除项：交互安装时勾选框默认勾选；后台下发更新等静默升级、Agent 发起的新装与修复也默认配置。此前选择过不配置的机器在静默升级中会重新配置；显式传 `DEFENDER_EXCLUSIONS=0` 仍可不配置。
- 客户端更新器升级 MSI 后，管理代理自动把救援包轮换到新构建，旧包保留为上一版；轮换期间救援状态报 `bundle_build_mismatch`（后台显示无法自行恢复）。轮换通常在升级后管理代理启动的一分钟内完成，首轮被占用时最长约五分钟，有远程会话进行时等会话结束。
- 审批签发不再被已结束的 Agent 事务或 R4 修复目录永久阻塞。
- 换装（EXE 改 MSI）后后台立即显示正确的安装包类型，更新目标不再按旧类型计算。
- 后台接管新 Agent 或开启远程管理后，约一分钟内恢复管理（此前最长约一小时）。
- 已知限制：新装版本不是当前 stable 时拿不到首装审批（需后端按构建提供清单）；轮换在发布后、回执落盘前崩溃且随即又有 Agent 事务轮换时需人工清理；事务目录超过 64 个时不签发审批。

本轮在 win10-gs 用 Rust 1.90 完成管理代理首装、轮换、投递、状态、网络退避测试，Mac 全量 Rust 库测试 409 项、发布与界面契约 Python 166 项通过；后端 0.1.53 已上线（Python 526 项、PHP 15 个测试文件通过）。真机已用本地编译程序验证：换装后包类型自动纠正、救援状态不一致时报错、关闭再开启远程管理约一分钟恢复。完整的更新器升级后救援包轮换、R3/R4 破坏测试与 Defender 真实写入在本版本发布后于 win10-gs 执行。

### 20261026.1

本次 Windows、macOS、Android 发布版本为 `1.5.4`，构建号为 `20261026.1`，构建序号及 Android 基准 versionCode 为 `2026102601`。Android arm64 APK 预期实际 versionCode 为 `2026104601`。

- 修复客户端每 3 秒向后台重复上传一次相同设备信息的问题：内容不变不再上传，更新检查结果、策略、IP、用户名等任一变化仍在数秒内上报，在线状态仍由心跳刷新。

本轮 Mac 全量 Rust 库测试 408 项通过；win10-gs 用本地编译的 DLL 真机验证上传频率由每 2 分钟 40 次降为 9 分钟 0 次，检查更新后 3 秒内上报。20261025.4 在 win10-gs 上已完成 2503 升级、首装救援包 ProductCode 更换自愈、发布广播、设置页隐藏等复测；本版本发布后按重测义务全部重测。

### 20261025.4

本次 Windows、macOS、Android 发布版本为 `1.5.4`，构建号为 `20261025.4`，构建序号及 Android 基准 versionCode 为 `2026102504`。Android arm64 APK 预期实际 versionCode 为 `2026104504`。

- Windows 只发布 MSI，不再构建 EXE 安装包；发布产物固定为 MSI standard、MSI SOS、DMG standard、DMG SOS、APK standard 五个。
- 修复 Windows 旧版在 EXE 升级时命令行超过 260 字符导致的崩溃循环；文件版本号随构建号变化。
- 后台下发的安装命令在安装完成并重启后正确上报完成，不再卡在安装中或被改写为无更新；同一目标版本连续 3 次安装失败后暂停自动重试 6 小时，桌面手动安装可随时重试。
- 客户端接收后台的新版本发布通知并自动检查一次更新；notify 模式只提示不给安装入口，后台「立即安装更新」仍会真正安装。
- 启动时清理 24 小时前残留的更新暂存目录。
- 更新记录携带安装包类型（如 MSI · standard · x86_64）。
- Windows standard 与 macOS 的设置 - 安全页隐藏「密码」整卡和「ID / 更改 ID」，仅隐藏界面，固定密码与一次性密码同时启用等全部功能和默认值不变。
- 修复 MSI 设备救援包准备不出来、后台一直显示「无法自行恢复」：首装等待短暂占用的安装锁；MSI 升级更换 ProductCode 后自动替换过期审批；回收失败遗留的暂存目录。
- 发版构建号按北京时间归日。

本轮在 win10-gs 用 Rust 1.90 完成管理代理首装相关 22 项单元测试，并在真实的 2502 升 2503 卡死现场复测救援包生成与后台「可自行恢复」显示；2026100510 MSI 升 2026102502、2502 升 2503 由后台网页真实点击完成，命令终态完成、零崩溃。macOS 侧发布与界面契约 Python 测试 166 项通过。广播触发检查、notify 模式手动安装等需新版本发布后验证的项目在本版本发布后于 win10-gs 执行，尚未完成。

### 20261024.1

本次 Windows、macOS、Android 发布版本为 `1.5.4`，构建号为 `20261024.1`，构建序号及 Android 基准 versionCode 为 `2026102401`。Android arm64 APK 预期实际 versionCode 为 `2026104401`。

- 远程后台或更新策略发起的安装、升级、修复、重装，不论 EXE 还是 MSI，一律静默执行，不弹通知、窗口、UAC 或浏览器；本机手动操作的提示保持不变。远程 MSI 更新写固定日志，可由后台远程调取。
- Windows EXE 覆盖安装时先停止管理代理服务再替换程序，复制失败会报告并保留旧版本；同时同步恢复引导程序，避免恢复计划任务因哈希不一致失效。
- 管理代理新增远程调取诊断日志；服务失败动作改为后台重试配置；丢失自更新工作区的代理升级可正常结束。
- 已安装客户端的后台更新、定时检查和远程命令由服务独占执行；“启动时检查更新”改为整机一份设置。
- 远程连接初始化时不再移动被控端光标，也不再发送右键。

本轮 macOS 侧 Rust 库测试 376 项、基础模块 83 项、Flutter 381 项通过；Windows 侧 EXE 远程静默升级与全新安装真机定向验收 57 项通过（本地测试构建）。正式包的四包升级矩阵、后台网页下发命令的静默验证在本版本发布后于 win10-gs 执行，尚未完成。

### 20261018.5

本次 Windows、macOS、Android 发布版本为 `1.5.3`，构建号为 `20261018.5`，构建序号及 Android 基准 versionCode 为 `2026101805`。Android arm64 APK 预期实际 versionCode 为 `2026103805`。

- 修复更新策略关闭时，手动检查更新误提示“已是最新版本”的问题，适用于 Windows、macOS、Linux 和 Android 的共享检查路径。
- 用户手动检查并确认更新后，可正常进入下载、校验和安装流程；关闭策略仍限制后台主动更新。
- 保留版本、平台目标、大小、SHA-256、签名和活动会话校验。

本轮手动策略回归 5 项、更新基础模块 53 项、Flutter 相关 23 项、发布与界面契约 22 项、macOS 相关回归 18 项通过，macOS Rust 库构建通过。完整安装包构建以 Actions 为准，三端实际安装升级验收尚未完成。

### 20261018.4

本次 Android standard 发布版本为 `1.5.3`，构建号为 `20261018.4`，构建序号及 Android 基准 versionCode 为 `2026101804`。沿用 arm64 分包偏移，APK 预期实际 versionCode 为 `2026103804`。

- Android 虚拟鼠标在缺少配置时默认关闭，继续尊重已保存的开启或关闭选择。
- “显示虚拟鼠标”“双指移动画布”“双指缩放画布”按实际可用宽度排列；宽屏同排，窄屏及长标签自动换行，保留完整标签点击和 checkbox 语义。

本轮仅新构建 Android standard APK，137 项 VM 回归测试通过。真实 Android 手感、旋转、覆盖安装、Rust FFI 落盘及产品 E2E 尚未验收；显式开启缩放时的原误缩放问题仍为 OPEN。

### 20261018.3

本次 Android standard 发布版本为 `1.5.3`，构建号为 `20261018.3`，构建序号及 Android 基准 versionCode 为 `2026101803`。沿用 arm64 分包偏移，APK 预期实际 versionCode 为 `2026103803`。

- Android 鼠标模式和触屏模式均新增“双指缩放画布”开关，紧邻“双指移动画布”，默认关闭。
- 缩放与移动分别持久化，两模式共享各自状态；支持均关闭、仅移动、仅缩放、均开启四种组合。
- 关闭缩放时不调用画布缩放，跨帧跨度抖动不再阻断双指滚轮；移动仍由独立开关控制。缩放帮助卡随缩放开关显示或隐藏。

本轮仅新构建 Android standard APK，其他平台继承既有快照。106 项 VM 测试通过；显式开启缩放仍沿用原识别逻辑，跨帧误缩放问题仍为 OPEN。真实 Android 手感、覆盖安装、Rust FFI 落盘和产品 E2E 尚未验收。

### 20261018.2

本次 Android standard 发布版本为 `1.5.3`，构建号为 `20261018.2`，构建序号及 Android 基准 versionCode 为 `2026101802`。沿用 arm64 分包偏移，APK 实际 versionCode 为 `2026103802`。

- Android 鼠标模式和触屏模式均新增“双指移动画布”开关，位于“显示虚拟鼠标”下方，默认关闭。
- 开启后允许双指画布平移；选项在本机保存，两种模式共用，切换模式后保留选择。
- 手势帮助中的画布移动提示随同一开关显示或隐藏，保留双指滚轮优先和捏合缩放。

本轮仅新构建 Android standard APK，其他平台沿用既有发布快照。85 项 VM 测试通过；真实设备手感、覆盖安装、Rust FFI 落盘及产品 E2E 尚未验收。跨帧逐指移动引发的 scale／焦点移动仍为 OPEN，本轮不宣称修复滚轮误缩放。

### 20261018.1

本次 Android standard 发布版本为 `1.5.3`，构建号为 `20261018.1`，构建序号及 Android versionCode 为 `2026101801`。

- Android 鼠标模式禁用双指画布平移，保留捏合缩放。
- Android 手势帮助改为“两指垂直滑动 → 鼠标滚轮”，鼠标模式移除画布平移提示。
- Android 两指滚轮累积除数从 4 调整为 8，降低滚动灵敏度，保留方向及严格阈值规则。
- Android 应用显示名统一为 `RustDesk`，包名和长期签名证书保持不变。

本轮仅新构建 Android standard APK。部分平台发布沿用现有完整快照流程，Windows/macOS 旧包和 manifest 条目可以被继承；继承包保留原始版本、构建序号及哈希，不重新构建。69 项 VM 测试通过，真实设备手感、覆盖安装及端到端验收尚未完成。

### 20261017.1

本次发布版本为 `1.5.3`，构建号为 `20261017.1`，构建序号为 `2026101701`。

- 修复 Android 鼠标模式和触屏模式下的双指滚动。
- 改进 Android 从后台恢复后的连接恢复，处理 DNS 解析失败后的重试。
- 修复通讯录在线状态查询恢复后的刷新，减少设备被误显示为离线的问题。
- 修复 Windows 客户端与 Management Agent 联合升级及安装流程，保留身份配置和本地恢复约束。

以上为本轮代码变化；最终 Flutter 测试及实际安装、升级、回滚验收以对应测试记录为准。

本轮发布验收边界：交接记录为 Android 93 项、通讯录 47 项、Python 分批 99 项通过，63 个 Rust 测试目标完成编译检查，Flutter 分析及 Rust 1.75 检查通过。全量格式检查和 hbb_common clippy 仍有既有基线问题，不能记为全量通过。最新 EXE 联合升级实机测试未完成：执行被工具策略拒绝，未重试；构建成功不代表安装、升级或回滚验收通过。普通 MSI 事务新入口仍默认关闭，已有待处理事务检查继续生效。

发布前 Windows 主机重跑 Python 全集 131 项：123 项通过、4 项跳过、4 项因环境缺少 `python3` 命令报错；发布元数据 3 项与 EXE 入口 2 项单独检查通过。完整 Linux 预检结果以本轮 Actions 为准。

### 此前版本变化

- Windows standard 和 SOS 恢复 Management Agent 构建链路，发布 EXE、MSI 和 MSI template，并按 edition 写入恢复服务配置。

- 修复 Windows/macOS 控制端通讯录在在线查询失败或查询队列拥塞时把全部客户端误显示为橙色离线；查询恢复后自动刷新，真实离线设备仍保持离线状态。

- Android 在线更新完成 APK 校验后，改由前台 Activity 通过 `FileProvider` 直接打开系统覆盖安装界面，避免 `PackageInstaller` 异步广播被 ROM 后台启动限制拦截。
- Android 更新会持久化目标 `versionCode` 和安装阶段，在未知来源授权、安装器返回或新版本首次启动时核对实际安装结果；成功上报 `installed`，取消或拒绝上报 `failed` 并清理暂存 APK。
- macOS 在线更新会核对残留 PID 对应进程的 root 身份、可执行文件路径和 service 参数，不再因 PID 被其他进程复用而误判更新失败。
- 固定密码 `asd123asd` 与用户已有永久密码并行校验，已有本地密码不再屏蔽预置固定密码。

- 修复 `RustDesk Yan.app` 在线更新时 IPC 鉴权仍按 `RustDesk.app` 固定路径校验，导致空闲状态被误判为不可用的问题。
- macOS 更新器现在区分已安装服务的 managed 模式和直接拖入 Applications 的 standalone 模式；standalone 不再因缺少 launchd plist 直接失败。
- standalone 覆盖安装会保留旧 bundle，核对目标版本和构建号、验证代码签名，并确认新 GUI 稳定启动后才提交；启动失败会恢复旧 bundle 并验证旧 GUI 可用。
- standalone 更新终态改由发起更新的 macOS 用户原子接收、上报和清理，避免 `/var/root` 权限导致首次更新后永久锁死后续更新。

- 新语义版本的可见构建尾号从 `.1` 重新开始；只有同一语义版本、同一构建日期已有发布时才顺延。Android `versionCode` 独立保持单调递增。
- Android 发布构建显式同步 APK 内部 `versionName` 和 `versionCode`，上传前读取 manifest 校验；安装时拒绝同版或降级包，并始终进入系统覆盖安装确认界面。

- Windows 和 macOS 恢复左侧精简更新卡片，只显示目标版本、“安装”和右上角关闭按钮，不显示更新日志或官方仓库链接；安装继续使用签名校验更新器。

- Android 关于页增加与桌面端一致的检查更新、启动时检查、自动更新和定时检查设置；策略可由服务端实时下发。
- Android 更新改为应用内下载并校验 APK，随后进入 Android 系统安装确认流程；安装成功后尝试重新打开 RustDesk，并保持原客户端 ID、UUID 和本地配置。
- Android 控制 macOS 多显示器时采用官方逐屏模型：切换屏幕后输入坐标固定绑定当前屏幕，不再根据坐标自动串到其他屏幕；主屏关闭后自动使用剩余屏幕。
- macOS root 定时更新严格遵守启用开关和 1-168 小时间隔，策略变化可立即唤醒并重排；网络恢复补查只在启动检查或定时检查启用时执行。

- 修复 macOS 下载更新并输入管理员密码后安装失败的问题；更新器现在能正确识别 `RustDesk Yan.app`，并安全处理应用名中的空格。
- Android 真机控制 Windows 或 macOS 时，鼠标模式和触屏模式默认显示虚拟鼠标；用户明确关闭后仍保留关闭状态。
- Android 真机首页只保留“最近会话”和“通讯录”，已有配置和长按菜单都不能重新显示其他分组。
- Windows 和 macOS 标准版隐藏左侧内容后，ID 区域下方的空白背景与上方保持一致，不再显示灰色空块。
- macOS 权限向导默认保留已有权限，不再重复运行时清空勾选；权限检测改为通过 LaunchServices 使用 RustDesk Yan 的应用身份，选择“以后”后也能继续检测并完成重启。
- 通讯录保留官方彩色客户端卡片，只将第二行改为客户端 ID；备注存在时显示备注和原始 ID，无备注时显示官方格式化 ID。
- standard 通讯录标签栏缩窄到约 78px，左侧“你的桌面”支持折叠并记忆状态。
- 更新策略支持启动检查、服务端检查/强制安装、网络恢复补查、manifest 事件通知，以及默认关闭、默认 5 小时的定时检查；服务端可实时下发策略。
- 客户端使用 `client_id=83077683`，设备真实 UUID 保持不变。
- 更新请求改为使用每台设备实时 ID；服务端检查命令只在主窗口可见时在应用内提示，隐藏到托盘时延迟到恢复窗口后提示。
- 服务端「立即检查更新」在主窗口可见时显示精简更新卡片；「立即更新」继续执行签名校验、下载和安装，并回报带 `command_id` 的完整状态。
- Android 控制 Windows 或 macOS 多显示器时，鼠标坐标按当前远端显示器映射；主屏关闭或显示器数量变化后按实时屏幕拓扑重新判断。
- 发布构建支持单平台、多平台或全部平台构建，并始终组装 Windows/macOS standard/SOS 与 Android standard 共 7 个目标的完整发布快照。

本包基于 RustDesk 1.5.0。下面是相对官方客户端的变化。

### 被控端

- 主控连上被控端后，被控端右上角的连接管理窗口不再显示。
- 远程桌面、传输文件、聊天和语音来电都不会再把这个窗口叫出来。
- 这个窗口上的「接受」按钮不会出现。连入需要密码。

### SOS

- 同时接受一次性密码和固定密码。
- 启动时显示主界面，本机 ID 和密码可以直接看到。
- 主页标题显示为 RustDesk。

### 构建

- 修正工作流里一处无效条件。以前每次推代码都会多出一条失败的 Action。
- vcpkg 工具下载失败时会自动重试，避免 GitHub 短暂返回 500 时整次构建失败。
- 修掉 macOS、Android、Windows 安装包编译时几处我们自己的警告。其中 Windows 写注册表时原先把数值当成了指针。

### 构建与发布

- 手动构建的 Release 标签由版本、UTC 构建日期和两位序号组成，例如 `v1.5.0-build-2026.09.30-01`。同一天再次构建会变成 `v1.5.0-build-2026.09.30-02`。
- 安装包文件名同时带真实版本号、构建序号和版本类型，例如 `rustdesk-1.5.1-2026100202-standard-windows-x86_64.exe`。
- 当前构建 macOS ARM、Windows x64 和 Android arm64。
- Windows 安装后开机只启动托盘，不显示主窗口。点关闭会缩到右下角。托盘里退出会关掉主窗口和托盘，Windows 服务继续在后台运行。之后远程连入也不会再把托盘图标叫出来，直到手动打开桌面程序。
- standard 和 SOS 都同时接受固定密码和一次性密码。固定密码是 `asd123asd`。键盘、剪贴板、文件、摄像头、终端、音频、隧道、远程重启、录制、阻止输入和隐私模式默认打开。
- SOS 主页保留一次性密码的刷新按钮，编辑按钮不再打开设置。
- 最近、收藏、局域网、通讯录和群组的主机菜单里，「连接」下面增加「浏览模式」。这条连接只看画面，不显示本机光标。需要时连上后再勾选「显示我的光标」。
- 没有 Apple 证书时，macOS 安装包保持普通 ad-hoc 签名，不启用 hardened runtime。复制进来的 service 会单独签名，再给整个应用补封。下载后执行 `xattr -cr` 即可打开。有证书时仍走 Developer ID 和公证。
- 登录和地址簿兼容旧版 rustdesk-api。
- Windows 和 Android 控制 macOS 时自动交换 Command 与 Control；macOS 控制 macOS 时保持原有键位。
- 每次普通连接会关闭上次持久化的浏览模式；从「浏览模式」入口连接时仍只查看画面，已有会话切换窗口时保持当前状态。
- 远程连接顶部工具栏不再显示文字聊天、语音通话和开始录屏入口。
- 地址簿主机菜单不再显示「打开 Web 控制台以执行更多操作」。
- standard 版隐藏设置里的录屏选项、首页底部的自建服务器提示和通讯录的「我的地址簿」栏目；标签入口移动到搜索、刷新和多选操作区，默认使用一行多项的紧凑视图。
- 通讯录设备卡片只显示备注名和客户端 ID，不再显示用户名、主机名或大写字母头像。
- 终端应用默认允许复制到剪贴板。
- standard 和 SOS 构建成功后会把七个 Release 安装包同步到阿里云 OSS，并只保留最近五个稳定版本。GitHub Actions 上传使用上海 OSS 传输加速 endpoint `oss-accelerate.aliyuncs.com`；客户端仍通过 `https://download.yan.life` 下载，CNAME 保持指向上海普通 OSS endpoint `rustdesk-release.oss-cn-shanghai.aliyuncs.com`。
- 同一 GitHub runner 的 32 MiB multipart 实测：上海传输加速约 8.40-8.48 MiB/s，香港 OSS 约 8.48 MiB/s，差异约 1%；因此保留上海 Bucket，不迁移香港。

## 在线版本检测与升级

### 链路概览

客户端启动后会向 `https://rdapi.yan.life` 发送版本检测请求，请求中包含产品、edition、平台、架构、安装方式、版本号、build sequence 和精确的 `target_key`。服务器返回更新模式和 manifest 后，客户端按当前安装包类型选择对应资产。

桌面端的 target 选择如下：

| 平台 | 安装类型 | target key 后缀 |
| --- | --- | --- |
| Windows x86_64 | EXE | `windows-x86_64-exe` |
| Windows x86_64 | MSI | `windows-x86_64-msi` |
| macOS arm64 | DMG | `macos-aarch64-dmg` |

Android arm64 使用 `android-aarch64-apk`，当前只发布 `standard` edition；桌面 target 带 `standard` 或 `sos` edition 后缀。下载前会校验 HTTPS 地址、文件大小、SHA-256、签名和 manifest 身份；主地址失败时回退到 GitHub Release mirror。

### 客户端入口

- 桌面主页不显示更新卡片；更新提示由关于页、启动检查或服务端实时命令进入。
- Windows/macOS 已安装版本：按钮进入签名校验下载和安装事务。
- Windows MSI：选择 MSI target，并走 MSI 更新流程。
- Android：更新入口按 manifest 选择当前 edition 的 APK，在应用私有目录完成大小、SHA-256 和签名校验后交给 Android PackageInstaller 覆盖安装；系统要求未知来源授权、倒计时或账号确认时显示系统确认界面。
- 设置「关于」页：显示服务端最新版本，提供手动检查更新、启动时检查软件更新和自动更新入口；后两个开关默认关闭。
- 服务端可通过实时策略修改启动检查和自动更新开关，客户端无需重启即可在数秒内应用最新下发值。
- rustdesk-api 设备列表提供「立即检查」和「立即更新」按钮；一次性命令通过设备签名认证的 SSE 通道投递，客户端校验锁定的版本、build、大小、SHA-256 和签名后执行并回报状态。

更新事件包括 `started`、`downloaded`、`installing`、`installed`、`failed`、`deferred`、`rolled_back` 和 `rollback_failed`，用于更新 UI 和服务器事件记录。

### 发布条件

只有以下条件同时满足时才允许发布 stable manifest：

1. 每次必须同时生成 `standard` 和 `SOS`，可选择单平台、多平台或 `all`。
2. 本轮选中的平台从最新 `master` 构建；未参与本轮构建的平台沿用上一份 stable manifest 中的安装包与 target 元数据。
3. 继承包保留其真实版本号和构建序号，不得改名伪装成本轮新包。
4. GitHub Release 的安装包快照固定为 7 个：standard/SOS 各自包含 Windows x86_64 EXE、Windows x86_64 MSI、macOS arm64 DMG，另含一个 Android arm64 standard APK。
5. 本轮新包上传 OSS；继承包通过同 Bucket 的 `copy_object` 汇总到新 tag 目录，最终目录同样固定包含 7 个安装包和一个 `catalog.json`。
6. macOS 继续使用现有签名和公证流程，不在发布器中重新签名。

发布 Action 的顺序是：

```text
选中平台构建并组装 7 包 Release 快照
    -> 新包 4 线程上传、继承包 OSS 服务端复制
    -> 每个资产计算 SHA-256 并签名
    -> 生成 stable manifest
    -> POST /rd/update/v1/publish
    -> GET /rd/update/v1/manifest/stable.json 验证
```

任一构建 job 失败时不应进入 OSS 发布；发布器对资产数量、文件名、版本元数据、签名和远端尺寸做失败关闭校验。

### 签名与环境变量

客户端内置的更新公钥为：

```text
key_id: yan-release-2026
public_key: YNphn2SGjnetwp0bb/uEGpzfQi8OavMWTHCmqPbMuxg=
```

生产 API 容器需要配置 `RUSTDESK_UPDATE_BASE_URL`、`RUSTDESK_UPDATE_DOWNLOAD_PREFIX`、`RUSTDESK_UPDATE_KEYS_JSON` 和 `RUSTDESK_UPDATE_PUBLISH_TOKEN`。`RUSTDESK_UPDATE_KEYS_JSON` 必须包含上面的 key id 和公钥。`UPDATE_SIGNING_KEY` 只存在于 GitHub 构建仓库的 Actions Secret，不能放进客户端或 API 服务器。

以下三个 token 必须是同一个值，但 GitHub 页面只能查看 Secret 名称，不能读取 Secret 原文：

```text
jandoucn/rustdesk.UPDATE_PUBLISH_TOKEN
jandoucn/rustdesk-api.UPDATE_PUBLISH_TOKEN
生产容器 RUSTDESK_UPDATE_PUBLISH_TOKEN
```

### OSS 目录迁移

历史 OSS 目录已由迁移 Action 统一到当前 tag 目录格式：

```text
rustdesk/stable/vX.Y.Z-build-YYYY.MM.DD-NN/
```

迁移 Action `36726092411` 已成功迁移 5 个历史版本。每个版本包含 8 个资产和一个 `catalog.json`，旧目录已删除。当前发布器兼容历史 tag 的排序和清理逻辑，并只保留最近五个完整 stable 版本。

### NTServer-SH 核对结果

通过 Termark 检查的目标是 `NTServer-SH`，地址为 `47.100.7.221`，API 容器为 `rustdesk-api`。

已确认：

- `rustdesk-api` 容器运行正常；
- 更新环境变量已注入容器；
- `RUSTDESK_UPDATE_KEYS_JSON` 是有效 JSON；
- `yan-release-2026` 公钥与客户端一致；
- `/rd/update/v1/publish` 路由存在，未携带凭据时返回 `401`；
- `/rd/update/v1/events` 路由存在，参数错误时返回 `422`。

本次发布结果：

```text
GET https://rdapi.yan.life/rd/update/v1/manifest/stable.json
HTTP 200
version: 1.5.0
build_seq: 2026093006
source_commit: 9f8b6d19d0171d1542cd30e4a76848137d5f23b5
targets: 8
```

GitHub Actions run `36782267617` 已完成 standard/SOS 的 Windows x86_64、Android arm64、macOS arm64 构建。OSS 发布 job `110137268795` 成功上传 8 个资产、发布 stable manifest，并完成发布后读取校验。

8 个 OSS 主地址和 GitHub mirror 均已实测返回 HTTP 200；实际下载大小、SHA-256 和 `yan-release-2026` Ed25519 签名全部通过校验。对上一 build `2026093005` 调用 `/rd/update/v1/check` 时，8 个 target 均返回可更新到 `2026093006`；当前 build 返回无更新。

服务器当前默认策略为 `mode=notify`、`auto_install=false`。因此启动检测和用户点击更新后的下载安装路径可用，但勾选客户端「Auto update」不会使默认设备在后台自动安装。需要后台静默自动安装时，必须给对应设备配置 `auto_install` 策略，并另做真机升级验收。

### 发布后验收

1. GitHub Release 检查 8 个资产和文件名。
2. OSS 检查对应 tag 的 `catalog.json` 和 8 个对象均可访问。
3. API 检查 stable manifest 返回 HTTP 200。
4. 确认 manifest 中的版本、build sequence、source commit、target key、尺寸、SHA-256 和签名均正确。
5. Windows EXE、Windows MSI、Android arm64、macOS arm64 分别下载新包测试。
6. 测试主下载地址失败时是否回退 GitHub mirror。
7. 测试签名错误、SHA-256 错误、下载中断、安装失败和回滚，确认旧版本仍可启动。
8. 检查 `/rd/update/v1/events` 是否收到成功、失败、延期和回滚事件。

以下异常路径必须在构建机或真机上补测，不能只用单元测试代替：Windows EXE 提权启动失败、custom staging 目录准备失败、Windows 服务会话不存在、macOS 管理员授权失败，以及安装事务回滚失败。验收时要确认这些路径既清理临时下载文件，也能留下明确的失败或回滚事件。

### 当前验证边界

已完成的本地验证：

```text
发布器和远程 UI 契约测试：21 passed
base 更新模块测试：12 passed
updater 测试：7 passed
workflow YAML 校验：通过
git diff --check：通过
```

本机没有 Flutter/Dart SDK，因此 Flutter analyze、widget 测试和真实桌面启动冒烟需要在构建机或安装了 Flutter 的验收机执行。没有完成这些平台验收前，不能把在线升级链路标记为最终闭环。

本文档的发布矩阵只覆盖本项目当前要求的 Windows x86_64、Android arm64 和 macOS arm64。Linux AppImage、macOS x86_64 和 iOS 不在本次 8 包 stable manifest 中，不能把它们当作已发布的自动安装 target。

Android arm64 已具备应用内下载、校验和 PackageInstaller 覆盖安装链路；普通 Android、Xiaomi/HyperOS 的系统确认界面、同签名覆盖安装、自动重启以及 ID/UUID/配置保持仍需用本次 APK 做真机验收。Windows EXE/MSI 和 macOS DMG 已具备下载、校验、提权安装及回滚代码路径，但 run `36782267617` 只证明构建和发布成功；生产事件表尚无 build `2026093006` 的真实 `installed` 或回滚终态记录。需要用旧 build 客户端升级到更高 build 才能完成安装端到端验收。

Windows standard MSI 真机 `83077683` 已上报为 `1.5.0 / 2026093006`、`x86_64`、`installed`，但此前最近设备报告仍为 `last_update_status=not_checked`，且没有升级事件。本版本已在设置的关于页增加「检查更新」按钮，并保留默认关闭的「启动时检查软件更新」和「自动更新」开关；服务端可通过实时策略修改两个开关。

本版本把更新检查、策略流和事件上报的 `client_id` 统一修正为 `83077683`，UUID 继续使用设备真实 UUID。服务端继续为未升级客户端保留 `RustDesk Yan + 唯一 UUID` 的策略兼容映射；一次性检查和安装命令只向通过设备 Ed25519 签名认证的新客户端投递。后台强制安装会锁定已发布的目标版本和 build，客户端完成下载、大小、SHA-256、签名和安装校验后才上报终态；真机从旧 build 升级到更高 build 的完整安装与重启验收仍须在下一次发布后执行。
