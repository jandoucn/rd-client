import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hbb/common/widgets/terminal_snippet_panel.dart';
import 'package:flutter_hbb/models/terminal_snippet_model.dart';
import 'package:flutter_test/flutter_test.dart';

class _FakeRepository implements TerminalSnippetSource {
  _FakeRepository(this.results, {this.cached});

  final List<FutureOr<TerminalSnippetLoadResult>> results;
  @override
  List<TerminalSnippet>? cached;
  int loads = 0;

  @override
  Future<TerminalSnippetLoadResult> load() async {
    final result = await results[loads.clamp(0, results.length - 1)];
    loads++;
    return result;
  }
}

TerminalSnippet _snippet(int id, String name, String command) =>
    TerminalSnippet(
        id: id, name: name, command: command, createdAt: 1, updatedAt: 1);

final _list = [
  _snippet(1, '重启服务', 'Restart-Service rustdesk'),
  _snippet(2, '查看日志', 'Get-Content C:\\log.txt\nGet-Date'),
];

TerminalSnippetLoadResult _ok([List<TerminalSnippet>? list]) =>
    TerminalSnippetLoadResult(TerminalSnippetLoadStatus.ok, list ?? _list);

TerminalSnippetLoadResult _status(TerminalSnippetLoadStatus status,
        [List<TerminalSnippet> list = const []]) =>
    TerminalSnippetLoadResult(status, list);

void main() {
  late List<(String, bool)> selected;

  Future<void> pump(WidgetTester tester, TerminalSnippetSource source) async {
    selected = [];
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: TerminalSnippetPanel(
          key: UniqueKey(),
          source: source,
          tr: (s) => s,
          onSelected: (command, execute) => selected.add((command, execute)),
        ),
      ),
    ));
  }

  testWidgets('lists snippets after loading', (tester) async {
    final source = _FakeRepository([_ok()]);
    await pump(tester, source);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    await tester.pumpAndSettle();
    expect(source.loads, 1);
    expect(find.text('重启服务'), findsOneWidget);
    expect(find.text('查看日志'), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsNothing);
  });

  testWidgets('shows the cached list immediately while refreshing',
      (tester) async {
    final pending = Completer<TerminalSnippetLoadResult>();
    final source = _FakeRepository([pending.future], cached: [_list.first]);
    await pump(tester, source);
    await tester.pump();
    expect(find.text('重启服务'), findsOneWidget);
    expect(find.text('查看日志'), findsNothing);
    pending.complete(_ok());
    await tester.pumpAndSettle();
    expect(find.text('查看日志'), findsOneWidget);
  });

  testWidgets('tapping a row inserts, the run button executes', (tester) async {
    await pump(tester, _FakeRepository([_ok()]));
    await tester.pumpAndSettle();
    await tester.tap(find.text('查看日志'));
    expect(selected, [('Get-Content C:\\log.txt\nGet-Date', false)]);
    await tester.tap(find.byKey(const ValueKey('terminal-snippet-run-1')));
    expect(selected.last, ('Restart-Service rustdesk', true));
    expect(selected, hasLength(2));
  });

  testWidgets('search filters by name and command, case-insensitive',
      (tester) async {
    await pump(tester, _FakeRepository([_ok()]));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'get-date');
    await tester.pump();
    expect(find.text('查看日志'), findsOneWidget);
    expect(find.text('重启服务'), findsNothing);
    await tester.enterText(find.byType(TextField), '重启');
    await tester.pump();
    expect(find.text('重启服务'), findsOneWidget);
    expect(find.text('查看日志'), findsNothing);
    await tester.enterText(find.byType(TextField), 'zzz');
    await tester.pump();
    expect(find.text('No matching terminal snippets'), findsOneWidget);
  });

  testWidgets('each failure state shows its own message without a list',
      (tester) async {
    final cases = {
      TerminalSnippetLoadStatus.notLoggedIn: 'Log in to use terminal snippets',
      TerminalSnippetLoadStatus.unauthorized: 'Log in to use terminal snippets',
      TerminalSnippetLoadStatus.forbidden:
          'Terminal snippets are only available to global administrators',
      TerminalSnippetLoadStatus.unsupported:
          'This server does not support terminal snippets',
    };
    for (final entry in cases.entries) {
      await pump(tester, _FakeRepository([_status(entry.key)]));
      await tester.pumpAndSettle();
      expect(find.text(entry.value), findsOneWidget, reason: '${entry.key}');
      expect(find.text('重启服务'), findsNothing);
      expect(find.text('Retry'), findsNothing);
    }
  });

  testWidgets('an empty list points to the web console', (tester) async {
    await pump(tester, _FakeRepository([_ok(const [])]));
    await tester.pumpAndSettle();
    expect(
        find.text(
            'No terminal snippets yet, add them on the web terminal page'),
        findsOneWidget);
  });

  testWidgets('a failed load can be retried', (tester) async {
    final source =
        _FakeRepository([_status(TerminalSnippetLoadStatus.failed), _ok()]);
    await pump(tester, source);
    await tester.pumpAndSettle();
    expect(find.text('Failed to load terminal snippets'), findsOneWidget);
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(source.loads, 2);
    expect(find.text('重启服务'), findsOneWidget);
    expect(find.text('Failed to load terminal snippets'), findsNothing);
  });

  testWidgets('a failed refresh keeps the offline list and says so',
      (tester) async {
    final source =
        _FakeRepository([_status(TerminalSnippetLoadStatus.failed, _list)]);
    await pump(tester, source);
    await tester.pumpAndSettle();
    expect(find.text('重启服务'), findsOneWidget);
    expect(
        find.text('Refresh failed, showing the last loaded terminal snippets'),
        findsOneWidget);
  });

  testWidgets('a permission loss clears a cached list', (tester) async {
    final pending = Completer<TerminalSnippetLoadResult>();
    final source = _FakeRepository([pending.future], cached: _list);
    await pump(tester, source);
    await tester.pump();
    expect(find.text('重启服务'), findsOneWidget);
    pending.complete(_status(TerminalSnippetLoadStatus.forbidden));
    await tester.pumpAndSettle();
    expect(find.text('重启服务'), findsNothing);
    expect(
        find.text(
            'Terminal snippets are only available to global administrators'),
        findsOneWidget);
  });
}
