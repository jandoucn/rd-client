import 'package:flutter/widgets.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_hbb/common/widgets/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  for (final touch in [false, true]) {
    for (final direction in [-1.0, 1.0]) {
      test('eight pixel accumulation touch=$touch direction=$direction', () {
        var integral = 0.0;
        var ticks = 0;
        for (var step = 1; step <= 18; step++) {
          final result = androidTwoFingerWheel(
              integral: integral, delta: Offset(0, direction),
              pointerCount: 2, androidController: true, touchMode: touch);
          expect(result.wheel, step % 9 == 0 ? direction.toInt() : 0);
          expect(result.integral, direction * (step % 9) / 8);
          integral = result.integral;
          ticks += result.wheel.abs();
        }
        expect(ticks, 2);
      });
    }
  }
  testWidgets('arena rejection clears two finger remainder', (tester) async {
    final recognizer = CustomTouchGestureRecognizer();
    var cancelled = 0;
    recognizer.onTwoFingerScaleCancel = () => cancelled++;
    recognizer.onUpdate!(ScaleUpdateDetails(pointerCount: 2));
    recognizer.rejectGesture(1);
    expect(cancelled, 1);
    recognizer.dispose();
  });
  testWidgets('two finger recognizer retains scale and pan details',
      (tester) async {
    final recognizer = CustomTouchGestureRecognizer();
    final updates = <ScaleUpdateDetails>[];
    recognizer.onTwoFingerScaleUpdate = updates.add;
    final details = ScaleUpdateDetails(
        pointerCount: 2, scale: 1.5, focalPointDelta: const Offset(7, 9));
    recognizer.onUpdate!(details);
    expect(updates, [details]);
    recognizer.dispose();
  });
  testWidgets('three finger callback retains existing delta', (tester) async {
    final recognizer = CustomTouchGestureRecognizer();
    final updates = <Offset>[];
    recognizer.onThreeFingerVerticalDragUpdate = (d) => updates.add(d.delta);
    recognizer.onUpdate!(ScaleUpdateDetails(
        pointerCount: 3, focalPointDelta: const Offset(7, 9)));
    expect(updates, [const Offset(7, 9)]);
    recognizer.dispose();
  });
  test('vertical movement accumulates with strict existing wheel threshold',
      () {
    var result = androidTwoFingerWheel(
        integral: 0,
        delta: const Offset(0, 8),
        pointerCount: 2,
        androidController: true,
        touchMode: false);
    expect(result, (integral: 1.0, wheel: 0));
    result = androidTwoFingerWheel(
        integral: result.integral,
        delta: const Offset(0, 1),
        pointerCount: 2,
        androidController: true,
        touchMode: false);
    expect(result, (integral: 0.0, wheel: 1));
  });
  test('negative and large movements emit one wheel step and reset', () {
    for (final dy in [-9.0, -100.0, 9.0, 100.0]) {
      expect(
          androidTwoFingerWheel(
              integral: 0,
              delta: Offset(0, dy),
              pointerCount: 2,
              androidController: true,
              touchMode: false),
          (integral: 0.0, wheel: dy.sign.toInt()));
    }
  });
  test('reversal cancels accumulated movement', () {
    expect(
        androidTwoFingerWheel(
            integral: 0.75,
            delta: const Offset(0, -6),
            pointerCount: 2,
            androidController: true,
            touchMode: false),
        (integral: 0.0, wheel: 0));
  });
  test('negative exact threshold waits for additional movement', () {
    final result = androidTwoFingerWheel(
      integral: 0,
      delta: const Offset(0, -8),
      pointerCount: 2,
      androidController: true,
      touchMode: false,
    );
    expect(result, (integral: -1.0, wheel: 0));
  });
  test('pointer change and cancelled gesture cannot carry a remainder', () {
    for (final boundary in [false, true]) {
      final reset = androidTwoFingerWheel(
        integral: 0.75,
        delta: const Offset(0, 100),
        pointerCount: boundary ? 2 : 1,
        androidController: true,
        touchMode: false,
        boundary: boundary,
      );
      final next = androidTwoFingerWheel(
        integral: reset.integral,
        delta: const Offset(0, 2),
        pointerCount: 2,
        androidController: true,
        touchMode: false,
      );
      expect(reset, (integral: 0.0, wheel: 0));
      expect(next, (integral: 0.25, wheel: 0));
    }
  });
  test('horizontal movement is ignored and diagonal uses vertical only', () {
    expect(
        androidTwoFingerWheel(
                integral: 0,
                delta: const Offset(100, 0),
                pointerCount: 2,
                androidController: true,
                touchMode: false)
            .wheel,
        0);
    expect(
        androidTwoFingerWheel(
                integral: 0,
                delta: const Offset(100, 9),
                pointerCount: 2,
                androidController: true,
                touchMode: false)
            .wheel,
        1);
  });

  test('Android touch mode vertical motion can produce a wheel step', () {
    expect(
      androidTwoFingerWheel(
        integral: 0,
        delta: const Offset(0, 9),
        pointerCount: 2,
        androidController: true,
        touchMode: true,
      ),
      (integral: 0.0, wheel: 1),
    );
  });
  for (final count in [0, 1, 3, 4]) {
    test('pointer count $count clears accumulated movement', () {
      expect(
          androidTwoFingerWheel(
              integral: 1,
              delta: const Offset(0, 100),
              pointerCount: count,
              androidController: true,
              touchMode: false),
          (integral: 0.0, wheel: 0));
    });
  }
  for (final gate in ['boundary', 'hold', 'camera', 'touch', 'nonAndroid']) {
    test('$gate suppresses wheel and clears remainder', () {
      expect(
          androidTwoFingerWheel(
              integral: 1,
               delta: gate == 'touch'
                   ? const Offset(100, 0)
                   : const Offset(0, 100),
               pointerCount: 2,
               androidController: gate != 'nonAndroid',
              touchMode: gate == 'touch',
              boundary: gate == 'boundary',
              specialHoldDrag: gate == 'hold',
              camera: gate == 'camera'),
          (integral: 0.0, wheel: 0));
    });
  }
}
