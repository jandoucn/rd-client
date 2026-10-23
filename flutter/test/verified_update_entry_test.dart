import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/desktop/widgets/update_progress.dart';

void main() {
  test('desktop update entry delegates to the verified Rust updater', () async {
    String? capturedKey;
    String? capturedValue;

    await requestVerifiedUpdate(({required key, required value}) async {
      capturedKey = key;
      capturedValue = value;
    });

    expect(capturedKey, verifiedUpdateCommandKey);
    expect(capturedValue, isEmpty);
  });

  test('desktop update entry blocks only active install states', () {
    expect(isVerifiedUpdateBusy('started'), isTrue);
    expect(isVerifiedUpdateBusy('installing'), isTrue);
    expect(isVerifiedUpdateBusy('downloaded'), isFalse);
    expect(isVerifiedUpdateBusy('failed'), isFalse);
  });

  // The request must be recorded before it goes out, or every failure looks
  // remote and the desk user never gets the browser fallback.
  test('the desk update entry records the request before asking for it',
      () async {
    localUpdateRequestStartedAt.value = null;
    addTearDown(() => localUpdateRequestStartedAt.value = null);
    final at = DateTime(2026, 10, 24, 9);
    String? captured;
    var seenAtRequest = DateTime(2025);
    await handleUpdateWith(at, ({required key, required value}) async {
      captured = key;
      seenAtRequest = localUpdateRequestStartedAt.value ?? DateTime(2025);
    });
    expect(captured, verifiedUpdateCommandKey);
    expect(seenAtRequest, at, reason: 'the timestamp must exist while the '
        'request is in flight, not after it returns');
  });

  // Remote, policy or background updates stay silent: only an update the person
  // at the desk started in this window may fall back to the browser on failure.
  final t0 = DateTime(2026, 10, 22, 12);
  final failed = {
    'status': 'failed',
    'url': 'https://download.yan.life/a',
    'request_origin': 'manual',
  };
  final remoteFailed = {
    'status': 'failed',
    'url': 'https://download.yan.life/a',
    'request_origin': 'command_install',
  };
  final installCheckOk = {'request_origin': 'manual', 'url': 'https://a'};
  final settingsCheckEmpty = {
    'request_origin': 'manual',
    'request_id': 'manual-1',
    'url': '',
  };

  test('only a locally requested update may open the browser on failure', () {
    expect(
      localUpdateFailureMayOpenBrowser(failed,
          requestStartedAt: null, now: t0),
      isFalse,
    );
    expect(
      localUpdateFailureMayOpenBrowser(failed, requestStartedAt: t0, now: t0),
      isTrue,
    );
    expect(
      localUpdateFailureMayOpenBrowser({'status': 'installing'},
          requestStartedAt: t0, now: t0),
      isFalse,
    );
  });

  // A local request can be in flight while a command or policy install fails in
  // the same process. The time window alone would let that remote failure open
  // the browser, which is exactly what the silence rule forbids.
  test('a remote failure is silent even while a local request is pending', () {
    expect(
      localUpdateFailureMayOpenBrowser(remoteFailed,
          requestStartedAt: t0, now: t0),
      isFalse,
    );
    expect(
      localUpdateFailureMayOpenBrowser(
          {'status': 'failed', 'url': 'https://a'}, // no origin at all
          requestStartedAt: t0,
          now: t0),
      isFalse,
    );
    expect(
      localUpdateFailureMayOpenBrowser(
          {...failed, 'request_origin': 'system'},
          requestStartedAt: t0,
          now: t0),
      isFalse,
    );
  });

  test('a local update request expires after the timeout', () {
    final justBefore =
        t0.add(localUpdateRequestTimeout - const Duration(seconds: 1));
    final atLimit = t0.add(localUpdateRequestTimeout);
    expect(localUpdateRequestTimeout, const Duration(minutes: 30));
    expect(localUpdateRequestActive(t0, justBefore), isTrue);
    expect(localUpdateRequestActive(t0, atLimit), isFalse);
    expect(localUpdateRequestActive(null, t0), isFalse);
    expect(
      localUpdateRequestActive(t0, t0.subtract(const Duration(seconds: 1))),
      isFalse,
      reason: 'clock moved backwards',
    );
    expect(
      localUpdateFailureMayOpenBrowser(failed,
          requestStartedAt: t0, now: atLimit),
      isFalse,
    );
    expect(localUpdateRequestAfterEvent({'status': 'started'}, t0, atLimit),
        isNull);
    expect(localUpdateRequestAfterCheck(installCheckOk, t0, atLimit), isNull);
  });

  test('a local update request ends at its terminal status', () {
    for (final status in [
      'failed',
      'installed',
      'rolled_back',
      'rollback_failed',
      '',
    ]) {
      expect(localUpdateRequestAfterEvent({'status': status}, t0, t0), isNull,
          reason: status);
    }
    expect(localUpdateRequestAfterEvent({}, t0, t0), isNull);
    // `deferred` means the install is waiting for active sessions to end and
    // will be retried; the native side reports it again as `started`. Treating
    // it as terminal drops the fallback before the retry can fail.
    for (final status in ['started', 'downloaded', 'installing', 'deferred']) {
      expect(localUpdateRequestAfterEvent({'status': status}, t0, t0), t0,
          reason: status);
    }
    expect(localUpdateRequestAfterEvent({'status': 'started'}, null, t0),
        isNull);
  });

  test('only the install check identifies itself by an empty request id', () {
    expect(isLocalInstallUpdateCheck(installCheckOk), isTrue);
    expect(
        isLocalInstallUpdateCheck({...installCheckOk, 'request_id': ''}), isTrue);
    expect(isLocalInstallUpdateCheck(settingsCheckEmpty), isFalse);
    expect(isLocalInstallUpdateCheck({'request_origin': 'system'}), isFalse);
    expect(isLocalInstallUpdateCheck({'request_origin': 'command_install'}),
        isFalse);
  });

  test('a local update request ends when its own install check finds nothing',
      () {
    expect(
      localUpdateRequestAfterCheck(
          {'request_origin': 'manual', 'url': ''}, t0, t0),
      isNull,
    );
    expect(
      localUpdateRequestAfterCheck(
          {'request_origin': 'manual', 'url': 'https://a', 'error': 'x'},
          t0,
          t0),
      isNull,
    );
    expect(localUpdateRequestAfterCheck(installCheckOk, t0, t0), t0);
    expect(
      localUpdateRequestAfterCheck(
          {'request_origin': 'command', 'url': ''}, t0, t0),
      t0,
    );
    expect(localUpdateRequestAfterCheck(installCheckOk, null, t0), isNull);
  });

  test('a concurrent settings check does not clear the local install request',
      () {
    expect(localUpdateRequestAfterCheck(settingsCheckEmpty, t0, t0), t0);
    expect(
      localUpdateRequestAfterCheck(
          {...settingsCheckEmpty, 'url': 'https://a', 'error': 'timeout'},
          t0,
          t0),
      t0,
    );
  });

  group('checkUpdate event handler wiring', () {
    setUp(() => localUpdateRequestStartedAt.value = null);
    tearDown(() => localUpdateRequestStartedAt.value = null);

    test('a remote failure does not open the browser', () {
      expect(applyLocalUpdateEvent(failed, now: t0), isFalse);
      expect(localUpdateRequestStartedAt.value, isNull);
    });

    test('a local failure opens the browser once', () {
      localUpdateRequestStartedAt.value = t0;
      applyLocalUpdateCheckEvent(installCheckOk, now: t0);
      expect(applyLocalUpdateEvent({'status': 'started'}, now: t0), isFalse);
      expect(applyLocalUpdateEvent(failed, now: t0), isTrue);
      expect(localUpdateRequestStartedAt.value, isNull);
      expect(applyLocalUpdateEvent(failed, now: t0), isFalse);
    });

    test('a concurrent settings check keeps the local install request', () {
      localUpdateRequestStartedAt.value = t0;
      applyLocalUpdateCheckEvent(installCheckOk, now: t0);
      applyLocalUpdateCheckEvent(settingsCheckEmpty, now: t0);
      expect(localUpdateRequestStartedAt.value, t0);
      expect(applyLocalUpdateEvent(failed, now: t0), isTrue);
    });

    test('a stale local request does not open the browser', () {
      localUpdateRequestStartedAt.value = t0;
      final later = t0.add(localUpdateRequestTimeout);
      expect(applyLocalUpdateEvent(failed, now: later), isFalse);
      expect(localUpdateRequestStartedAt.value, isNull);
    });

    test('an install check that finds nothing ends the local request', () {
      localUpdateRequestStartedAt.value = t0;
      applyLocalUpdateCheckEvent({'request_origin': 'manual', 'url': ''},
          now: t0);
      expect(localUpdateRequestStartedAt.value, isNull);
      expect(applyLocalUpdateEvent(failed, now: t0), isFalse);
    });
  });
}
