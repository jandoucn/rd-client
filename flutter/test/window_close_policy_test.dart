import 'package:flutter/services.dart';
import 'package:flutter_hbb/utils/platform_channel.dart';
import 'package:flutter_hbb/utils/window_close_policy.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('closing every window', () {
    test('keeps the macOS process (and its server) running', () {
      expect(quitWhenAllWindowsClosed(isMacOS: true), isFalse);
    });

    test('keeps the previous behaviour elsewhere', () {
      expect(quitWhenAllWindowsClosed(isMacOS: false), isTrue);
    });
  });

  group('dock reopen', () {
    const channel = 'org.rustdesk.rustdesk/host';
    const codec = StandardMethodCodec();

    Future<void> sendFromNative(String method) async {
      await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
              channel, codec.encodeMethodCall(MethodCall(method)), (_) {});
    }

    test('native reopen request shows the main window', () async {
      var reopened = 0;
      RdPlatformChannel.instance.setReopenMainWindowHandler(() async {
        reopened++;
      });

      await sendFromNative('reopenMainWindow');
      expect(reopened, 1);

      await sendFromNative('somethingElse');
      expect(reopened, 1);
    });
  });
}
