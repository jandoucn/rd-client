/// 「终端（以管理员身份运行）」与「终端（SYSTEM）」的显示规则：Windows 被控或未知平台。
///
/// 平台取自通讯录 / 最近会话里 peer 记录的 platform。网页手动添加的设备没有这个字段，
/// 旧数据里还可能是 sysinfo 原文（如 "windows / Windows 11 Pro"）。这些都按未知平台处理，
/// 入口仍然显示；连接后由被控端决定是否支持（旧版会提示升级）。
/// 只有明确是其他平台（Linux / Mac OS / Android）时才隐藏。
///
/// 这里不引用 consts.dart：它会连带加载 FFI 桥，纯规则无法单测。
/// [_nonWindows] 必须与 consts.dart 的 kPeerPlatformLinux / MacOS / Android 保持一致。
const Set<String> _nonWindows = {'Linux', 'Mac OS', 'Android'};

bool terminalAdminMenusApply(String platform) =>
    !_nonWindows.contains(platform.trim());
