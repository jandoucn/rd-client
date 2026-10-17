import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common/hbbs/hbbs.dart';
import 'package:flutter_hbb/models/ab_model.dart';
import 'package:flutter_hbb/models/peer_model.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  late Ab ab;
  late Peers model;
  late MockClient client;
  late Future<http.Response> Function(http.Request) respond;

  Future<void> online(String onlines, String offlines) async {
    await platformFFI.tryHandle({
      'name': 'callback_query_onlines',
      'onlines': onlines,
      'offlines': offlines,
    });
  }

  http.Response peersResponse(List<String> ids) => http.Response(
      jsonEncode({
        'total': ids.length,
        'data': ids.map((id) => {'id': id, 'alias': 'new-$id'}).toList()
      }),
      200);

  setUp(() {
    respond = (request) async => request.url.path.endsWith('/peers')
        ? peersResponse(['a', 'b', 'c'])
        : http.Response('[]', 200);
    client = MockClient((request) => respond(request));
    ab = Ab(AbProfile('fixture-ab', 'fixture', '', '', 0, null), true,
        apiServer: () async => 'http://fixture.invalid',
        headers: () => {},
        post: (uri, headers) => client.post(uri, headers: headers));
    ab.peers.addAll(['a', 'b', 'c'].map((id) => Peer.fromJson({'id': id})));
    model = Peers(
        name: 'ab-http-fixture',
        getInitPeers: () => ab.peers,
        loadEvent: 'fixture-ab-load');
  });
  tearDown(() {
    model.dispose();
    client.close();
  });

  for (final personal in [true, false]) {
    testWidgets('metadata reload emits no offline frame personal=$personal',
        (tester) async {
      model.dispose();
      ab = Ab(AbProfile('fixture-ab', 'fixture', '', '', 0, null), personal,
          apiServer: () async => 'http://fixture.invalid',
          headers: () => {},
          post: (uri, headers) => client.post(uri, headers: headers));
      ab.peers.addAll(['a', 'b', 'c'].map((id) => Peer.fromJson({'id': id})));
      model = Peers(name: 'ab-http-fixture', getInitPeers: () => ab.peers,
          loadEvent: 'fixture-ab-load');
      await online('a,b', 'c');
      final frames = <List<bool>>[];
      await tester.pumpWidget(MaterialApp(home: Obx(() {
        frames.add(ab.peers.map((p) => p.online).toList());
        return Text(ab.peers.map((p) => '${p.id}:${p.online}').join(','));
      })));
      final entered = Completer<void>();
      final response = Completer<http.Response>();
      respond = (request) async {
        if (request.url.path.endsWith('/peers')) {
          entered.complete();
          return response.future;
        }
        return http.Response('[]', 200);
      };
      final pull = ab.pullAb(quiet: true);
      await tester.runAsync(() => entered.future);
      await tester.pump();
      response.complete(http.Response(jsonEncode({'total': 3, 'data': [
        {'id': 'a', 'hash': 'changed-session-hash', 'hostname': 'new-host'},
        {'id': 'b'}, {'id': 'c'}
      ]}), 200));
      await tester.runAsync(() => pull);
      await platformFFI.tryHandle({'name': 'fixture-ab-load'});
      await tester.pump();
      expect(ab.initialized, isTrue);
      expect(ab.peers.first.hostname, 'new-host');
      expect(ab.peers.first.hash, 'changed-session-hash');
      expect(frames, isNotEmpty);
      for (final frame in frames) {
        expect(frame, [true, true, false]);
      }
      await online('', 'b');
      expect(ab.peers[1].online, isFalse);
    });
  }

  for (final body in ['{}', '{"total":3}', '{"total":3,"data":null}']) {
    test('invalid peer response preserves verified objects: $body', () async {
      await online('a,b', 'c');
      final previous = ab.peers.toList();
      respond = (request) async => request.url.path.endsWith('/peers')
          ? http.Response(body, 200) : http.Response('[]', 200);
      await ab.pullAb(quiet: true);
      expect(ab.initialized, isFalse);
      expect(ab.peers, orderedEquals(previous));
      expect(ab.peers.map((p) => p.online), [true, true, false]);
    });
  }

  test('later page failure never publishes a partial peer list', () async {
    await online('a,b', 'c');
    final previous = ab.peers.toList();
    var pages = 0;
    respond = (request) async {
      if (!request.url.path.endsWith('/peers')) return http.Response('[]', 200);
      pages++;
      return pages == 1
          ? http.Response('{"total":101,"data":[{"id":"a"}]}', 200)
          : http.Response('{"error":"page unavailable"}', 503);
    };
    await ab.pullAb(quiet: true);
    expect(pages, 2);
    expect(ab.initialized, isFalse);
    expect(ab.peers, orderedEquals(previous));
    expect(ab.peers.map((p) => p.online), [true, true, false]);
  });

  test('invalid pagination envelope retains all verified peer objects', () async {
    await online('a,b', 'c');
    final previous = ab.peers.toList();
    for (final body in [
      '{"data":[]}',
      '{"total":null,"data":[]}',
      '{"total":-1,"data":[]}',
      '{"total":"3","data":[]}',
      '{"total":1.5,"data":[]}',
      '{"total":3,"data":{}}',
      '{"total":1,"data":[null]}',
    ]) {
      respond = (request) async => request.url.path.endsWith('/peers')
          ? http.Response(body, 200) : http.Response('[]', 200);
      await ab.pullAb(quiet: true);
      expect(ab.initialized, isFalse, reason: body);
      expect(ab.peers, orderedEquals(previous), reason: body);
      expect(ab.peers.map((p) => p.online), [true, true, false], reason: body);
    }
  });

  test('malformed later page never commits successfully decoded first page',
      () async {
    await online('a,b', 'c');
    final previous = ab.peers.toList();
    var pages = 0;
    respond = (request) async {
      if (!request.url.path.endsWith('/peers')) return http.Response('[]', 200);
      pages++;
      return http.Response(pages == 1
          ? '{"total":101,"data":[{"id":"a","alias":"partial"}]}'
          : '{"total":101,"data":null}', 200);
    };
    await ab.pullAb(quiet: true);
    expect(pages, 2);
    expect(ab.initialized, isFalse);
    expect(ab.peers, orderedEquals(previous));
    expect(ab.peers.first.alias, isEmpty);
    expect(ab.peers.map((p) => p.online), [true, true, false]);
  });

  test('tags barrier rejects old scope without publishing pending peer rows',
      () async {
    await online('a,b', 'c');
    final previous = ab.peers.toList();
    ab.tags.value = ['original-tag'];
    ab.tagColors.value = {'original-tag': 1};
    final entered = Completer<void>();
    final response = Completer<http.Response>();
    respond = (request) async {
      if (request.url.path.endsWith('/peers')) {
        expect(request.url.queryParameters['ab'], 'fixture-ab');
        return peersResponse(['old-response-only']);
      }
      expect(request.url.path, '/api/ab/tags/fixture-ab');
      entered.complete();
      return response.future;
    };
    final pull = ab.pullAb(quiet: true);
    await entered.future;
    final pendingPeers = ab.peers.toList();
    final pendingOnline = ab.peers.map((p) => p.online).toList();
    ab.setSharedProfile(AbProfile('replacement-ab', 'fixture', '', '', 0, null));
    final replacement = Peer.fromJson({'id': 'new-scope', 'alias': 'new-owner'});
    ab.peers.value = [replacement];
    ab.tags.value = ['new-scope-tag'];
    ab.tagColors.value = {'new-scope-tag': 2};
    await online('new-scope', '');
    response.complete(http.Response('[{"name":"old-tag","color":3}]', 200));
    await pull;
    expect(ab.initialized, isFalse);
    expect(ab.profile.guid, 'replacement-ab');
    expect(ab.peers, orderedEquals([replacement]));
    expect(replacement.online, isTrue);
    expect(ab.tags, ['new-scope-tag']);
    expect(ab.tagColors, {'new-scope-tag': 2});
    expect(pendingOnline, [true, true, false]);
    expect(pendingPeers, orderedEquals(previous));
  });

  test('tags barrier commits latest online callbacks only after tags complete',
      () async {
    await online('a', 'b,c');
    final previous = ab.peers.toList();
    final entered = Completer<void>();
    final response = Completer<http.Response>();
    respond = (request) async {
      if (request.url.path.endsWith('/peers')) return peersResponse(['a', 'b', 'c']);
      entered.complete();
      return response.future;
    };
    final pull = ab.pullAb(quiet: true);
    await entered.future;
    final pendingPeers = ab.peers.toList();
    await online('b', 'a');
    response.complete(http.Response('[{"name":"updated-tag","color":4}]', 200));
    await pull;
    expect(ab.initialized, isTrue);
    expect(ab.peers.map((p) => p.online), [false, true, false]);
    expect(ab.peers.first.alias, 'new-a');
    expect(ab.tags, ['updated-tag']);
    expect(ab.tagColors, {'updated-tag': 4});
    await online('', 'b');
    expect(ab.peers[1].online, isFalse);
    expect(pendingPeers, orderedEquals(previous));
  });

  for (final failures in ['peers', 'tags', 'both']) {
    test('tags barrier HTTP failure preserves original snapshot: $failures',
        () async {
      await online('a,b', 'c');
      final previous = ab.peers.toList();
      ab.tags.value = ['original-tag'];
      ab.tagColors.value = {'original-tag': 1};
      respond = (request) async {
        final peersRequest = request.url.path.endsWith('/peers');
        if (failures == 'both' || failures == (peersRequest ? 'peers' : 'tags')) {
          return http.Response('{"error":"fixture unavailable"}', 503);
        }
        return peersRequest ? peersResponse(['a', 'b', 'c'])
            : http.Response('[{"name":"uncommitted-tag","color":5}]', 200);
      };
      await ab.pullAb(quiet: true);
      expect(ab.initialized, isFalse);
      expect(ab.peers, orderedEquals(previous));
      expect(ab.peers.map((p) => p.online), [true, true, false]);
      expect(ab.tags, ['original-tag']);
      expect(ab.tagColors, {'original-tag': 1});
    });
  }

  test('actual Ab HTTP reload retains online peers and updates metadata',
      () async {
    await online('a,b', 'c');
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isTrue);
    expect(ab.peers.map((p) => p.online), [true, true, false]);
    expect(ab.peers.first.alias, 'new-a');
  });

  test('callback during HTTP reload wins over state at request start',
      () async {
    await online('a', 'b,c');
    final entered = Completer<void>();
    final response = Completer<http.Response>();
    respond = (request) async {
      if (request.url.path.endsWith('/peers')) {
        entered.complete();
        return response.future;
      }
      return http.Response('[]', 200);
    };
    final pull = ab.pullAb(quiet: true);
    await entered.future;
    await online('b', 'a');
    response.complete(peersResponse(['a', 'b', 'c']));
    await pull;
    expect(ab.initialized, isTrue);
    expect(ab.peers.map((p) => p.online), [false, true, false]);
    await online('a', 'b');
    expect(ab.peers.map((p) => p.online), [true, false, false]);
  });

  test('failed HTTP reload preserves old objects and verified status',
      () async {
    await online('a,b', 'c');
    final previous = ab.peers.toList();
    respond = (request) async => request.url.path.endsWith('/peers')
        ? http.Response('{"error":"fixture unavailable"}', 503)
        : http.Response('[]', 200);
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isFalse);
    expect(ab.peers, orderedEquals(previous));
    expect(ab.peers.map((p) => p.online), [true, true, false]);
  });

  test('removed peer does not lend online state to new or readded IDs',
      () async {
    await online('a,b', 'c');
    respond = (request) async => request.url.path.endsWith('/peers')
        ? peersResponse(['b', 'd'])
        : http.Response('[]', 200);
    await ab.pullAb(quiet: true);
    expect(ab.peers.map((p) => p.id), ['b', 'd']);
    expect(ab.peers.map((p) => p.online), [true, false]);
    respond = (request) async => request.url.path.endsWith('/peers')
        ? peersResponse(['a', 'b', 'd'])
        : http.Response('[]', 200);
    await ab.pullAb(quiet: true);
    expect(ab.peers.map((p) => p.online), [false, true, false]);
  });

  testWidgets('actual Ab reload preserves visible unrelated peer',
      (tester) async {
    await online('a,b', 'c');
    await tester.pumpWidget(MaterialApp(
        home: Scaffold(
            body: Obx(() => Column(
                children: ab.peers
                    .map((p) => Text('${p.id}:${p.online}'))
                    .toList())))));
    expect(find.text('b:true'), findsOneWidget);
    await tester.runAsync(() => ab.pullAb(quiet: true));
    await tester.pump();
    expect(ab.initialized, isTrue);
    expect(find.text('b:true'), findsOneWidget);
    expect(find.text('c:false'), findsOneWidget);
  });

  test('another address book with the same ID does not inherit online state',
      () async {
    await online('a,b', 'c');
    final other = Ab(AbProfile('other-account-ab', 'other', '', '', 0, null),
        true,
        apiServer: () async => 'http://fixture.invalid',
        headers: () => {},
        post: (uri, headers) => client.post(uri, headers: headers));
    await other.pullAb(quiet: true);
    expect(other.initialized, isTrue);
    expect(other.peers.map((p) => p.online), [false, false, false]);
    expect(ab.peers.map((p) => p.online), [true, true, false]);
    expect(identical(other.peers.first, ab.peers.first), isFalse);
    expect(ab.peers.first.alias, isEmpty);
    expect(other.peers.first.alias, 'new-a');
  });

  test('valid empty metadata response removes old peers without caching them',
      () async {
    await online('a,b', 'c');
    respond = (request) async => request.url.path.endsWith('/peers')
        ? peersResponse([])
        : http.Response('[]', 200);
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isTrue);
    expect(ab.peers, isEmpty);
    respond = (request) async => request.url.path.endsWith('/peers')
        ? peersResponse(['a'])
        : http.Response('[]', 200);
    await ab.pullAb(quiet: true);
    expect(ab.peers.single.online, isFalse);
    await online('a', '');
    expect(ab.peers.single.online, isTrue);
    await online('', 'a');
    expect(ab.peers.single.online, isFalse);
  });

  test('same instance changed GUID cannot inherit online state through load',
      () async {
    await online('a,b', 'c');
    ab.setSharedProfile(AbProfile('replacement-ab', 'fixture', '', '', 0, null));
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isTrue);
    expect(ab.peers.map((p) => p.online), [false, false, false]);
    await platformFFI.tryHandle({'name': 'fixture-ab-load'});
    expect(model.peers.map((p) => p.online), [false, false, false]);
    await online('a', '');
    expect(ab.peers.first.online, isTrue);
  });

  test('same GUID renamed profile retains online state through load', () async {
    await online('a,b', 'c');
    ab.setSharedProfile(AbProfile('fixture-ab', 'renamed', '', '', 0, null));
    await ab.pullAb(quiet: true);
    await platformFFI.tryHandle({'name': 'fixture-ab-load'});
    expect(ab.initialized, isTrue);
    expect(model.peers.map((p) => p.online), [true, true, false]);
  });

  test('changed GUID rejects an old in-flight peer response', () async {
    await online('a,b', 'c');
    final entered = Completer<void>();
    final response = Completer<http.Response>();
    respond = (request) async {
      if (request.url.path.endsWith('/peers')) {
        expect(request.url.queryParameters['ab'], 'fixture-ab');
        entered.complete();
        return response.future;
      }
      return http.Response('[]', 200);
    };
    final pull = ab.pullAb(quiet: true);
    await entered.future;
    ab.setSharedProfile(AbProfile('replacement-ab', 'fixture', '', '', 0, null));
    response.complete(peersResponse(['old-response-only']));
    await pull;
    expect(ab.initialized, isFalse);
    expect(ab.peers.any((p) => p.id == 'old-response-only'), isFalse);
    expect(ab.peers.every((p) => !p.online), isTrue);
    await platformFFI.tryHandle({'name': 'fixture-ab-load'});
    expect(model.peers.every((p) => !p.online), isTrue);
  });
}
