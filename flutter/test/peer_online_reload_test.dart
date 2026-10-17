import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_hbb/common/hbbs/hbbs.dart';
import 'package:flutter_hbb/models/ab_model.dart';
import 'package:flutter_hbb/models/peer_model.dart';
import 'package:flutter_hbb/models/platform_model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:http/http.dart' as http;

void main() {
  late RxList<Peer> addressBook;
  late Peers model;
  late Ab ab;
  late List<Map<String, String>> responsePeers;

  Future<void> online({String onlines = '', String offlines = ''}) async {
    expect(
        await platformFFI.tryHandle({
          'name': 'callback_query_onlines',
          'onlines': onlines,
          'offlines': offlines,
        }),
        isTrue);
  }

  setUp(() {
    responsePeers = [];
    ab = Ab(AbProfile('fixture', 'fixture', '', '', 0, null), true,
        apiServer: () async => 'http://fixture.invalid',
        headers: () => {},
        post: (uri, headers) async => http.Response(
            uri.path.endsWith('/peers')
                ? jsonEncode(
                    {'total': responsePeers.length, 'data': responsePeers})
                : '[]',
            200));
    addressBook = ab.peers;
    addressBook.addAll(<Peer>[
      Peer.fromJson({'id': 'fixture-a'}),
      Peer.fromJson({'id': 'fixture-b'}),
      Peer.fromJson({'id': 'fixture-c'}),
    ]);
    model = Peers(
      name: 'online-reload-fixture',
      getInitPeers: () => addressBook,
      loadEvent: 'fixture_load_address_book',
    );
  });

  tearDown(() => model.dispose());

  test('switching address book does not transfer same-ID online evidence',
      () async {
    await online(onlines: 'fixture-a,fixture-b');
    final previous = addressBook;
    addressBook = <Peer>[Peer.fromJson({'id': 'fixture-a'})].obs;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(model.peers.single.online, isFalse);
    expect(previous.first.online, isTrue);
  });

  test('returning to verified personal book preserves its own online evidence',
      () async {
    await online(onlines: 'fixture-a,fixture-b');
    final personal = addressBook;
    addressBook = <Peer>[Peer.fromJson({'id': 'shared-only'})].obs;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    addressBook = personal;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(model.peers.map((peer) => peer.online), [true, true, false]);
    await online(offlines: 'fixture-b');
    expect(model.peers.map((peer) => peer.online), [true, false, false]);
  });

  test('recent metadata load retains online evidence without an Ab supplier',
      () async {
    final recent = Peers(
        name: 'recent-online-fixture',
        getInitPeers: null,
        loadEvent: 'fixture_load_recent');
    addTearDown(recent.dispose);
    Future<void> load(String alias) => platformFFI.tryHandle({
          'name': 'fixture_load_recent',
          'peers': jsonEncode([
            {'id': 'fixture-a', 'alias': alias}
          ]),
        }).then((_) {});
    await load('before');
    await online(onlines: 'fixture-a');
    await load('after-connect');
    expect(recent.peers.single.alias, 'after-connect');
    expect(recent.peers.single.online, isTrue);
    await online(offlines: 'fixture-a');
    expect(recent.peers.single.online, isFalse);
  });

  test('offline callback after book switch survives reload and switching back',
      () async {
    await online(onlines: 'fixture-a,fixture-b');
    final personal = addressBook;
    final shared = <Peer>[Peer.fromJson({'id': 'fixture-a'})].obs;
    addressBook = shared;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    await online(onlines: 'fixture-a');
    expect(shared.single.online, isTrue);
    await online(offlines: 'fixture-a');
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(shared.single.online, isFalse);
    addressBook = personal;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(personal.first.online, isTrue);
    await online(offlines: 'fixture-a');
    responsePeers = [{'id': 'fixture-a'}, {'id': 'fixture-b'}];
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isTrue);
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(personal.map((peer) => peer.online), [false, true]);
    addressBook = shared;
    await platformFFI.tryHandle({'name': 'fixture_load_address_book'});
    expect(shared.single.online, isFalse);
  });

  test('single peer response preserves other verified online peers', () async {
    await online(onlines: 'fixture-a,fixture-b', offlines: 'fixture-c');
    await online(offlines: 'fixture-a');
    expect(model.peers.map((peer) => peer.online), [false, true, false]);
  });

  test('empty callback does not turn verified online peers offline', () async {
    await online(onlines: 'fixture-a,fixture-b', offlines: 'fixture-c');
    await online();
    expect(model.peers.map((peer) => peer.online), [true, true, false]);
  });

  test('address book replacement preserves verified state on load event',
      () async {
    await online(onlines: 'fixture-a,fixture-b', offlines: 'fixture-c');
    responsePeers = [
      {'id': 'fixture-b', 'alias': 'updated metadata'},
      {'id': 'fixture-a'},
      {'id': 'fixture-c'},
    ];
    await ab.pullAb(quiet: true);
    expect(ab.initialized, isTrue);
    expect(
        await platformFFI.tryHandle({
          'name': 'fixture_load_address_book',
        }),
        isTrue);
    expect(model.peers.first.alias, 'updated metadata');
    expect(model.peers.map((peer) => peer.online), [true, true, false]);
  });

  testWidgets('metadata reload keeps unrelated peer visibly online',
      (tester) async {
    await online(onlines: 'fixture-a,fixture-b', offlines: 'fixture-c');
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Obx(() => Column(
              children: addressBook
                  .map((peer) =>
                      Text('${peer.id}:${peer.online ? 'online' : 'offline'}'))
                  .toList(),
            )),
      ),
    ));
    expect(find.text('fixture-b:online'), findsOneWidget);

    responsePeers = [
      {'id': 'fixture-a', 'hash': 'fixture-new-hash'},
      {'id': 'fixture-b'},
      {'id': 'fixture-c'},
    ];
    await tester.runAsync(() => ab.pullAb(quiet: true));
    expect(ab.initialized, isTrue);
    await tester.pump();
    expect(addressBook.first.hash, 'fixture-new-hash');
    expect(find.text('fixture-b:online'), findsOneWidget);
    expect(find.text('fixture-c:offline'), findsOneWidget);
  });
}
