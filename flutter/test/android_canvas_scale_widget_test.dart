import 'package:flutter/material.dart';
import 'dart:ui' as ui;
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_hbb/mobile/widgets/gesture_help.dart';
import 'package:flutter_test/flutter_test.dart';

import 'android_touch_pointer_widget_test.dart' as pointer;
import 'android_canvas_option_widget_test.dart' as option;

class ScaleStore extends option.StoredCanvasMode {
  ScaleStore(super.store);
  @override
  String readCanvasScalingOption() => store['scale'] ?? '';
  @override
  Future<void> writeCanvasScalingOption(String value) async {
    if (failWrite) throw StateError('fixture scale write failure');
    store['scale'] = value;
  }
}

class RealImage extends Fake implements ImageModel {
  RealImage(this.image);
  @override
  final ui.Image image;
  @override
  double get minScale => 0.1;
  @override
  double get maxScale => 10;
}

class RealCanvasController extends pointer.FakeController {
  RealCanvasController(bool touch, ui.Image image) : imageModel = RealImage(image), super(touch);
  @override
  final SessionID sessionId = SessionID('00000000-0000-4000-8000-000000000001');
  @override
  final RealImage imageModel;
  @override
  late final ActualCanvas canvasModel = ActualCanvas(WeakReference<FFI>(this));
}

class ActualCanvas extends CanvasModel implements pointer.FakeCanvas {
  ActualCanvas(super.parent);
  @override
  final scales = <double>[];
  @override
  final pans = <double>[];
  @override
  double getAdjustY() => 0;
  @override
  updateScale(double value, Offset focalPoint) {
    scales.add(value);
    super.updateScale(value, focalPoint);
  }
}

void main() {
  test('scale persists independently and missing scale preserves stored move Y', () async {
    final store = {'canvas': 'Y'};
    final mode = ScaleStore(store)..loadCanvasMovementOption()..loadCanvasScalingOption();
    expect(mode.twoFingerCanvasScaling, isFalse);
    expect(mode.twoFingerCanvasMovement, isTrue);
    await mode.toggleCanvasScaling();
    final reloaded = ScaleStore(store)..loadCanvasMovementOption()..loadCanvasScalingOption();
    expect(reloaded.twoFingerCanvasScaling, isTrue);
    expect(reloaded.twoFingerCanvasMovement, isTrue);
    await reloaded.toggleCanvasScaling();
    expect(store, {'canvas': 'Y', 'scale': 'N'});
    expect(mode.twoFingerCanvasMovement, isTrue);
  });
  for (final value in ['', 'N', 'true', 'invalid', 'Y']) {
    test('scale stored value is explicit Y only: $value', () {
      final mode = ScaleStore({'canvas': 'Y', 'scale': value})
        ..loadCanvasMovementOption()..loadCanvasScalingOption();
      expect(mode.twoFingerCanvasScaling, value == 'Y');
      expect(mode.twoFingerCanvasMovement, isTrue);
    });
  }
  test('scale persistence failure preserves both previous options', () async {
    final store = {'canvas': 'Y', 'scale': 'Y'};
    final mode = ScaleStore(store)..loadCanvasMovementOption()..loadCanvasScalingOption();
    mode.failWrite = true;
    await expectLater(mode.toggleCanvasScaling(), throwsStateError);
    expect(mode.twoFingerCanvasScaling, isTrue);
    expect(mode.twoFingerCanvasMovement, isTrue);
    expect(store, {'canvas': 'Y', 'scale': 'Y'});
  });
  for (final touch in [false, true]) {
    for (final movement in [false, true]) {
      for (final scaling in [false, true]) {
        testWidgets('independent canvas combination touch=$touch move=$movement scale=$scaling', (tester) async {
          final ffi = pointer.FakeController(touch);
          if (movement) await ffi.ffiModel.virtualMouseMode.toggleCanvasMovement();
          if (scaling) await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
          final pointers = await pointer.startTouches(tester, ffi);
          await pointer.move(tester, pointers, const Offset(30, 0));
          await tester.sendEventToBinding(pointers[1].move(pointers[1].location! + const Offset(50, 0)));
          await tester.pump();
          expect(ffi.canvasModel.pans.any((v) => v != 0), movement);
          expect(ffi.canvasModel.scales.isNotEmpty, scaling);
          if (scaling) expect(ffi.canvasModel.scales.any((v) => v > 1), isTrue);
          expect(ffi.inputModel.wheels, isEmpty);
          await pointer.finish(tester, pointers);
        });
      }
    }
    testWidgets('actual canvas offset and zoom unchanged across separate frames touch=$touch', (tester) async {
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawRect(const Rect.fromLTWH(0, 0, 10, 10), Paint());
      final image = await recorder.endRecording().toImage(10, 10);
      final ffi = RealCanvasController(touch, image);
      final pointers = await pointer.startTouches(tester, ffi, spacing: const Offset(0, 100));
      final initial = [ffi.canvasModel.x, ffi.canvasModel.y, ffi.canvasModel.scale];
      for (var step = 0; step < 3; step++) {
        for (final p in pointers) {
          await tester.sendEventToBinding(p.move(p.location! + const Offset(0, 40)));
          await tester.pump(const Duration(milliseconds: 40));
          expect([ffi.canvasModel.x, ffi.canvasModel.y, ffi.canvasModel.scale], initial);
        }
      }
      expect(ffi.canvasModel.scales, isEmpty);
      expect(ffi.inputModel.wheels, isNotEmpty);
      await pointer.finish(tester, pointers);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
      final pinch = await pointer.startTouches(tester, ffi);
      await tester.sendEventToBinding(pinch[1].move(const Offset(360, 200)));
      await tester.pump();
      expect(ffi.canvasModel.scale, greaterThan(initial[2]));
      expect([ffi.canvasModel.x, ffi.canvasModel.y], isNot(initial.sublist(0, 2)));
      await pointer.finish(tester, pinch);
      image.dispose();
      ffi.canvasModel.dispose();
    });
  }
  for (final touch in [false, true]) {
    testWidgets('default scale off blocks cross-frame scale jitter touch=$touch', (tester) async {
      final ffi = pointer.FakeController(touch);
      final pointers = await pointer.startTouches(tester, ffi,
          spacing: const Offset(0, 100));
      for (var step = 0; step < 3; step++) {
        for (final p in pointers) {
          await tester.sendEventToBinding(p.move(p.location! + const Offset(0, 40)));
          await tester.pump(const Duration(milliseconds: 40));
        }
      }
      expect(ffi.inputModel.wheels, isNotEmpty);
      expect(ffi.canvasModel.scales, isEmpty);
      expect(ffi.canvasModel.pans, isEmpty);
      await pointer.finish(tester, pointers);
    });
    testWidgets('scale checkbox adjacent to move defaults unchecked touch=$touch', (tester) async {
      final ffi = pointer.FakeController(touch);
      await tester.pumpWidget(MaterialApp(home: Scaffold(body: GestureHelp(
        touchMode: touch,
        onTouchModeChange: (_) {},
        virtualMouseMode: ffi.ffiModel.virtualMouseMode,
        android: true,
        translator: (value) => value,
      ))));
      final checkbox = find.byKey(const ValueKey('two-finger-canvas-scaling'));
      expect(checkbox, findsOneWidget);
      expect(tester.widget<Checkbox>(checkbox).value, isFalse);
      final label = find.text('Two-finger canvas scaling');
      expect(label, findsOneWidget);
      expect((tester.getCenter(label).dy - tester.getCenter(find.text('Two-finger canvas movement')).dy).abs(), lessThan(60));
      expect(find.text('Canvas Zoom'), findsNothing);
      await tester.tap(label);
      await tester.pump();
      expect(ffi.ffiModel.virtualMouseMode.twoFingerCanvasScaling, isTrue);
      expect(ffi.ffiModel.virtualMouseMode.twoFingerCanvasMovement, isFalse);
      expect(find.text('Canvas Zoom'), findsOneWidget);
      await tester.tap(find.text(touch ? 'Mouse mode' : 'Touch mode'));
      await tester.pumpAndSettle();
      expect(tester.widget<Checkbox>(checkbox).value, isTrue);
      await tester.tap(checkbox);
      await tester.pump();
      expect(ffi.ffiModel.virtualMouseMode.storedScale, 'N');
      expect(find.text('Canvas Zoom'), findsNothing);
      await tester.tap(find.text(touch ? 'Touch mode' : 'Mouse mode'));
      await tester.pumpAndSettle();
      expect(tester.widget<Checkbox>(checkbox).value, isFalse);
    });
  }
}
