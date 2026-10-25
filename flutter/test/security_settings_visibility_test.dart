import 'dart:io';

import 'package:flutter_hbb/desktop/pages/desktop_setting_page.dart';
import 'package:flutter_test/flutter_test.dart';

// 设置 - 安全 里的「密码」整卡与「ID / 更改 ID」只在界面隐藏。
// 密码方式（默认同时使用两种密码）、一次性密码长度、数字一次性密码、固定密码
// 等选项本身仍由配置、远程策略和命令行生效，不能因为界面隐藏而失效。
void main() {
  test('Windows and macOS hide the password and change ID cards', () {
    expect(hidesSecurityPasswordAndIdCards(windows: true, macos: false),
        isTrue);
    expect(hidesSecurityPasswordAndIdCards(windows: false, macos: true),
        isTrue);
  });

  test('other desktop platforms keep the original cards', () {
    expect(hidesSecurityPasswordAndIdCards(windows: false, macos: false),
        isFalse);
  });

  test('the security page wires both cards through the visibility policy', () {
    final source =
        File('lib/desktop/pages/desktop_setting_page.dart').readAsStringSync();
    final build = source.substring(
        source.indexOf('permissions(context),'),
        source.indexOf('more(context),'));
    expect(build, contains('if (!hideSecurityCards) password(context)'));
    expect(build,
        contains('if (!isChangeIdDisabled() && !hideSecurityCards)'));
    // 只隐藏界面：密码卡与更改 ID 的实现本身必须原样保留。
    expect(source, contains('Widget password(BuildContext context)'));
    expect(source, contains('Widget changeId()'));
    expect(source, contains("translate('Use both passwords')"));
  });
}
