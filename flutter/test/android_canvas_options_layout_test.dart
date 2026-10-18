import 'package:flutter/material.dart';
import 'package:flutter_hbb/mobile/widgets/gesture_help.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'android_touch_pointer_widget_test.dart' as pointer;

const labels = ['Show virtual mouse', 'Two-finger canvas movement', 'Two-finger canvas scaling'];

void main() {
  for (final android in [false, true]) {
    for (final value in ['', 'Y', 'N']) {
      test('virtual mouse default android=$android stored=$value', () {
        final mode = VirtualMouseMode(pointer.FakeModel(false));
        mode.setShowVirtualMouse(showVirtualMouseFromStoredOption(value: value, android: android));
        expect(mode.showVirtualMouse, value == 'Y');
      });
    }
  }
  for (final touch in [false, true]) {
    for (final narrow in [false, true]) {
      testWidgets('ordered responsive options touch=$touch narrow=$narrow', (tester) async {
        final semantics = tester.ensureSemantics();
        await tester.binding.setSurfaceSize(const Size(1600, 1600));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final mode = pointer.FakeController(touch).ffiModel.virtualMouseMode;
        mode.setShowVirtualMouse(showVirtualMouseFromStoredOption(value: '', android: true));
        String tr(String key) => narrow && labels.contains(key) ? '$key extended translated label' : key;
        await tester.pumpWidget(MaterialApp(home: Scaffold(body: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(width: narrow ? 320 : 1600, child: MediaQuery(
            data: MediaQueryData(size: const Size(1600, 1600), textScaler: TextScaler.linear(narrow ? 2 : 1)),
            child: SingleChildScrollView(child: GestureHelp(touchMode: touch, onTouchModeChange: (_) {}, virtualMouseMode: mode, android: true, translator: tr)),
          )),
        ))));
        expect(tester.takeException(), isNull);
        expect(tester.widget<Checkbox>(find.byKey(const ValueKey('show-virtual-mouse'))).value, isFalse);
        final rects = labels.map((key) => tester.getRect(find.text(tr(key)))).toList();
        if (narrow) {
          expect(rects[1].top, greaterThan(rects[0].top));
          expect(rects[2].top, greaterThan(rects[1].top));
          for (final rect in rects) { expect(rect.right, lessThanOrEqualTo(320)); }
        } else {
          expect(rects[1].center.dy, closeTo(rects[0].center.dy, 1));
          expect(rects[2].center.dy, closeTo(rects[0].center.dy, 1));
          expect(rects[1].left, greaterThan(rects[0].right));
          expect(rects[2].left, greaterThan(rects[1].right));
        }
        await tester.tap(find.text(tr(labels[1])));
        await tester.pump();
        expect(mode.twoFingerCanvasMovement, isTrue);
        expect(mode.twoFingerCanvasScaling, isFalse);
        expect(tester.getSemantics(find.byKey(const ValueKey('two-finger-canvas-movement'))),
            matchesSemantics(label: tr(labels[1]), hasCheckedState: true,
                isChecked: true, hasEnabledState: true, isEnabled: true,
                isFocusable: true, hasTapAction: true));
        semantics.dispose();
        expect(tester.takeException(), isNull);
      });
    }
  }
}
