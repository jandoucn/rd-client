import 'package:flutter_hbb/desktop/widgets/mac_quit_offline.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('launch asks the core to restore an explicit-quit offline state', () {
    final calls = <String>[];
    MacQuitOffline(setCommon: (k) async => calls.add(k)).onLaunch();
    expect(calls, ['mac-restore-after-quit']);
  });

  test('a failing restore does not throw into the caller', () async {
    MacQuitOffline(setCommon: (_) async => throw StateError('ipc down'))
        .onLaunch();
    await Future<void>.delayed(Duration.zero);
  });
}
