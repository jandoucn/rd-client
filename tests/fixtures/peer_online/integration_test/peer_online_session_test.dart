import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter_hbb/common.dart';
import 'package:flutter_hbb/common/widgets/peer_card.dart';
import 'package:flutter_hbb/main.dart' as app;
import 'package:flutter_hbb/models/ab_model.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as image;
import 'package:integration_test/integration_test.dart';

// Child engines must retain the production entrypoint and channel ownership.
Future<void> main(List<String> args) async {
  if (args.isNotEmpty && args.first == 'multi_window') {
    await app.main(args);
    return;
  }
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('VM personal Ab preserves online peers across a real session',
      (tester) async {
    final fixture = await _preflight();
    final root = Directory(fixture['evidenceDirectory'] as String);
    expect(await root.exists(), isTrue, reason: 'Missing protected evidence directory');
    final run = List.generate(16, (_) => Random.secure().nextInt(256)
        .toRadixString(16).padLeft(2, '0')).join();
    final exchange = Directory('${root.path}/$run');
    await exchange.create();
    final ids = (fixture['peerIds'] as List).cast<String>();
    expect(ids.length, greaterThanOrEqualTo(3));
    expect(ids.toSet().length, ids.length);
    final expected = Map<String, dynamic>.from(fixture['online'] as Map);
    expect(expected.keys.toSet(), ids.toSet());
    expect(expected.values.every((v) => v is bool), isTrue);
    expect(expected[ids.first], isTrue);
    expect(expected.values.where((v) => v == true).length, greaterThanOrEqualTo(2));
    await _request(exchange, 'prepare', {'run': run, 'pid': pid});
    final ready = await _reply(tester, exchange, 'prepare');
    expect(ready['run'], run);
    expect(ready['capabilities'], containsAll([
      'native-window-auth', 'native-window-close', 'remote-pixel-challenge',
      'native-client-capture', 'server-audit', 'isolated-egress'
    ]));
    // The VM firewall must already deny production endpoints before app.main.
    expect(ready['isolationPolicyId'], fixture['isolationPolicyId']);
    await app.main(args);
    await _until(tester, () => find.byType(app.App).evaluate().isNotEmpty,
        'Application did not mount');
    expect(await bind.mainGetApiServer(), fixture['api']);
    await _until(tester, () => gFFI.userModel.isLogin, 'Missing test authentication');
    expect(gFFI.userModel.userName.value, fixture['account']);
    final tab = find.byTooltip(gFFI.peerTabModel.tabTooltip(3));
    expect(tab, findsOneWidget);
    await tester.tap(tab);
    await _until(tester, () => gFFI.abModel.current is Ab &&
        gFFI.abModel.current.peers.length >= 3, 'Missing real non-legacy Ab/third peer');
    final ab = gFFI.abModel.current as Ab;
    expect(ab.personal, isTrue);
    expect(ab.profile.guid, fixture['abGuid']);
    await _until(tester, () => ids.every((id) => ab.peers.any(
        (p) => p.id == id && p.online == expected[id])), 'Initial online states missing');
    void assertPeers() {
      expect(identical(gFFI.abModel.current, ab), isTrue);
      for (final id in ids) {
        final peers = ab.peers.where((p) => p.id == id).toList();
        expect(peers, hasLength(1), reason: 'Missing/duplicate test peer');
        expect(peers.single.online, expected[id], reason: 'Online state changed');
        final cards = tester.widgetList<AddressBookPeerCard>(
            find.byType(AddressBookPeerCard)).where((w) => w.peer.id == id);
        expect(cards, hasLength(1), reason: 'Test peers must all be visible');
        expect(cards.single.peer.online, expected[id]);
      }
    }
    assertPeers();
    final oldPeer = ab.peers.singleWhere((p) => p.id == ids.first);
    final oldHash = oldPeer.hash;
    final beforeWindows = await DesktopMultiWindow.getAllSubWindowIds();
    expect(beforeWindows, isEmpty, reason: 'Use a fresh isolated client');
    final card = find.byWidgetPredicate((w) =>
        w is AddressBookPeerCard && w.peer.id == ids.first);
    final started = DateTime.now().toUtc();
    await _request(exchange, 'connect', {
      'run': run, 'peerId': ids.first, 'startedUtc': started.toIso8601String(),
    });
    await tester.tap(card);
    await tester.pump(const Duration(milliseconds: 80));
    await tester.tap(card);
    await tester.pump();
    final connected = await _reply(tester, exchange, 'connect', check: assertPeers);
    expect(connected['run'], run);
    final windowId = connected['windowId'] as int;
    expect(await DesktopMultiWindow.getAllSubWindowIds(), contains(windowId));
    for (var round = 0; round < 2; round++) {
      final bits = List.generate(64, (_) => Random.secure().nextBool() ? 1 : 0);
      await _request(exchange, 'frame-$round', {
        'run': run, 'peerId': ids.first, 'windowId': windowId, 'bits': bits,
      });
      final frame = await _reply(tester, exchange, 'frame-$round', check: assertPeers);
      expect(frame['run'], run);
      expect(frame['windowId'], windowId);
      final png = image.decodePng(await File('${exchange.path}/frame-$round.png').readAsBytes());
      expect(png, isNotNull, reason: 'Missing native remote-window PNG');
      final x = frame['x'] as int;
      final y = frame['y'] as int;
      final cell = frame['cell'] as int;
      expect(cell, greaterThanOrEqualTo(4));
      expect(x >= 0 && y >= 0 && x + 8 * cell <= png!.width &&
          y + 8 * cell <= png.height, isTrue);
      for (var i = 0; i < bits.length; i++) {
        final pixel = png!.getPixel(x + (i % 8) * cell + cell ~/ 2,
            y + (i ~/ 8) * cell + cell ~/ 2);
        final channels = [pixel.r, pixel.g, pixel.b];
        expect(channels.every((c) => bits[i] == 1 ? c > 220 : c < 35), isTrue,
            reason: 'Remote pixels do not match fresh challenge');
      }
    }
    await _until(tester, () {
      assertPeers();
      final peer = ab.peers.singleWhere((p) => p.id == ids.first);
      return !identical(peer, oldPeer) && peer.hash.isNotEmpty && peer.hash != oldHash;
    }, 'No real hash-sync reload observed');
    await _request(exchange, 'close', {'run': run, 'windowId': windowId});
    final closed = await _reply(tester, exchange, 'close', check: assertPeers);
    expect(closed['run'], run);
    expect(await DesktopMultiWindow.getAllSubWindowIds(), isNot(contains(windowId)),
        reason: 'Driver close acknowledgement is not proof of window closure');
    // The protected server exports request metadata, never credentials/hash values.
    final audit = await _reply(tester, exchange, 'audit', check: assertPeers);
    expect(audit['run'], run);
    expect(audit['api'], fixture['api']);
    final records = (audit['records'] as List).cast<Map<String, dynamic>>();
    final updates = records.where((r) => r['method'] == 'POST' &&
        r['path'] == '/api/ab/peer/update/${ab.profile.guid}' &&
        r['peerId'] == ids.first && r['changedFields'] is List &&
        (r['changedFields'] as List).contains('hash') && r['status'] == 200 &&
        DateTime.parse(r['utc'] as String).isAfter(started)).toList();
    expect(updates, isNotEmpty, reason: 'Missing server hash-sync audit');
    expect(records.any((r) => r['method'] == 'POST' && r['path'] == '/api/ab/peers' &&
        r['ab'] == ab.profile.guid && r['status'] == 200 &&
        DateTime.parse(r['utc'] as String).isAfter(
            DateTime.parse(updates.first['utc'] as String))), isTrue,
        reason: 'Missing subsequent real Ab reload audit');
    for (var i = 0; i < 50; i++) {
      await tester.pump(const Duration(milliseconds: 100));
      assertPeers();
    }
    binding.reportData = {'run': run, 'evidenceDirectory': exchange.path,
      'peerCount': ids.length, 'remotePixelChallenges': 2};
  }, timeout: const Timeout(Duration(minutes: 9)));
}

Future<Map<String, dynamic>> _preflight() async {
  if (!Platform.isWindows) throw StateError('Windows VM required');
  final path = Platform.environment['RUSTDESK_VM_E2E_MANIFEST'];
  if (path == null || !RegExp(r'^[A-Za-z]:\\').hasMatch(path)) {
    throw StateError('Protected VM manifest absolute path required');
  }
  // Reject inherited/shared fixture ACLs; credentials stay in the VM profile.
  final acl = await Process.run('powershell.exe', ['-NoProfile', '-NonInteractive',
    '-Command', r'$ErrorActionPreference="Stop"; '
        r'$a=Get-Acl -LiteralPath $env:RUSTDESK_VM_E2E_MANIFEST; '
        r'$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; '
        r'if(!$a.AreAccessRulesProtected){exit 2}; '
        r'foreach($r in $a.Access){if($r.AccessControlType -eq "Allow") {'
        r'$s=$r.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value; '
        r'if($s -notin @($sid,"S-1-5-18","S-1-5-32-544")){exit 3}}}']);
  if (acl.exitCode != 0) throw StateError('Manifest ACL is not private/protected');
  final probe = await Process.run('powershell.exe', ['-NoProfile', '-NonInteractive',
    '-Command', r'$c=Get-CimInstance Win32_ComputerSystem; '
        r'$p=Get-CimInstance Win32_ComputerSystemProduct; '
        r'@{name=$env:COMPUTERNAME;manufacturer=$c.Manufacturer;model=$c.Model;uuid=$p.UUID}|ConvertTo-Json -Compress']);
  if (probe.exitCode != 0) throw StateError('VM identity probe failed');
  final vm = jsonDecode(probe.stdout as String) as Map<String, dynamic>;
  if (!RegExp(r'VirtualBox|VMware|Virtual Machine|KVM|QEMU', caseSensitive: false)
      .hasMatch('${vm['manufacturer']} ${vm['model']}')) {
    throw StateError('Physical host rejected');
  }
  final fixture = jsonDecode(await File(path).readAsString()) as Map<String, dynamic>;
  if (fixture['schema'] != 1 ||
      !DateTime.parse(fixture['expiresUtc'] as String).isAfter(DateTime.now().toUtc())) {
    throw StateError('Expired/unsupported fixture manifest');
  }
  for (final key in ['account', 'abGuid', 'evidenceDirectory']) {
    if (fixture[key] is! String || (fixture[key] as String).isEmpty) {
      throw StateError('Missing fixture field: $key');
    }
  }
  for (final key in ['name', 'manufacturer', 'model', 'uuid']) {
    if (vm[key] != (fixture['vm'] as Map)[key]) throw StateError('VM identity mismatch: $key');
  }
  final api = Uri.parse(fixture['api'] as String);
  if (!api.hasAuthority || api.host == 'rdapi.yan.life' ||
      !(api.host == '127.0.0.1' || api.host.endsWith('.test'))) {
    throw StateError('Only isolated loopback/.test API allowed');
  }
  if (fixture['isolationPolicyId'] is! String ||
      (fixture['isolationPolicyId'] as String).isEmpty) throw StateError('Missing egress policy');
  return fixture;
}

Future<void> _request(Directory dir, String phase, Map<String, dynamic> data) async {
  final pending = File('${dir.path}/$phase.request.tmp');
  await pending.writeAsString(jsonEncode(data), flush: true);
  await pending.rename('${dir.path}/$phase.request.json');
}

Future<Map<String, dynamic>> _reply(WidgetTester tester, Directory dir,
    String phase, {void Function()? check}) async {
  final file = File('${dir.path}/$phase.response.json');
  await _until(tester, () {
    check?.call();
    return file.existsSync();
  }, 'VM driver missing evidence: $phase');
  final response = jsonDecode(await file.readAsString()) as Map<String, dynamic>;
  if (response.containsKey('error')) throw StateError('VM driver failed: $phase');
  return response;
}

Future<void> _until(WidgetTester tester, bool Function() ready, String failure) async {
  final deadline = DateTime.now().add(const Duration(seconds: 45));
  while (!ready()) {
    if (DateTime.now().isAfter(deadline)) throw TestFailure(failure);
    await tester.pump(const Duration(milliseconds: 100));
  }
}
