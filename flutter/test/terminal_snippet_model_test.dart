import 'dart:async';

import 'package:flutter_hbb/models/model.dart';
import 'package:flutter_hbb/models/terminal_model.dart';
import 'package:flutter_hbb/models/terminal_snippet_model.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

class _Call {
  _Call(this.url, this.headers);
  final Uri url;
  final Map<String, String> headers;
}

class _FakeFFI implements FFI {
  @override
  String id = 'test-peer';

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test('snippets reach a real TerminalModel through its input channel',
      () async {
    final model = TerminalModel(_FakeFFI());
    addTearDown(model.dispose);
    Future<bool> send(String command, bool execute) => sendTerminalSnippet(
        command: command,
        execute: execute,
        paste: model.pasteText,
        sendKey: model.sendVirtualKey);

    expect(await send(' \n', true), isFalse);
    expect(model.debugBufferedInputCount, 0);
    expect(await send('dir\nver', false), isTrue);
    expect(model.debugBufferedInputCount, 1);
    expect(await send('dir\nver\n', true), isTrue);
    expect(model.debugBufferedInputCount, 3);
  });

  group('terminalSnippetFillText', () {
    test('folds every newline form into one space and trims', () {
      expect(terminalSnippetFillText('Get-Service\r\nrustdesk\n\nx\ry'),
          'Get-Service rustdesk x y');
      expect(terminalSnippetFillText('  a   b  '), 'a b');
      expect(terminalSnippetFillText('a\tb'), 'a\tb');
    });

    test('blank commands produce nothing', () {
      expect(terminalSnippetFillText(''), '');
      expect(terminalSnippetFillText(' \r\n \n'), '');
    });
  });

  group('terminalSnippetRunBody', () {
    test('keeps inner lines and strips only trailing newlines', () {
      expect(terminalSnippetRunBody('a\nb\r\n\n'), 'a\nb');
      expect(terminalSnippetRunBody('dir'), 'dir');
    });

    test('blank commands produce nothing', () {
      expect(terminalSnippetRunBody(''), isNull);
      expect(terminalSnippetRunBody('  \n\r\n'), isNull);
    });
  });

  group('sendTerminalSnippet', () {
    late List<String> events;
    Future<void> paste(String text) async => events.add('paste:$text');
    Future<void> key(String text) async => events.add('key:$text');

    setUp(() => events = []);

    test('fill pastes folded text without Enter', () async {
      final sent = await sendTerminalSnippet(
          command: 'echo 1\necho 2\n',
          execute: false,
          paste: paste,
          sendKey: key);
      expect(sent, isTrue);
      expect(events, ['paste:echo 1 echo 2']);
    });

    test('run pastes the body and then sends exactly one Enter', () async {
      final sent = await sendTerminalSnippet(
          command: 'echo 1\necho 2\r\n\n',
          execute: true,
          paste: paste,
          sendKey: key);
      expect(sent, isTrue);
      expect(events, ['paste:echo 1\necho 2', 'key:\r']);
    });

    test('blank commands send nothing in either mode', () async {
      for (final execute in [false, true]) {
        final sent = await sendTerminalSnippet(
            command: ' \n ', execute: execute, paste: paste, sendKey: key);
        expect(sent, isFalse);
      }
      expect(events, isEmpty);
    });
  });

  group('TerminalSnippetRepository', () {
    late String token;
    late String api;
    late List<_Call> calls;
    late FutureOr<http.Response> Function() respond;

    TerminalSnippetRepository repo() => TerminalSnippetRepository(
          apiServer: () async => api,
          accessToken: () => token,
          httpGet: (url, headers) async {
            calls.add(_Call(url, headers));
            return respond();
          },
        );

    const okBody =
        '{"data":[{"id":7,"name":"重启","command":"Restart-Service rustdesk","created_at":1,"updated_at":2},'
        '{"id":"bad"},{"id":8,"name":"多行","command":"a\\nb","created_at":3,"updated_at":4}],'
        '"limit":200,"server_time":5}';

    http.Response ok() => http.Response(okBody, 200,
        headers: {'content-type': 'application/json; charset=utf-8'});

    setUp(() {
      token = 'tok-a';
      api = 'https://rdapi.example/';
      calls = [];
      respond = ok;
    });

    test('requests the client endpoint with the bearer token', () async {
      final result = await repo().load();
      expect(result.status, TerminalSnippetLoadStatus.ok);
      expect(calls, hasLength(1));
      expect(calls.single.url.toString(),
          'https://rdapi.example/api/terminal-snippets');
      expect(calls.single.headers['Authorization'], 'Bearer tok-a');
      expect(result.snippets.map((s) => s.id), [7, 8]);
      expect(result.snippets.first.name, '重启');
      expect(result.snippets.last.command, 'a\nb');
      expect(result.limit, 200);
    });

    test('keeps the server order instead of re-sorting by time', () async {
      // 网页拖拽排序后的顺序由服务端返回顺序决定，客户端不能再按时间或 id 重排。
      respond = () => http.Response(
          '{"data":['
          '{"id":3,"name":"丙","command":"c","created_at":30,"updated_at":30},'
          '{"id":1,"name":"甲","command":"a","created_at":10,"updated_at":10},'
          '{"id":2,"name":"乙","command":"b","created_at":20,"updated_at":20}'
          '],"limit":200,"server_time":40}',
          200,
          headers: {'content-type': 'application/json; charset=utf-8'});
      final result = await repo().load();
      expect(result.snippets.map((s) => s.name), ['丙', '甲', '乙']);
    });

    test('without a token it reports not logged in and never calls the API',
        () async {
      token = '';
      final result = await repo().load();
      expect(result.status, TerminalSnippetLoadStatus.notLoggedIn);
      expect(result.snippets, isEmpty);
      expect(calls, isEmpty);
    });

    test('without an API server it fails without calling the API', () async {
      api = '  ';
      final result = await repo().load();
      expect(result.status, TerminalSnippetLoadStatus.failed);
      expect(calls, isEmpty);
    });

    test('maps status codes to load states', () async {
      final cases = {
        401: TerminalSnippetLoadStatus.unauthorized,
        403: TerminalSnippetLoadStatus.forbidden,
        404: TerminalSnippetLoadStatus.unsupported,
        500: TerminalSnippetLoadStatus.failed,
      };
      for (final entry in cases.entries) {
        respond = () => http.Response('{"error":"x"}', entry.key);
        final result = await repo().load();
        expect(result.status, entry.value, reason: '${entry.key}');
        expect(result.snippets, isEmpty);
      }
    });

    test('a malformed body is a failure', () async {
      for (final body in ['not json', '{"data":{}}', '[]']) {
        respond = () => http.Response(body, 200);
        final result = await repo().load();
        expect(result.status, TerminalSnippetLoadStatus.failed, reason: body);
      }
    });

    test('caches the last list per access token', () async {
      final repository = repo();
      expect(repository.cached, isNull);
      await repository.load();
      expect(repository.cached!.map((s) => s.id), [7, 8]);
      token = 'tok-b';
      expect(repository.cached, isNull);
    });

    test('network failures keep the cached list as an offline fallback',
        () async {
      final repository = repo();
      await repository.load();
      respond = () => throw Exception('offline');
      final failed = await repository.load();
      expect(failed.status, TerminalSnippetLoadStatus.failed);
      expect(failed.snippets.map((s) => s.id), [7, 8]);
      respond = () => http.Response('', 502);
      expect((await repository.load()).snippets.map((s) => s.id), [7, 8]);
    });

    test('401, 403 and 404 drop the cached list', () async {
      for (final status in [401, 403, 404]) {
        final repository = repo();
        respond = ok;
        await repository.load();
        expect(repository.cached, isNotNull);
        respond = () => http.Response('{}', status);
        await repository.load();
        expect(repository.cached, isNull, reason: '$status');
      }
    });

    test('a cache from another API server is never reused', () async {
      final repository = repo();
      await repository.load();
      expect(repository.cached, isNotNull);
      api = 'https://other.example';
      respond = () => throw Exception('offline');
      final result = await repository.load();
      expect(result.status, TerminalSnippetLoadStatus.failed);
      expect(result.snippets, isEmpty);
      expect(repository.cached, isNull);
    });

    test('a cache from another token is never returned on failure', () async {
      final repository = repo();
      await repository.load();
      token = 'tok-b';
      respond = () => throw Exception('offline');
      final result = await repository.load();
      expect(result.status, TerminalSnippetLoadStatus.failed);
      expect(result.snippets, isEmpty);
    });
  });
}
