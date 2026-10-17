import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/mobile/android_connection_resume.dart';

void main() {
  test('healthy Android app switches never reconnect', () {
    var retries = 0;
    final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    recovery.lifecycle(AppLifecycleState.paused, android: true);
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 0);
  });

  for (final failure in AndroidConnectionFailure.values) {
    test('background $failure retries once on resume without a timer', () {
      var retries = 0;
      final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
      recovery.lifecycle(AppLifecycleState.paused, android: true);
      recovery.failed(failure);
      recovery.failed(failure);
      expect(retries, 0);
      recovery.lifecycle(AppLifecycleState.resumed, android: true);
      recovery.lifecycle(AppLifecycleState.resumed, android: true);
      expect(retries, 1);
      recovery.lifecycle(AppLifecycleState.paused, android: true);
      recovery.lifecycle(AppLifecycleState.resumed, android: true);
      expect(retries, 1);
    });
  }

  test('foreground errors and inactive alone do not arm recovery', () {
    var retries = 0;
    final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
    recovery.failed(AndroidConnectionFailure.dnsLookup);
    recovery.lifecycle(AppLifecycleState.inactive, android: true);
    recovery.failed(AndroidConnectionFailure.transportClosed);
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 0);
  });

  test('successful transport clears obsolete background error', () {
    var retries = 0;
    final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
    recovery.lifecycle(AppLifecycleState.paused, android: true);
    recovery.failed(AndroidConnectionFailure.dnsLookup);
    recovery.connected();
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 0);
  });

  test('disconnect is terminal even if late errors arrive', () {
    var retries = 0;
    final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
    recovery.lifecycle(AppLifecycleState.paused, android: true);
    recovery.failed(AndroidConnectionFailure.dnsLookup);
    recovery.close();
    recovery.failed(AndroidConnectionFailure.restartPending);
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    recovery.lifecycle(AppLifecycleState.paused, android: true);
    recovery.failed(AndroidConnectionFailure.transportClosed);
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 0);
  });

  test('iOS and desktop lifecycle never arm recovery', () {
    var retries = 0;
    final recovery = AndroidConnectionResume(onRetry: () { retries++; return true; });
    recovery.lifecycle(AppLifecycleState.paused, android: false);
    recovery.failed(AndroidConnectionFailure.dnsLookup);
    recovery.lifecycle(AppLifecycleState.resumed, android: false);
    expect(retries, 0);
  });

  test('retry is consumed before reentrant notification', () {
    var retries = 0;
    late AndroidConnectionResume recovery;
    recovery = AndroidConnectionResume(onRetry: () {
      retries++;
      recovery.failed(AndroidConnectionFailure.dnsLookup);
      recovery.lifecycle(AppLifecycleState.resumed, android: true);
      return true;
    });
    recovery.lifecycle(AppLifecycleState.paused, android: true);
    recovery.failed(AndroidConnectionFailure.dnsLookup);
    recovery.lifecycle(AppLifecycleState.resumed, android: true);
    expect(retries, 1);
  });
}
