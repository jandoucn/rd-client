import 'dart:async';

import 'package:get/get.dart';
import 'package:flutter_hbb/models/platform_model.dart';

const verifiedUpdateCommandKey = 'install-verified-update';
final desktopUpdateCardVisible = false.obs;

typedef UpdateCommand = Future<void> Function({
  required String key,
  required String value,
});

Future<void> requestVerifiedUpdate(UpdateCommand command) {
  return command(key: verifiedUpdateCommandKey, value: '');
}

bool isVerifiedUpdateBusy(String status) {
  return status == 'started' || status == 'installing';
}

// Set only while an update the person at the desk started is in flight; remote,
// policy and background updates never set it and therefore stay silent.
final localUpdateRequestStartedAt = Rxn<DateTime>();

// The native side may end a request without any terminal event; a stale request
// must not let a later unrelated failure open the browser.
const localUpdateRequestTimeout = Duration(minutes: 30);

void handleUpdate(String _) {
  unawaited(handleUpdateWith(DateTime.now(), bind.mainSetCommon));
}

// Split from `handleUpdate` so a test can prove the timestamp is recorded before
// the request goes out. Without it every failure would look remote and the desk
// user would get no browser fallback at all.
Future<void> handleUpdateWith(DateTime now, UpdateCommand command) async {
  localUpdateRequestStartedAt.value = now;
  await requestVerifiedUpdate(command);
}

bool localUpdateRequestActive(DateTime? startedAt, DateTime now) {
  if (startedAt == null) return false;
  final elapsed = now.difference(startedAt);
  return !elapsed.isNegative && elapsed < localUpdateRequestTimeout;
}

// A local request can be in flight while a command or policy install fails in
// the same process, so the time window alone cannot tell them apart. The native
// side stamps every local UI event with the origin of the request that produced
// it; anything that is not the desk's own request stays silent.
bool localUpdateFailureMayOpenBrowser(
  Map<String, dynamic> event, {
  required DateTime? requestStartedAt,
  required DateTime now,
}) =>
    localUpdateRequestActive(requestStartedAt, now) &&
    event['status']?.toString() == 'failed' &&
    event['request_origin']?.toString() == 'manual';

// `deferred` is not terminal: the native side postpones the install until the
// active sessions end and reports `started` again for the retry. Ending the
// request here would drop the browser fallback before that retry can fail.
const _localUpdateActiveStatuses = {'started', 'downloaded', 'installing', 'deferred'};

DateTime? localUpdateRequestAfterEvent(
  Map<String, dynamic> event,
  DateTime? startedAt,
  DateTime now,
) =>
    localUpdateRequestActive(startedAt, now) &&
            _localUpdateActiveStatuses.contains(event['status']?.toString())
        ? startedAt
        : null;

// `install-verified-update` checks with origin `manual` and no request id; the
// settings "Check for updates" button also uses `manual` but always sends one.
bool isLocalInstallUpdateCheck(Map<String, dynamic> event) =>
    event['request_origin']?.toString() == 'manual' &&
    (event['request_id']?.toString() ?? '').isEmpty;

DateTime? localUpdateRequestAfterCheck(
  Map<String, dynamic> event,
  DateTime? startedAt,
  DateTime now,
) {
  if (!localUpdateRequestActive(startedAt, now)) return null;
  if (!isLocalInstallUpdateCheck(event)) return startedAt;
  final error = event['error']?.toString() ?? '';
  final url = event['url']?.toString() ?? '';
  return error.isEmpty && url.isNotEmpty ? startedAt : null;
}

void applyLocalUpdateCheckEvent(Map<String, dynamic> event, {DateTime? now}) {
  localUpdateRequestStartedAt.value = localUpdateRequestAfterCheck(
      event, localUpdateRequestStartedAt.value, now ?? DateTime.now());
}

bool applyLocalUpdateEvent(Map<String, dynamic> event, {DateTime? now}) {
  final at = now ?? DateTime.now();
  final startedAt = localUpdateRequestStartedAt.value;
  localUpdateRequestStartedAt.value =
      localUpdateRequestAfterEvent(event, startedAt, at);
  return localUpdateFailureMayOpenBrowser(event,
      requestStartedAt: startedAt, now: at);
}
