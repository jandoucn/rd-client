import 'package:flutter/widgets.dart';
import 'package:flutter_hbb/mobile/android_connection_resume.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('rejected late DNS retry retains eligibility until accepted', () {
    var attempts = 0;
    final resume = AndroidConnectionResume(onRetry: () => ++attempts > 1);
    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
    expect(attempts, 1);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isTrue);
    expect(attempts, 2);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
    expect(attempts, 2);
  });

  test('rejected retry leaves the error on the normal presentation path', () {
    final resume = AndroidConnectionResume(onRetry: () => false);
    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
  });

  test('foreground transport and restart stay on their existing paths', () {
    var retries = 0;
    final resume = AndroidConnectionResume(onRetry: () { retries++; return true; });
    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.transportClosed), isFalse);
    expect(resume.deferFailure(AndroidConnectionFailure.restartPending), isFalse);
    expect(retries, 0);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isTrue);
    expect(retries, 1);
  });

  test('late DNS reports handled only for the first triggered retry', () {
    var retries = 0;
    final resume = AndroidConnectionResume(onRetry: () { retries++; return true; });
    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isTrue);
    expect(retries, 1);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
    expect(retries, 1);
  });

  test('manual retry consumes pending and late failure eligibility', () {
    var retries = 0;
    final resume = AndroidConnectionResume(onRetry: () { retries++; return true; });
    resume.lifecycle(AppLifecycleState.paused, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isTrue);
    resume.retryStarted();
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 0);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isFalse);
    resume.connected();
    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    expect(resume.deferFailure(AndroidConnectionFailure.dnsLookup), isTrue);
    expect(retries, 1);
  });

  test('recognizes the Android DNS lookup failure text', () {
    expect(
      classifyAndroidConnectionFailure(
        type: 'error',
        title: 'Connection Error',
        text:
            'failed to lookup address information: no address associated with hostname',
      ),
      AndroidConnectionFailure.dnsLookup,
    );
  });

  test('does not classify unrelated connection errors as DNS failures', () {
    for (final text in [
      'Remote desktop is offline',
      'Wrong Password',
      'Connection not allowed',
      'Incoming only mode',
    ]) {
      expect(
        classifyAndroidConnectionFailure(
          type: 'error',
          title: 'Connection Error',
          text: text,
        ),
        isNull,
        reason: text,
      );
    }
  });

  test('a DNS error after resume retries once after background interruption',
      () {
    var retries = 0;
    final resume = AndroidConnectionResume(onRetry: () { retries++; return true; });

    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    resume.failed(AndroidConnectionFailure.dnsLookup);
    resume.failed(AndroidConnectionFailure.dnsLookup);

    expect(retries, 1);
  });

  test('a healthy resume does not arm a retry', () {
    var retries = 0;
    final resume = AndroidConnectionResume(onRetry: () { retries++; return true; });

    resume.lifecycle(AppLifecycleState.paused, android: true);
    resume.lifecycle(AppLifecycleState.resumed, android: true);
    resume.connected();
    resume.failed(AndroidConnectionFailure.dnsLookup);

    expect(retries, 0);
  });

  test('closed recovery suppresses only its own retry events', () {
    expect(
      shouldIgnoreClosedAndroidRecoveryEvent(
        controllerClosed: true,
        type: 'restarting-show',
        title: 'Connecting...',
        text: '',
      ),
      isTrue,
    );
    expect(
      shouldIgnoreClosedAndroidRecoveryEvent(
        controllerClosed: true,
        type: 'input-password',
        title: 'Connection Error',
        text: 'input-password',
      ),
      isFalse,
    );
    expect(
      shouldIgnoreClosedAndroidRecoveryEvent(
        controllerClosed: false,
        type: 'error',
        title: 'Connection Error',
        text: 'Timeout',
      ),
      isFalse,
    );
  });
}
