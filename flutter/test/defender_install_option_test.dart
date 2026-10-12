import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/desktop/widgets/defender_install_option.dart';

void main() {
  test('Defender is opt-in and saved choice is reused', () {
    expect(defenderConsentFromOptions({}), isFalse);
    expect(defenderConsentFromOptions({'DEFENDER_EXCLUSIONS': '1'}), isTrue);
    expect(defenderConsentFromOptions({'DEFENDER_EXCLUSIONS': '0'}), isFalse);
    expect(defenderInstallToken(false), ' defender=0');
    expect(defenderInstallToken(true), ' defender=1');
  });

  testWidgets('checkbox exposes consent, restrictions and disabled state',
      (tester) async {
    var selected = false;
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(body: StatefulBuilder(
        builder: (context, setState) => DefenderInstallOption(
          selected: selected,
          enabled: true,
          onChanged: (value) => setState(() => selected = value),
        ),
      )),
    ));
    expect(tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).value,
        isFalse);
    expect(find.textContaining('360'), findsOneWidget);
    expect(find.textContaining('安装目录'), findsOneWidget);
    expect(find.textContaining('ManagementAgent'), findsOneWidget);
    expect(find.textContaining('Client'), findsOneWidget);
    expect(find.textContaining('受控文件夹'), findsOneWidget);
    await tester.tap(find.byType(CheckboxListTile));
    await tester.pump();
    expect(selected, isTrue);
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(body: DefenderInstallOption(
        selected: true,
        enabled: false,
        onChanged: (_) => fail('disabled changed'),
      )),
    ));
    expect(tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).onChanged,
        isNull);
  });
}
