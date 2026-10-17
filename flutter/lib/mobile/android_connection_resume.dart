import 'package:flutter/widgets.dart';

enum AndroidConnectionFailure { dnsLookup, transportClosed, restartPending }

/// Retries background failures, or the first late DNS failure after resume, once.
class AndroidConnectionResume {
  AndroidConnectionResume({required this.onRetry});

  final bool Function() onRetry;
  bool _paused = false;
  bool _closed = false;
  bool _backgroundedSinceReady = false;
  bool _retryConsumed = false;
  AndroidConnectionFailure? _failure;

  bool get paused => _paused && !_closed;
  bool get closed => _closed;

  void failed(AndroidConnectionFailure failure) {
    deferFailure(failure);
  }

  bool deferFailure(AndroidConnectionFailure failure) {
    if (_closed || !_backgroundedSinceReady || _retryConsumed) return false;
    if (!_paused && failure != AndroidConnectionFailure.dnsLookup) return false;
    _failure = failure;
    if (_paused) return true;
    return _retry();
  }

  void connected() {
    _failure = null;
    _backgroundedSinceReady = false;
    _retryConsumed = false;
  }

  void retryStarted() {
    if (_closed) return;
    _retryConsumed = true;
    _failure = null;
  }

  bool _retry() {
    if (_closed || _retryConsumed || _failure == null) return false;
    _retryConsumed = true;
    _failure = null;
    final triggered = onRetry();
    if (!triggered && !_closed) _retryConsumed = false;
    return triggered;
  }

  void close() {
    _closed = true;
    _paused = false;
    _failure = null;
  }

  void lifecycle(AppLifecycleState state, {required bool android}) {
    if (!android || _closed) return;
    if (state == AppLifecycleState.paused) {
      _paused = true;
      _backgroundedSinceReady = true;
    } else if (state == AppLifecycleState.resumed) {
      _paused = false;
      if (_failure != null) {
        _retry();
      }
    }
  }
}

AndroidConnectionFailure? classifyAndroidConnectionFailure({
  required String type,
  required String title,
  required String text,
}) {
  if (type == 'error' && title == 'Connection Error') {
    if (text.contains('failed to lookup address information:') ||
        text.contains('Failed to look up host for ')) {
      return AndroidConnectionFailure.dnsLookup;
    }
    if (text == 'Reset by the peer' || text == 'Timeout') {
      return AndroidConnectionFailure.transportClosed;
    }
  }
  if (type == 'restarting' || type == 'restarting-show') {
    return AndroidConnectionFailure.restartPending;
  }
  return null;
}

bool shouldIgnoreClosedAndroidRecoveryEvent({
  required bool controllerClosed,
  required String type,
  required String title,
  required String text,
}) =>
    controllerClosed &&
    classifyAndroidConnectionFailure(type: type, title: title, text: text) !=
        null;
