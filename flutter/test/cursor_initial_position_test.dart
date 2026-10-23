import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/models/input_model.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:uuid/uuid.dart';

/// Stands in for the core: records the moves that would reach the peer.
class _RecordingInput extends InputModel {
  _RecordingInput(WeakReference<FFI> parent) : super(parent);
  final moves = <(double, double)>[];

  @override
  Future<void> moveMouse(double x, double y) async {
    moves.add((x, y));
  }
}

class _FFI extends Fake implements FFI {
  _FFI() {
    ffiModel = FfiModel(WeakReference(this));
    cursorModel = CursorModel(WeakReference(this));
    canvasModel = CanvasModel(WeakReference(this));
    inputModel = _RecordingInput(WeakReference(this));
  }

  @override
  final SessionID sessionId = const Uuid().v4obj();
  @override
  final ConnType connType = ConnType.defaultConn;
  @override
  late final FfiModel ffiModel;
  @override
  late final CursorModel cursorModel;
  @override
  late final CanvasModel canvasModel;
  @override
  late final InputModel inputModel;

  _RecordingInput get recorder => inputModel as _RecordingInput;
}

void main() {
  test('the first frame does not drag the peer cursor to a corner', () async {
    // Every controller runs this on the first image. Sending that move puts the peer's real
    // pointer at the display origin: harmless on Windows, a hot corner on macOS, and a lost
    // position everywhere.
    final ffi = _FFI();
    await initializeCursorAndCanvas(ffi);
    expect(ffi.recorder.moves, isEmpty);
  });

  test('restoring a saved cursor position does not move the peer cursor', () {
    // The saved position is already what the user left behind; moving the peer to the display
    // origin throws that away before the session is even drawn.
    final ffi = _FFI();
    ffi.cursorModel.updateDisplayOriginWithCursor(0, 0, 640, 480);
    expect(ffi.recorder.moves, isEmpty);
    expect(ffi.cursorModel.x, 640);
    expect(ffi.cursorModel.y, 480);
  });

  test('switching display still moves the peer cursor onto the new display',
      () {
    // Explicit switch: the peer cursor has to leave the display that is no longer shown.
    final ffi = _FFI();
    ffi.cursorModel.updateDisplayOrigin(1920, 0);
    expect(ffi.recorder.moves, [(1920.0, 0.0)]);
  });

  test('a view style refresh does not re-send an unpositioned pointer', () {
    // A view style refresh re-sends the last pointer position so the peer cursor follows a
    // scroll or scale change. Before the user has moved over the image that position is still
    // the model's zero initial value, and sending it puts the peer cursor in the corner of its
    // display: the same jump as the password click used to cause, from a different call site.
    expect(peerCursorHasPosition(Offset.zero), isFalse);
    expect(peerCursorHasPosition(const Offset(0, 3)), isTrue);
    expect(peerCursorHasPosition(const Offset(640, 480)), isTrue);
  });
}
