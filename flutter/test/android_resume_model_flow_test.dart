import 'package:flutter/widgets.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/mobile/android_connection_resume.dart';
import 'package:flutter_hbb/models/input_model.dart';
import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:uuid/uuid.dart';

class _Input extends Fake implements InputModel {
  Object? reconnectBoundary;
  @override
  bool setRelativeMouseMode(bool value) {
    if (reconnectBoundary != null) throw reconnectBoundary!;
    return true;
  }
}

class _Session extends Fake implements FFI {
  @override
  final sessionId = const Uuid().v4obj();
  @override
  String id = 'fixture-peer';
  @override
  bool closed = false;
  @override
  final inputModel = _Input();
  @override
  final dialogManager = OverlayDialogManager();
}

class _Model extends FfiModel {
  _Model(_Session session) : super(WeakReference<FFI>(session));
  int retries = 0;
  final shownMessages = <Map<String, dynamic>>[];
  void manualReconnect(_Session session) =>
      super.reconnect(session.dialogManager, session.sessionId, false);
  @override
  Future<void> showMsgBox(SessionID id, String type, String title, String text,
      String link, bool hasRetry, OverlayDialogManager manager,
      {bool? hasCancel}) async {
    shownMessages.add({
      'type': type,
      'title': title,
      'text': text,
      'hasRetry': hasRetry,
    });
  }

  @override
  void reconnect(OverlayDialogManager manager, SessionID id, bool relay) {
    retries++;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('Android close hook is inert on a non Android model', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = false;
    final controller = model.createAndroidConnectionResume(session.id);
    model.closeAndroidConnectionResume();
    expect(controller.closed, isFalse);
  });

  test('changed peer cannot trigger the captured recovery callback', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    session.id = 'replacement-peer';
    expect(controller.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
    expect(model.retries, 0);
  });

  test('close intent blocks manual reconnect before native dispatch', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    model.createAndroidConnectionResume(session.id);
    model.closeAndroidConnectionResume();
    (session.inputModel as _Input).reconnectBoundary =
        StateError('closed session must not dispatch');
    model.manualReconnect(session);
    expect(model.retries, 0);
  });

  test('manual model reconnect consumes recovery before native dispatch', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    controller.deferFailure(AndroidConnectionFailure.dnsLookup);
    final boundary = StateError('native dispatch boundary');
    (session.inputModel as _Input).reconnectBoundary = boundary;
    expect(() => model.manualReconnect(session), throwsA(same(boundary)));
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
    expect(controller.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
  });

  test('disposing an old page does not close its replacement recovery', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final old = model.createAndroidConnectionResume(session.id);
    final current = model.createAndroidConnectionResume(session.id);
    current.lifecycle(AppLifecycleState.paused, android: true);
    current.deferFailure(AndroidConnectionFailure.dnsLookup);
    model.closeAndroidConnectionResume(expected: old);
    current.lifecycle(AppLifecycleState.resumed, android: true);
    expect(current.closed, isFalse);
    expect(model.retries, 1);
  });
  for (final text in [
    'Failed to connect to rendezvous server: failed to lookup address information: No address associated with hostname',
    'Reset by the peer',
    'Timeout',
  ]) {
    test('real model msgbox resumes once: $text', () async {
      final session = _Session();
      final model = _Model(session);
      model.androidConnectionResumePlatformForTest = true;
      final controller = model.createAndroidConnectionResume(session.id);
      controller.lifecycle(AppLifecycleState.paused, android: true);
      await model.startEventListener(session.sessionId, session.id)({
        'name': 'msgbox',
        'type': 'error',
        'title': 'Connection Error',
        'text': text,
        'link': '',
        'hasRetry': 'true',
      });
      expect(model.retries, 0);
      expect(model.shownMessages, isEmpty);
      controller.lifecycle(AppLifecycleState.resumed, android: true);
      controller.lifecycle(AppLifecycleState.resumed, android: true);
      expect(model.retries, 1);
    });
  }

  test('connection_ready clears real model pending failure', () async {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Timeout',
      'link': ''
    }, session.sessionId, session.id);
    await model.startEventListener(session.sessionId, session.id)({
      'name': 'connection_ready',
      'secure': 'true',
      'direct': 'true',
    });
    expect(model.shownMessages, isEmpty);
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
  });

  test('closed session and replaced controller never reconnect', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final old = model.createAndroidConnectionResume(session.id);
    old.lifecycle(AppLifecycleState.paused, android: true);
    old.failed(AndroidConnectionFailure.dnsLookup);
    final current = model.createAndroidConnectionResume(session.id);
    old.lifecycle(AppLifecycleState.resumed, android: true);
    current.lifecycle(AppLifecycleState.paused, android: true);
    current.failed(AndroidConnectionFailure.dnsLookup);
    session.closed = true;
    current.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
  });

  for (final text in [
    'Connection closed',
    'Wrong Password',
    'Connection not allowed',
    'Incoming only mode'
  ]) {
    test('unclassified or terminal text never arms recovery: $text', () {
      final session = _Session();
      final model = _Model(session);
      model.androidConnectionResumePlatformForTest = true;
      final controller = model.createAndroidConnectionResume(session.id);
      controller.lifecycle(AppLifecycleState.paused, android: true);
      model.handleMsgBox({
        'type': 'error',
        'title': 'Connection Error',
        'text': text,
        'link': ''
      }, session.sessionId, session.id);
      controller.lifecycle(AppLifecycleState.resumed, android: true);
      expect(model.retries, 0);
    });
  }

  test('user close rejects late restart and failure events', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Timeout',
      'link': '',
    }, session.sessionId, session.id);
    expect(model.shownMessages, isEmpty);
    model.closeAndroidConnectionResume();
    expect(controller.closed, isTrue);
    model.clear();
    for (final type in ['restarting-show', 'restarting']) {
      model.handleMsgBox({
        'type': type,
        'title': 'Connecting...',
        'text': '',
        'link': '',
      }, session.sessionId, session.id);
    }
    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Wrong Password',
      'link': '',
    }, session.sessionId, session.id);
    expect(model.shownMessages.single['text'], 'Wrong Password');
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
  });

  test('old listener cannot arm a replacement controller', () async {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    model.createAndroidConnectionResume(session.id);
    final oldListener = model.startEventListener(session.sessionId, session.id);
    final replacement = model.createAndroidConnectionResume(session.id);
    replacement.lifecycle(AppLifecycleState.paused, android: true);
    await oldListener({
      'name': 'msgbox',
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Timeout',
      'link': '',
    });
    replacement.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
    expect(model.shownMessages, isEmpty);
    await oldListener({
      'name': 'msgbox',
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Wrong Password',
      'link': '',
    });
    expect(model.shownMessages.single['text'], 'Wrong Password');
  });

  test('non Android events preserve the existing reconnect path', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = false;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.close();
    model.handleMsgBox({
      'type': 'restarting-show',
      'title': 'Connecting...',
      'text': '',
      'link': '',
    }, session.sessionId, session.id);
    expect(model.retries, 1);
  });

  test('healthy session preserves permissions and never retries', () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    model.setPermissions({'keyboard': true, 'clipboard': false});
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 0);
    expect(model.permissions, {'keyboard': true, 'clipboard': false});
  });

  test('foreground DNS error keeps the existing error presentation path',
      () async {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    model.createAndroidConnectionResume(session.id);
    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text':
          'failed to lookup address information: no address associated with hostname',
      'link': '',
    }, session.sessionId, session.id);

    expect(model.shownMessages, hasLength(1));
    expect(model.shownMessages.single['hasRetry'], isFalse);
    expect(model.retries, 0);
  });

  test('DNS error arriving after resume is consumed by one automatic retry',
      () {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    controller.lifecycle(AppLifecycleState.resumed, android: true);

    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text':
          'failed to lookup address information: no address associated with hostname',
      'link': '',
    }, session.sessionId, session.id);

    expect(model.retries, 1);
    expect(model.shownMessages, isEmpty);
  });

  test('DNS error after a retry was consumed returns to normal error handling',
      () async {
    final session = _Session();
    final model = _Model(session);
    model.androidConnectionResumePlatformForTest = true;
    final controller = model.createAndroidConnectionResume(session.id);
    controller.lifecycle(AppLifecycleState.paused, android: true);
    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text': 'Timeout',
      'link': '',
    }, session.sessionId, session.id);
    controller.lifecycle(AppLifecycleState.resumed, android: true);
    expect(model.retries, 1);

    model.handleMsgBox({
      'type': 'error',
      'title': 'Connection Error',
      'text':
          'failed to lookup address information: no address associated with hostname',
      'link': '',
    }, session.sessionId, session.id);

    expect(model.shownMessages, hasLength(1));
    expect(model.shownMessages.single['hasRetry'], isFalse);
    expect(model.retries, 1);
  });
}
