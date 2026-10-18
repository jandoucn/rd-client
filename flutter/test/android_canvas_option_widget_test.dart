import 'package:flutter/material.dart';
import 'package:flutter_hbb/mobile/widgets/gesture_help.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_test/flutter_test.dart';

import 'android_touch_pointer_widget_test.dart' as pointer;

class StoredCanvasMode extends VirtualMouseMode {
  StoredCanvasMode(this.store) : super(pointer.FakeModel(false));
  final Map<String, String> store;
  bool failWrite = false;
  @override
  String readCanvasMovementOption() => store['canvas'] ?? '';
  @override
  Future<void> writeCanvasMovementOption(String value) async {
    if (failWrite) throw StateError('fixture write failure');
    store['canvas'] = value;
  }
}

String identity(String value) => value;

void main() {
  test('failed persistence preserves old option and state', () async {
    final store = {'canvas': 'Y'};
    final mode = StoredCanvasMode(store)..loadCanvasMovementOption();
    mode.failWrite = true;
    await expectLater(mode.toggleCanvasMovement(), throwsStateError);
    expect(mode.twoFingerCanvasMovement, isTrue);
    expect(store['canvas'], 'Y');
  });
  for (final touch in [false, true]) {
    testWidgets('UI checkbox controls the same session raw pointer gate touch=$touch', (tester) async {
      final ffi = pointer.FakeController(touch);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: GestureHelp(
        touchMode: touch,
        onTouchModeChange: (_) {},
        virtualMouseMode: ffi.ffiModel.virtualMouseMode,
        android: true,
        translator: identity,
      ))));
      await tester.tap(find.text('Two-finger canvas movement'));
      await tester.pump();
      expect(ffi.ffiModel.virtualMouseMode.twoFingerCanvasMovement, isTrue);
      final pointers = await pointer.startTouches(tester, ffi);
      await pointer.move(tester, pointers, const Offset(30, 0));
      await pointer.move(tester, pointers, const Offset(20, 0));
      expect(ffi.canvasModel.pans.where((value) => value != 0), isNotEmpty);
      await pointer.finish(tester, pointers);
    });
  }
  for (final value in ['', 'N', 'true', 'invalid', 'Y']) {
    test('only explicit Y enables stored canvas option: $value', () {
      final mode = StoredCanvasMode({'canvas': value})..loadCanvasMovementOption();
      expect(mode.twoFingerCanvasMovement, value == 'Y');
    });
  }
  for (final touch in [false, true]) {
    testWidgets('canvas checkbox defaults off, persists and survives mode switch touch=$touch', (tester) async {
      final store = <String, String>{};
      final mode = StoredCanvasMode(store)..loadCanvasMovementOption();
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: GestureHelp(
        touchMode: touch,
        onTouchModeChange: (_) {},
        virtualMouseMode: mode,
        android: true,
        translator: identity,
      ))));
      final label = find.text('Two-finger canvas movement');
      expect(label, findsOneWidget);
      final checkbox = find.byKey(const ValueKey('two-finger-canvas-movement'));
      expect(tester.widget<Checkbox>(checkbox).value, isFalse);
      expect(mode.twoFingerCanvasMovement, isFalse);
      expect((tester.getCenter(label).dy - tester.getCenter(find.text('Show virtual mouse')).dy).abs(), lessThan(60));
      await tester.tap(label);
      await tester.pump();
      expect(mode.twoFingerCanvasMovement, isTrue);
      expect(store['canvas'], 'Y');
      expect(find.text('Canvas Move'), findsOneWidget);
      await tester.tap(find.text(touch ? 'Mouse mode' : 'Touch mode'));
      await tester.pumpAndSettle();
      expect(tester.widget<Checkbox>(checkbox).value, isTrue);
      final reloaded = StoredCanvasMode(store)..loadCanvasMovementOption();
      expect(reloaded.twoFingerCanvasMovement, isTrue);
      await tester.tap(checkbox);
      await tester.pump();
      expect(mode.twoFingerCanvasMovement, isFalse);
      expect(store['canvas'], 'N');
      expect(find.text('Canvas Move'), findsNothing);
      await tester.tap(find.text(touch ? 'Touch mode' : 'Mouse mode'));
      await tester.pumpAndSettle();
      expect(tester.widget<Checkbox>(checkbox).value, isFalse);
      expect(tester.takeException(), isNull);
    });
  }
}
