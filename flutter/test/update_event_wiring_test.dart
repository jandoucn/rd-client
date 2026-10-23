import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/desktop/widgets/update_progress.dart';
import 'package:flutter_hbb/models/state_model.dart';

// These drive the handler that `checkUpdate()` actually registers, so the
// silence rule is proven at the entry point instead of only on the predicate.
void main() {
  late List<String> opened;

  setUp(() {
    opened = <String>[];
    localUpdateRequestStartedAt.value = null;
  });
  tearDown(() => localUpdateRequestStartedAt.value = null);

  Future<void> pump(Map<String, dynamic> evt) => handleSoftwareUpdateEvent(
        evt,
        openFallback: (event, {String fallbackUrl = ''}) async {
          opened.add(event['status']?.toString() ?? '');
        },
      );

  test('a remote failure never reaches the browser', () async {
    localUpdateRequestStartedAt.value = DateTime.now();
    await pump({
      'status': 'failed',
      'url': 'https://download.yan.life/a',
      'request_origin': 'command_install',
    });
    expect(opened, isEmpty);
  });

  test('a failure without an origin never reaches the browser', () async {
    localUpdateRequestStartedAt.value = DateTime.now();
    await pump({'status': 'failed', 'url': 'https://download.yan.life/a'});
    expect(opened, isEmpty);
  });

  // The desk's own update must still fall back, or a failed install leaves the
  // user with nothing to click.
  test('the desk request falls back exactly once', () async {
    localUpdateRequestStartedAt.value = DateTime.now();
    await pump({
      'status': 'failed',
      'url': 'https://download.yan.life/a',
      'request_origin': 'manual',
    });
    expect(opened, ['failed']);
    await pump({
      'status': 'failed',
      'url': 'https://download.yan.life/a',
      'request_origin': 'manual',
    });
    expect(opened, ['failed'], reason: 'the request ended at the first failure');
  });

  test('a stale desk request does not fall back', () async {
    localUpdateRequestStartedAt.value = DateTime.now()
        .subtract(localUpdateRequestTimeout + const Duration(minutes: 1));
    await pump({
      'status': 'failed',
      'url': 'https://download.yan.life/a',
      'request_origin': 'manual',
    });
    expect(opened, isEmpty);
  });

  // The status line reflects every event, indifferent to who asked for it.
  test('a remote started event still moves the status line', () async {
    await pump({'status': 'downloading', 'request_origin': 'system'});
    expect(stateGlobal.updateStatus.value, 'downloading');
    expect(opened, isEmpty);
  });
}
