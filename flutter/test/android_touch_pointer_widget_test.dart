import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hbb/common/widgets/remote_input.dart';
import 'package:flutter_hbb/models/input_model.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';

class TestPlatform extends TouchInputPlatform {
  const TestPlatform({this.android = true});
  final bool android;
  @override
  bool get isAndroid => android;
  @override
  bool get isMobile => true;
  @override
  bool get isDesktop => false;
  @override
  bool get isWebDesktop => false;
}

class FakeInput extends Fake implements InputModel {
  @override
  final relativeMouseMode = false.obs;
  final wheels = <int>[];
  @override
  Future<void> scroll(int y) async => wheels.add(y);
  @override
  Future<void> sendMouse(String type, MouseButtons button) async {}
  @override
  bool shouldIgnoreTouchTap(Offset position) => false;
}

class FakeModel extends Fake implements FfiModel {
  FakeModel(this.touchMode);
  @override
  late final PointerVirtualMouseMode virtualMouseMode = PointerVirtualMouseMode(this);
  @override
  final bool touchMode;
  @override
  bool get isPeerAndroid => false;
  @override
  bool get isPeerMobile => false;
}

class PointerVirtualMouseMode extends VirtualMouseMode {
  PointerVirtualMouseMode(super.model);
  String stored = '';
  String storedScale = '';
  @override
  String readCanvasScalingOption() => storedScale;
  @override
  Future<void> writeCanvasScalingOption(String value) async { storedScale = value; }
  @override
  String readCanvasMovementOption() => stored;
  @override
  Future<void> writeCanvasMovementOption(String value) async { stored = value; }
}

class FakeCanvas extends Fake implements CanvasModel {
  final scales = <double>[];
  final pans = <double>[];
  @override
  bool get locked => false;
  @override
  dynamic updateScale(double value, Offset focalPoint) => scales.add(value);
  @override
  dynamic panX(double value) => pans.add(value);
  @override
  dynamic panY(double value) => pans.add(value);
}

class FakeCursor extends Fake implements CursorModel {
  @override
  Offset get offset => const Offset(200, 200);
  @override
  Rect getVisibleRect() => const Rect.fromLTWH(0, 0, 800, 600);
  @override
  bool isInRemoteRect(Offset position) => true;
  final drags = <Offset>[];
  @override
  bool shouldBlock(double x, double y) => false;
  @override
  dynamic updatePan(Offset delta, Offset position, bool touchMode) async {
    drags.add(delta);
  }
}

class FakeController extends Fake implements FFI {
  FakeController(bool touch) : ffiModel = FakeModel(touch);
  @override
  final FakeInput inputModel = FakeInput();
  @override
  final FakeModel ffiModel;
  @override
  final FakeCanvas canvasModel = FakeCanvas();
  @override
  final FakeCursor cursorModel = FakeCursor();
}

Future<List<TestPointer>> startTouches(WidgetTester tester, FakeController ffi,
    {bool android = true, bool camera = false, int count = 2,
    ValueChanged<Map<String, Object?>>? trace,
    Offset spacing = const Offset(100, 0)}) async {
  await tester.pumpWidget(MaterialApp(
      home: RawTouchGestureDetectorRegion(
    ffi: ffi,
    platform: TestPlatform(android: android),
    debugScaleTrace: trace,
    isCamera: camera,
    child: const SizedBox.expand(child: ColoredBox(color: Colors.white)),
  )));
  final pointers =
      List.generate(count, (i) => TestPointer(i + 1, PointerDeviceKind.touch));
  for (var i = 0; i < count; i++) {
    await tester
        .sendEventToBinding(pointers[i].down(const Offset(200, 200) + spacing * i.toDouble()));
  }
  await tester.pump(const Duration(milliseconds: 120));
  return pointers;
}

Future<void> move(
    WidgetTester tester, List<TestPointer> pointers, Offset delta) async {
  for (final pointer in pointers) {
    await tester.sendEventToBinding(pointer.move(pointer.location! + delta));
  }
  await tester.pump();
}

Future<void> finish(WidgetTester tester, List<TestPointer> pointers,
    {bool cancel = false}) async {
  for (final pointer in pointers) {
    if (pointer.isDown) {
      await tester.sendEventToBinding(cancel ? pointer.cancel() : pointer.up());
    }
  }
  await tester.pump(const Duration(milliseconds: 250));
  await tester.pumpWidget(const SizedBox());
}

void main() {
  tearDown(() => isSpecialHoldDragActive = false);
  for (final touch in [false, true]) {
    testWidgets('enabled canvas option retains wheel priority and pinch touch=$touch', (tester) async {
      final ffi = FakeController(touch);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
      await ffi.ffiModel.virtualMouseMode.toggleCanvasMovement();
      var pointers = await startTouches(tester, ffi);
      await move(tester, pointers, const Offset(0, 30));
      await move(tester, pointers, const Offset(0, 20));
      expect(ffi.inputModel.wheels, isNotEmpty);
      expect(ffi.canvasModel.pans, isEmpty);
      expect(ffi.canvasModel.scales, isEmpty);
      await finish(tester, pointers);
      pointers = await startTouches(tester, ffi);
      await tester.sendEventToBinding(pointers[1].move(const Offset(350, 200)));
      await tester.pump();
      await tester.sendEventToBinding(pointers[1].move(const Offset(390, 200)));
      await tester.pump();
      expect(ffi.canvasModel.scales.any((value) => value > 1), isTrue);
      await finish(tester, pointers);
    });
    testWidgets('explicit canvas option allows pan and disabling stops pan touch=$touch',
        (tester) async {
      final ffi = FakeController(touch);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasMovement();
      final pointers = await startTouches(tester, ffi);
      await move(tester, pointers, const Offset(30, 0));
      await move(tester, pointers, const Offset(20, 0));
      expect(ffi.canvasModel.pans.where((value) => value != 0), isNotEmpty);
      expect(ffi.inputModel.wheels, isEmpty);
      ffi.canvasModel.pans.clear();
      await ffi.ffiModel.virtualMouseMode.toggleCanvasMovement();
      await move(tester, pointers, const Offset(20, 0));
      expect(ffi.canvasModel.pans, isEmpty);
      await finish(tester, pointers);
    });
  }
  for (final touch in [false, true]) {
    testWidgets('fixed finger continuous pinch returns to one touch=$touch',
        (tester) async {
      final ffi = FakeController(touch);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
      final pointers = await startTouches(tester, ffi);
      for (var step = 0; step < 8; step++) {
        await tester.sendEventToBinding(
            pointers[1].move(pointers[1].location! + const Offset(12, 0)));
        await tester.pump(const Duration(milliseconds: 40));
      }
      expect(ffi.canvasModel.scales.length, greaterThan(1));
      expect(ffi.canvasModel.scales.any((value) => value > 1), isTrue);
      await tester.sendEventToBinding(pointers[1].move(const Offset(300, 200)));
      await tester.pump();
      final scale = ffi.canvasModel.scales.fold<double>(1, (a, b) => a * b);
      expect(scale, closeTo(1, 0.000001));
      expect(ffi.canvasModel.pans, isEmpty);
      await finish(tester, pointers);
    });
  }
  for (final spacing in [const Offset(0, 100), const Offset(100, 0), const Offset(20, 100)]) {
    for (final delta in [const Offset(30, 0), const Offset(30, 20)]) {
      testWidgets('Android mouse parallel cannot pan spacing=$spacing delta=$delta', (tester) async {
        final ffi = FakeController(false);
        await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
        final pointers = await startTouches(tester, ffi, spacing: spacing);
        await move(tester, pointers, delta);
        await move(tester, pointers, delta);
        expect(ffi.canvasModel.scales, isNotEmpty);
        expect(ffi.canvasModel.pans, isEmpty);
        expect(ffi.inputModel.wheels, isEmpty);
        await finish(tester, pointers);
      });
    }
  }
  for (final touch in [false, true]) {
    for (final spacing in [const Offset(0, 100), const Offset(100, 0), const Offset(20, 100)]) {
      testWidgets('batched parallel pointers touch=$touch spacing=$spacing', (tester) async {
        final ffi = FakeController(touch);
        var traceCount = 0;
        final pointers = await startTouches(tester, ffi, spacing: spacing,
            trace: !touch && spacing == const Offset(100, 0)
                ? (event) {
                    if (traceCount++ < 48) debugPrint('ANDROID_SCALE_TRACE $event');
                  }
                : null);
        await move(tester, pointers, const Offset(0, 30));
        final previousWheels = ffi.inputModel.wheels.length;
        for (final pointer in pointers) {
          await tester.sendEventToBinding(
              pointer.move(pointer.location! + const Offset(0, 20)));
          expect(ffi.inputModel.wheels.length, previousWheels);
        }
        expect(tester.binding.hasScheduledFrame, isTrue);
        await tester.pump();
        expect(ffi.inputModel.wheels.length, greaterThan(previousWheels));
        expect(ffi.inputModel.wheels, isNotEmpty);
        expect(ffi.inputModel.wheels, everyElement(1));
        expect(ffi.canvasModel.scales, isEmpty);
        expect(ffi.canvasModel.pans, isEmpty);
        await finish(tester, pointers);
      });
      testWidgets('batched opposite pointers pinch touch=$touch spacing=$spacing', (tester) async {
        final ffi = FakeController(touch);
        await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
        final pointers = await startTouches(tester, ffi, spacing: spacing);
        final spread = spacing / spacing.distance * 30;
        await tester.sendEventToBinding(pointers[0].move(pointers[0].location! - spread));
        await tester.sendEventToBinding(pointers[1].move(pointers[1].location! + spread));
        await tester.pump();
        expect(ffi.inputModel.wheels, isEmpty);
        expect(ffi.canvasModel.scales.any((value) => value > 1.05), isTrue);
        expect(ffi.canvasModel.pans, isEmpty);
        await finish(tester, pointers);
      });
    }
  }
  for (final touch in [false, true]) {
    for (final direction in [-1.0, 1.0]) {
      testWidgets('two modes vertical touch=$touch direction=$direction',
          (tester) async {
        final ffi = FakeController(touch);
        final pointers = await startTouches(tester, ffi);
        await move(tester, pointers, Offset(0, 30 * direction));
        await move(tester, pointers, Offset(0, 20 * direction));
        expect(ffi.inputModel.wheels, isNotEmpty);
        expect(ffi.inputModel.wheels, everyElement(direction.toInt()));
        expect(ffi.canvasModel.scales, isEmpty);
        expect(ffi.canvasModel.pans, isEmpty);
        await finish(tester, pointers);
      });
    }
    testWidgets('pinch remains canvas when held and translated touch=$touch',
        (tester) async {
      final ffi = FakeController(touch);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
      final pointers = await startTouches(tester, ffi);
      await tester.sendEventToBinding(
          pointers[1].move(const Offset(380, 200)));
      await tester.pump();
      expect(ffi.canvasModel.scales.any((scale) => scale > 1.05), isTrue);
      await move(tester, pointers, const Offset(0, 30));
      await move(tester, pointers, const Offset(0, 20));
      expect(ffi.inputModel.wheels, isEmpty);
      expect(ffi.canvasModel.pans, isEmpty);
      await finish(tester, pointers);
    });
    testWidgets('camera vertical remains canvas touch=$touch', (tester) async {
      final ffi = FakeController(touch);
      final pointers = await startTouches(tester, ffi, camera: true);
      await move(tester, pointers, const Offset(0, 30));
      await move(tester, pointers, const Offset(0, 20));
      expect(ffi.inputModel.wheels, isEmpty);
      expect(ffi.canvasModel.scales, isNotEmpty);
      expect(ffi.canvasModel.pans.where((v) => v != 0), isNotEmpty);
      await finish(tester, pointers);
    });
    testWidgets('cancel and count boundary cannot emit wheel touch=$touch',
        (tester) async {
      final ffi = FakeController(touch);
      final pointers = await startTouches(tester, ffi);
      await move(tester, pointers, const Offset(0, 30));
      await move(tester, pointers, const Offset(0, 20));
      expect(ffi.inputModel.wheels, isNotEmpty);
      final count = ffi.inputModel.wheels.length;
      await tester.sendEventToBinding(pointers[1].cancel());
      await tester.sendEventToBinding(
          pointers[0].move(pointers[0].location! + const Offset(0, 60)));
      await tester.pump();
      expect(ffi.inputModel.wheels.length, count);
      await finish(tester, pointers, cancel: true);
      expect(ffi.inputModel.wheels.length, count);
    });
    testWidgets('three pointers do not use Android wheel touch=$touch',
        (tester) async {
      final ffi = FakeController(touch);
      final pointers = await startTouches(tester, ffi, count: 3);
      await move(tester, pointers, const Offset(0, 30));
      await move(tester, pointers, const Offset(0, 20));
      expect(ffi.inputModel.wheels, isEmpty);
      await finish(tester, pointers, cancel: true);
    });
  }
  for (final direction in [1.0, -1.0]) {
    testWidgets('raw Android pointers scroll $direction without canvas changes',
        (tester) async {
      final ffi = FakeController(false);
      final pointers = await startTouches(tester, ffi);
      await move(tester, pointers, Offset(0, 30 * direction));
      await move(tester, pointers, Offset(0, 20 * direction));
      expect(ffi.inputModel.wheels, isNotEmpty);
      expect(ffi.inputModel.wheels, everyElement(direction.toInt()));
      expect(ffi.canvasModel.scales, isEmpty);
      expect(ffi.canvasModel.pans, isEmpty);
      final count = ffi.inputModel.wheels.length;
      await finish(tester, pointers);
      expect(ffi.inputModel.wheels.length, count);
    });
  }
  testWidgets('raw cancel and remaining single pointer cannot scroll',
      (tester) async {
    final ffi = FakeController(false);
    final pointers = await startTouches(tester, ffi);
    await move(tester, pointers, const Offset(0, 30));
    await move(tester, pointers, const Offset(0, 20));
    expect(ffi.inputModel.wheels, isNotEmpty);
    final count = ffi.inputModel.wheels.length;
    await tester.sendEventToBinding(pointers[1].cancel());
    await tester.sendEventToBinding(pointers[0].move(const Offset(200, 400)));
    await tester.pump();
    expect(ffi.inputModel.wheels.length, count);
    expect(ffi.canvasModel.scales, isEmpty);
    await finish(tester, pointers, cancel: true);
  });
  for (final android in [true, false]) {
    testWidgets('raw two pointers retain mobile canvas android=$android',
        (tester) async {
      final ffi = FakeController(android);
      await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
      final pointers = await startTouches(tester, ffi, android: android);
      await move(tester, pointers, const Offset(30, 0));
      await tester.sendEventToBinding(pointers[1].move(const Offset(350, 250)));
      await tester.pump();
      expect(ffi.canvasModel.scales, isNotEmpty);
      if (android) {
        expect(ffi.canvasModel.pans, isEmpty);
      } else {
        expect(ffi.canvasModel.pans.where((v) => v != 0), isNotEmpty);
      }
      expect(ffi.inputModel.wheels, isEmpty);
      await finish(tester, pointers);
    });
  }
  testWidgets('raw Android touch-mode vertical swipe scrolls without zoom',
      (tester) async {
    final ffi = FakeController(true);
    final pointers = await startTouches(tester, ffi);
    await move(tester, pointers, const Offset(0, 30));
    await move(tester, pointers, const Offset(0, 20));
    expect(ffi.inputModel.wheels, isNotEmpty);
    expect(ffi.inputModel.wheels, everyElement(1));
    expect(ffi.canvasModel.scales, isEmpty);
    expect(ffi.canvasModel.pans, isEmpty);
    await finish(tester, pointers);
  });
  testWidgets('raw Android touch-mode horizontal swipe cannot pan without scroll',
      (tester) async {
    final ffi = FakeController(true);
    await ffi.ffiModel.virtualMouseMode.toggleCanvasScaling();
    final pointers = await startTouches(tester, ffi);
    await move(tester, pointers, const Offset(30, 0));
    await move(tester, pointers, const Offset(20, 0));
    expect(ffi.inputModel.wheels, isEmpty);
    expect(ffi.canvasModel.scales, isNotEmpty);
    expect(ffi.canvasModel.pans, isEmpty);
    await finish(tester, pointers);
  });
  testWidgets('raw iOS three pointers retain wheel path', (tester) async {
    final ffi = FakeController(false);
    final pointers = await startTouches(tester, ffi, android: false, count: 3);
    await move(tester, pointers, const Offset(0, 30));
    await move(tester, pointers, const Offset(0, 20));
    expect(ffi.inputModel.wheels, isNotEmpty);
    await finish(tester, pointers);
  });
  testWidgets('raw special hold drag takes priority', (tester) async {
    final ffi = FakeController(false);
    isSpecialHoldDragActive = true;
    final pointers = await startTouches(tester, ffi);
    await move(tester, pointers, const Offset(0, 30));
    await move(tester, pointers, const Offset(0, 20));
    expect(ffi.cursorModel.drags, isNotEmpty);
    expect(ffi.inputModel.wheels, isEmpty);
    expect(ffi.canvasModel.scales, isEmpty);
    await finish(tester, pointers);
  });
  testWidgets('raw Android mouse-mode camera pointers retain canvas gestures',
      (tester) async {
    final ffi = FakeController(false);
    final pointers = await startTouches(tester, ffi, camera: true);
    await move(tester, pointers, const Offset(0, 30));
    await move(tester, pointers, const Offset(0, 20));
    expect(ffi.inputModel.wheels, isEmpty);
    expect(ffi.canvasModel.scales, isNotEmpty);
    expect(ffi.canvasModel.pans.where((value) => value != 0), isNotEmpty);
    await finish(tester, pointers);
  });
}
