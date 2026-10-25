import 'package:flutter/material.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/desktop/widgets/desktop_update_card.dart';
import 'package:flutter_hbb/models/state_model.dart';
import 'package:flutter_test/flutter_test.dart';

const _url = 'https://download.example/update.exe';

const _promptOrigins = ['command', 'startup', 'system'];

void main() {
  setUp(() => stateGlobal.updateUrl.value = '');
  tearDown(() => stateGlobal.updateUrl.value = '');

  group('disabled mode', () {
    test('hides both desktop prompts for every automated origin', () {
      for (final origin in _promptOrigins) {
        expect(
          shouldShowDesktopUpdatePrompt(
            windowVisible: true,
            autoUpdate: false,
            requestOrigin: origin,
            updateUrl: _url,
            mode: 'disabled',
          ),
          isFalse,
          reason: 'origin=$origin must not prompt when the policy is disabled',
        );
        expect(
          shouldShowStartupUpdatePrompt(
            isDesktopMainWindow: true,
            autoUpdate: false,
            requestOrigin: origin,
            updateUrl: _url,
            mode: 'disabled',
          ),
          isFalse,
          reason: 'origin=$origin must not prompt when the policy is disabled',
        );
      }
    });

    test('has no prompt action and no usable url', () {
      expect(
        updatePromptAction(mode: 'disabled', allowAutoUpdate: true),
        UpdatePromptAction.none,
      );
      expect(
        updatePromptAction(mode: 'disabled', allowAutoUpdate: false),
        UpdatePromptAction.none,
      );
      expect(effectiveUpdateUrl(mode: 'disabled', url: _url), isEmpty);
      expect(
        updatePromptAllowsInstall(
          updatePromptAction(mode: 'disabled', allowAutoUpdate: true),
        ),
        isFalse,
      );
    });

    test('a disabled check result leaves no install url behind', () {
      handleCheckSoftwareUpdateFinish({
        'target_version': '1.5.0',
        'target_build_seq': 150,
        'current_version': '1.4.9',
        'channel': 'stable',
        'mode': 'disabled',
        'url': _url,
        'request_origin': 'system',
      });
      expect(stateGlobal.updateUrl.value, isEmpty);
      expect(stateGlobal.updateStatus.value, 'Up to date');
    });
  });

  group('notify mode', () {
    test('still prompts but never offers an install action', () {
      for (final origin in _promptOrigins) {
        expect(
          shouldShowDesktopUpdatePrompt(
            windowVisible: true,
            autoUpdate: false,
            requestOrigin: origin,
            updateUrl: _url,
            mode: 'notify',
          ),
          isTrue,
          reason: 'origin=$origin must still be announced to the user',
        );
      }
      for (final allowAutoUpdate in [true, false]) {
        final action = updatePromptAction(
          mode: 'notify',
          allowAutoUpdate: allowAutoUpdate,
        );
        expect(action, UpdatePromptAction.notify);
        expect(updatePromptAllowsInstall(action), isFalse);
        expect(
          updatePromptAutoInstalls(action),
          isFalse,
          reason: 'a notify policy must never install on its own',
        );
      }
    });

    test('a notify check result keeps the notice reachable', () {
      handleCheckSoftwareUpdateFinish({
        'target_version': '1.5.0',
        'mode': 'notify',
        'url': _url,
        'request_origin': 'system',
      });
      expect(stateGlobal.updateUrl.value, _url);
      expect(stateGlobal.updateStatus.value, 'Update available');
    });
  });

  group('download mode', () {
    test('allows the download and leaves the install to a click', () {
      final action =
          updatePromptAction(mode: 'download', allowAutoUpdate: true);
      expect(action, UpdatePromptAction.download);
      expect(updatePromptAllowsInstall(action), isTrue);
      expect(updatePromptAutoInstalls(action), isFalse);
    });
  });

  group('auto_install mode', () {
    test('installs automatically only when auto update is allowed locally', () {
      final allowed =
          updatePromptAction(mode: 'auto_install', allowAutoUpdate: true);
      expect(allowed, UpdatePromptAction.autoInstall);
      expect(updatePromptAutoInstalls(allowed), isTrue);
    });

    test('does not install automatically when auto update is disallowed', () {
      final blocked =
          updatePromptAction(mode: 'auto_install', allowAutoUpdate: false);
      expect(blocked, isNot(UpdatePromptAction.autoInstall));
      expect(updatePromptAutoInstalls(blocked), isFalse);
      expect(updatePromptAllowsInstall(blocked), isTrue);
    });
  });

  group('prompt regressions', () {
    test('an unrecognised origin still never prompts', () {
      for (final origin in ['manual', 'command_install', '', 'unknown']) {
        expect(
          shouldShowDesktopUpdatePrompt(
            windowVisible: true,
            autoUpdate: false,
            requestOrigin: origin,
            updateUrl: _url,
            mode: 'notify',
          ),
          isFalse,
        );
      }
      expect(
        shouldShowStartupUpdatePrompt(
          isDesktopMainWindow: true,
          autoUpdate: false,
          requestOrigin: 'system',
          updateUrl: _url,
          mode: 'notify',
        ),
        isFalse,
      );
    });

    test('an empty url still never prompts', () {
      for (final mode in ['notify', 'download', 'auto_install']) {
        expect(
          shouldShowDesktopUpdatePrompt(
            windowVisible: true,
            autoUpdate: false,
            requestOrigin: 'command',
            updateUrl: '',
            mode: mode,
          ),
          isFalse,
        );
        expect(
          shouldShowStartupUpdatePrompt(
            isDesktopMainWindow: true,
            autoUpdate: false,
            requestOrigin: 'startup',
            updateUrl: '',
            mode: mode,
          ),
          isFalse,
        );
      }
    });

    test('an invisible window and auto update still suppress the prompt', () {
      expect(
        shouldShowDesktopUpdatePrompt(
          windowVisible: false,
          autoUpdate: false,
          requestOrigin: 'command',
          updateUrl: _url,
          mode: 'notify',
        ),
        isFalse,
      );
      expect(
        shouldShowDesktopUpdatePrompt(
          windowVisible: true,
          autoUpdate: true,
          requestOrigin: 'command',
          updateUrl: _url,
          mode: 'notify',
        ),
        isFalse,
      );
    });
  });

  group('desktop update card', () {
    testWidgets('notify renders no install action', (tester) async {
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 280,
            child: DesktopUpdateCard(
              version: '1.5.2',
              installLabel: '安装',
              onClose: () {},
            ),
          ),
        ),
      ));

      expect(find.text('1.5.2'), findsOneWidget);
      expect(find.text('安装'), findsNothing);
      expect(find.byType(OutlinedButton), findsNothing);
      expect(find.byIcon(Icons.close), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('a confirmed install runs exactly once', (tester) async {
      var installs = 0;
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: SizedBox(
            width: 280,
            child: DesktopUpdateCard(
              version: '1.5.2',
              installLabel: '安装',
              onInstall: () => installs++,
              onClose: () {},
            ),
          ),
        ),
      ));

      expect(installs, 0, reason: 'rendering a card must not start an install');
      await tester.tap(find.text('安装'));
      expect(installs, 1);
    });
  });
}
