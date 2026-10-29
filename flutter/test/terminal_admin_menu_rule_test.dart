import 'package:flutter_hbb/common/terminal_admin_menu_rule.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  // 「终端（以管理员身份运行）」「终端（SYSTEM）」只对 Windows 被控或未知平台显示。
  // 通讯录里手动添加的设备没有 platform，必须当作未知平台显示，否则新装设备连不了 SYSTEM 终端。
  test('Windows peer shows admin and SYSTEM terminal entries', () {
    expect(terminalAdminMenusApply('Windows'), isTrue);
  });

  test('unknown (empty) platform shows the entries', () {
    expect(terminalAdminMenusApply(''), isTrue);
    expect(terminalAdminMenusApply('   '), isTrue);
  });

  test('unrecognized or raw platform text counts as unknown and shows the entries', () {
    expect(terminalAdminMenusApply('windows / Windows 11 Pro - 11 (26300)'), isTrue);
    expect(terminalAdminMenusApply('windows'), isTrue);
    expect(terminalAdminMenusApply('FreeBSD'), isTrue);
  });

  test('known non-Windows platforms hide the entries', () {
    expect(terminalAdminMenusApply('Linux'), isFalse);
    expect(terminalAdminMenusApply('Mac OS'), isFalse);
    expect(terminalAdminMenusApply('Android'), isFalse);
  });
}
