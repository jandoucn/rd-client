import 'package:flutter_hbb/desktop/widgets/mac_permission_guide.dart';
import 'package:flutter_test/flutter_test.dart';

class _Fixture {
  final granted = <MacPermission, bool>{
    MacPermission.screenRecording: false,
    MacPermission.accessibility: false,
  };
  final requested = <MacPermission>[];
  final opened = <MacPermission>[];
  String state;
  int restarts = 0;
  int agentRestarts = 0;
  bool allowRestart = true;
  final events = <String>[];

  _Fixture({this.state = ''});

  MacPermissionGuide guide({
    String buildSeq = '2026103101',
    List<MacPermission> required = const [
      MacPermission.screenRecording,
      MacPermission.accessibility,
    ],
  }) =>
      MacPermissionGuide(
        buildSeq: buildSeq,
        required: required,
        isGranted: (p) => granted[p] ?? false,
        request: (p) {
          events.add('request:${p.name}');
          requested.add(p);
        },
        adminSetup: (missing) async {
          await Future<void>.delayed(Duration.zero);
          events.add('setup:${macAdminSetupArg(missing)}');
        },
        openSettings: opened.add,
        readState: () => state,
        writeState: (v) => state = v,
        restart: () {
          if (!allowRestart) return false;
          restarts++;
          return true;
        },
        restartAgent: () {
          if (!allowRestart) return false;
          agentRestarts++;
          return true;
        },
      );
}

void main() {
  group('start', () {
    test('requests the first missing permission and opens its settings page', () async {
      final f = _Fixture();
      final g = f.guide();
      await g.start();

      expect(g.active, isTrue);
      expect(f.requested, [MacPermission.screenRecording]);
      expect(f.opened, [MacPermission.screenRecording]);
      expect(f.state, '2026103101:1');
      expect(f.restarts, 0);
    });

    test('skips straight to accessibility when screen recording is granted', () async {
      final f = _Fixture();
      f.granted[MacPermission.screenRecording] = true;
      await f.guide().start();

      expect(f.requested, [MacPermission.accessibility]);
      expect(f.opened, [MacPermission.accessibility]);
    });

    test('marks the build done without prompting when nothing is missing', () async {
      final f = _Fixture();
      f.granted.updateAll((_, __) => true);
      final g = f.guide();
      await g.start();

      expect(g.active, isFalse);
      expect(f.requested, isEmpty);
      expect(f.opened, isEmpty);
      expect(f.state, '2026103101:done');
      expect(f.restarts, 0);
    });

    test('does nothing for a build already completed', () async {
      final f = _Fixture(state: '2026103101:done');
      final g = f.guide();
      await g.start();

      expect(g.active, isFalse);
      expect(f.requested, isEmpty);
      expect(f.state, '2026103101:done');
    });

    test('stops auto-prompting after three launches of the same build', () async {
      final f = _Fixture(state: '2026103101:3');
      final g = f.guide();
      await g.start();

      expect(g.active, isFalse);
      expect(f.requested, isEmpty);
      expect(f.state, '2026103101:3');
    });

    test('counts launches within a build and resets the count on a new build', () async {
      final f = _Fixture(state: '2026103101:2');
      await f.guide().start();
      expect(f.state, '2026103101:3');

      final g = _Fixture(state: '2026103101:done');
      await g.guide(buildSeq: '2026103102').start();
      expect(g.state, '2026103102:1');
      expect(g.requested, [MacPermission.screenRecording]);
    });

    test('treats a malformed stored state as a fresh build', () async {
      for (final stored in [
        'garbage',
        '2026103101',
        '2026103101:x',
        ':1',
        '2026103101:-5',
      ]) {
        final f = _Fixture(state: stored);
        await f.guide().start();
        expect(f.state, '2026103101:1', reason: 'stored=$stored');
      }
    });

    test('is inert without a build number or required permissions', () async {
      final a = _Fixture();
      final ga = a.guide(buildSeq: '');
      await ga.start();
      expect(ga.active, isFalse);
      expect(a.requested, isEmpty);
      expect(a.state, '');

      final b = _Fixture();
      final gb = b.guide(required: const []);
      await gb.start();
      expect(gb.active, isFalse);
      expect(b.requested, isEmpty);
      expect(b.state, '');
    });
  });

  group('stale grant cleanup', () {
    test('first launch of a build clears stale grants before prompting',
        () async {
      final f = _Fixture();
      await f.guide().start();
      expect(f.events, ['setup:screen,accessibility', 'request:screenRecording']);
    });

    test('later launches of the same build do not ask again', () async {
      final f = _Fixture(state: '2026103101:1');
      await f.guide().start();
      expect(f.events, ['request:screenRecording']);
    });

    test('first launch with every grant still runs setup, but keeps grants',
        () async {
      final f = _Fixture();
      f.granted.updateAll((_, __) => true);
      await f.guide().start();
      expect(f.events, ['setup:'],
          reason: 'the daemon may still need installing');
      expect(f.state, '2026103101:done');
    });

    test('a completed build never asks again', () async {
      final f = _Fixture(state: '2026103101:done');
      await f.guide().start();
      expect(f.events, isEmpty);
    });
  });

  test('only the permissions that are missing are reported', () async {
    final f = _Fixture();
    f.granted[MacPermission.accessibility] = true;
    await f.guide().start();
    expect(f.events.first, 'setup:screen',
        reason: 'a working accessibility grant must not be reset');
  });

  test('admin setup argument names each missing permission once', () {
    expect(macAdminSetupArg(const []), '');
    expect(
        macAdminSetupArg(const [
          MacPermission.accessibility,
          MacPermission.screenRecording,
          MacPermission.accessibility,
        ]),
        'accessibility,screen');
  });

  group('later launch of a prompted build', () {
    test('restarts only the server agent once grants were made off-guide', () async {
      final f = _Fixture(state: '2026103101:1');
      f.granted.updateAll((_, __) => true);
      final g = f.guide();
      await g.start();

      expect(g.active, isFalse);
      expect(f.agentRestarts, 1);
      expect(f.restarts, 0);
      expect(f.state, '2026103101:done');
    });

    test('defers the agent restart while a session is active', () async {
      final f = _Fixture(state: '2026103101:2')..allowRestart = false;
      f.granted.updateAll((_, __) => true);
      final g = f.guide();
      await g.start();

      expect(g.active, isTrue);
      expect(f.agentRestarts, 0);
      expect(f.state, '2026103101:2',
          reason: 'quitting now must leave the restart pending for next launch');
      g.tick();
      expect(f.agentRestarts, 0);

      f.allowRestart = true;
      expect(g.tick(), isFalse);
      expect(g.active, isFalse);
      expect(f.agentRestarts, 1);
      expect(f.restarts, 0);
      expect(f.state, '2026103101:done');
    });

    test('a fresh build that is already granted does not touch the agent', () async {
      final f = _Fixture(state: '2026103100:done');
      f.granted.updateAll((_, __) => true);
      await f.guide().start();

      expect(f.agentRestarts, 0);
      expect(f.state, '2026103101:done');
    });
  });

  group('tick', () {
    test('waits while the current permission is still off', () async {
      final f = _Fixture();
      final g = f.guide();
      await g.start();
      expect(g.tick(), isFalse);
      expect(g.tick(), isFalse);

      expect(g.active, isTrue);
      expect(f.requested, [MacPermission.screenRecording]);
      expect(f.restarts, 0);
    });

    test('moves to the next permission once the current one is granted', () async {
      final f = _Fixture();
      final g = f.guide();
      await g.start();
      f.granted[MacPermission.screenRecording] = true;
      expect(g.tick(), isTrue);

      expect(f.requested,
          [MacPermission.screenRecording, MacPermission.accessibility]);
      expect(f.opened,
          [MacPermission.screenRecording, MacPermission.accessibility]);
      expect(f.restarts, 0);
    });

    test('restarts exactly once after every required permission is granted', () async {
      final f = _Fixture();
      final g = f.guide();
      await g.start();
      f.granted.updateAll((_, __) => true);
      expect(g.tick(), isTrue);
      expect(g.tick(), isFalse);

      expect(g.active, isFalse);
      expect(f.restarts, 1);
      expect(f.state, '2026103101:done');
    });

    test('defers the restart while a session is active', () async {
      final f = _Fixture()..allowRestart = false;
      final g = f.guide();
      await g.start();
      f.granted.updateAll((_, __) => true);
      g.tick();
      g.tick();

      expect(g.active, isTrue);
      expect(f.restarts, 0);
      expect(f.state, '2026103101:1');

      f.allowRestart = true;
      g.tick();
      expect(g.active, isFalse);
      expect(f.restarts, 1);
      expect(f.state, '2026103101:done');
    });

    test('a later tick after completion does not prompt again', () async {
      final f = _Fixture();
      final g = f.guide();
      await g.start();
      f.granted.updateAll((_, __) => true);
      g.tick();
      f.granted[MacPermission.accessibility] = false;
      g.tick();

      expect(f.requested, [MacPermission.screenRecording]);
      expect(f.restarts, 1);
    });

    test('tick before start is a no-op', () async {
      final f = _Fixture();
      f.guide().tick();
      expect(f.requested, isEmpty);
      expect(f.restarts, 0);
    });
  });

  test('guides accessibility before screen recording', () async {
    expect(kMacGuidePermissionOrder,
        [MacPermission.accessibility, MacPermission.screenRecording]);
  });

  group('settings urls', () {
    test('point at the exact privacy panes', () async {
      expect(macPermissionSettingsUrl(MacPermission.screenRecording),
          'x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture');
      expect(macPermissionSettingsUrl(MacPermission.accessibility),
          'x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility');
    });
  });

  group('relaunch', () {
    test('derives the bundle from the main executable', () async {
      expect(
          macBundlePathFromExecutable(
              '/Applications/RustDesk Yan.app/Contents/MacOS/RustDesk Yan'),
          '/Applications/RustDesk Yan.app');
      expect(macBundlePathFromExecutable('/usr/local/bin/rustdesk'), isNull);
      expect(
          macBundlePathFromExecutable('/Applications/X.app/Contents/Other/X'),
          isNull);
    });

    test('waits for this process, restarts the server agent, then reopens', () async {
      final args = macRelaunchArgs(
        bundlePath: '/Applications/RustDesk Yan.app',
        pid: 4321,
        uid: 501,
        agentLabel: 'com.carriez.RustDesk Yan_server',
      );

      expect(args.first, '-c');
      final script = args[1];
      expect(args.sublist(2), [
        'rustdesk-relaunch',
        '4321',
        '501',
        'com.carriez.RustDesk Yan_server',
        '/Applications/RustDesk Yan.app',
      ]);
      final wait = script.indexOf('kill -0 "\$1"');
      final giveUp = script.indexOf('kill -0 "\$1" 2>/dev/null && exit 1');
      final kick = script.indexOf('launchctl kickstart -k "gui/\$2/\$3"');
      expect(script, contains('-lt 300'));
      expect(giveUp, greaterThan(wait));
      expect(kick, greaterThan(giveUp));
      final open = script.indexOf('/usr/bin/open "\$4"');
      expect(wait, greaterThanOrEqualTo(0));
      expect(kick, greaterThan(wait));
      expect(open, greaterThan(kick));
      expect(script, isNot(contains('RustDesk Yan')),
          reason: 'values must be passed as arguments, not interpolated');
    });

    test('refuses unsafe inputs', () async {
      expect(
          () => macRelaunchArgs(
              bundlePath: 'relative/X.app', pid: 1, uid: 501, agentLabel: 'l'),
          throwsArgumentError);
      expect(
          () => macRelaunchArgs(
              bundlePath: '/Applications/X.app', pid: 0, uid: 501, agentLabel: 'l'),
          throwsArgumentError);
      expect(
          () => macRelaunchArgs(
              bundlePath: '/Applications/X.app', pid: 1, uid: -1, agentLabel: 'l'),
          throwsArgumentError);
      expect(
          () => macRelaunchArgs(
              bundlePath: '/Applications/X.app', pid: 1, uid: 501, agentLabel: ''),
          throwsArgumentError);
    });
  });
}
