import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;

import '../utils/http_service.dart' as http_service;
import 'platform_model.dart';

/// One terminal snippet maintained by the account on the rd-api web console.
class TerminalSnippet {
  const TerminalSnippet({
    required this.id,
    required this.name,
    required this.command,
    required this.createdAt,
    required this.updatedAt,
  });

  final int id;
  final String name;
  final String command;
  final int createdAt;
  final int updatedAt;

  static TerminalSnippet? tryParse(Object? raw) {
    if (raw is! Map) return null;
    final id = raw['id'];
    final name = raw['name'];
    final command = raw['command'];
    if (id is! int || name is! String || command is! String) return null;
    final createdAt = raw['created_at'];
    final updatedAt = raw['updated_at'];
    return TerminalSnippet(
      id: id,
      name: name,
      command: command,
      createdAt: createdAt is int ? createdAt : 0,
      updatedAt: updatedAt is int ? updatedAt : 0,
    );
  }
}

enum TerminalSnippetLoadStatus {
  ok,
  notLoggedIn,
  unauthorized,
  forbidden,
  unsupported,
  failed,
}

class TerminalSnippetLoadResult {
  const TerminalSnippetLoadResult(this.status, this.snippets, {this.limit});

  final TerminalSnippetLoadStatus status;

  /// For [TerminalSnippetLoadStatus.failed] this is the last list loaded with
  /// the same access token, so the picker stays usable offline.
  final List<TerminalSnippet> snippets;
  final int? limit;
}

/// What the snippet picker needs; lets widget tests avoid network and FFI.
abstract class TerminalSnippetSource {
  List<TerminalSnippet>? get cached;
  Future<TerminalSnippetLoadResult> load();
}

typedef TerminalSnippetHttpGet = Future<http.Response> Function(
    Uri url, Map<String, String> headers);

/// Read-only sync of `GET /api/terminal-snippets` for the logged-in account.
///
/// The list stays in memory only: snippets may hold credentials or internal
/// hosts, so they are never written to local options or files.
class TerminalSnippetRepository implements TerminalSnippetSource {
  TerminalSnippetRepository({
    required this.apiServer,
    required this.accessToken,
    required this.httpGet,
  });

  factory TerminalSnippetRepository.platform() => TerminalSnippetRepository(
        apiServer: () => bind.mainGetApiServer(),
        accessToken: () => bind.mainGetLocalOption(key: 'access_token'),
        httpGet: (url, headers) => http_service.get(url, headers: headers),
      );

  static final TerminalSnippetRepository shared =
      TerminalSnippetRepository.platform();

  final Future<String> Function() apiServer;
  final String Function() accessToken;
  final TerminalSnippetHttpGet httpGet;

  String? _cacheToken;
  String? _cacheBase;
  List<TerminalSnippet>? _cache;

  @override
  List<TerminalSnippet>? get cached {
    final token = accessToken();
    if (token.isEmpty || token != _cacheToken) return null;
    return _cache;
  }

  @override
  Future<TerminalSnippetLoadResult> load() async {
    final token = accessToken();
    if (token.isEmpty) {
      _clear();
      return const TerminalSnippetLoadResult(
          TerminalSnippetLoadStatus.notLoggedIn, []);
    }
    final api = (await apiServer()).trim();
    if (api.isEmpty) return _failed(token);
    final base = api.endsWith('/') ? api.substring(0, api.length - 1) : api;
    if (base != _cacheBase) _clear();
    final http.Response response;
    try {
      response = await httpGet(Uri.parse('$base/api/terminal-snippets'),
          {'Authorization': 'Bearer $token'});
    } catch (e) {
      debugPrint('[TerminalSnippet] load failed: ${e.runtimeType}');
      return _failed(token);
    }
    switch (response.statusCode) {
      case 200:
        return _parse(token, base, response) ?? _failed(token);
      case 401:
        _clear();
        return const TerminalSnippetLoadResult(
            TerminalSnippetLoadStatus.unauthorized, []);
      case 403:
        _clear();
        return const TerminalSnippetLoadResult(
            TerminalSnippetLoadStatus.forbidden, []);
      case 404:
        _clear();
        return const TerminalSnippetLoadResult(
            TerminalSnippetLoadStatus.unsupported, []);
      default:
        debugPrint('[TerminalSnippet] load status ${response.statusCode}');
        return _failed(token);
    }
  }

  TerminalSnippetLoadResult? _parse(
      String token, String base, http.Response response) {
    final Object? body;
    try {
      body = jsonDecode(utf8.decode(response.bodyBytes));
    } catch (_) {
      return null;
    }
    if (body is! Map || body['data'] is! List) return null;
    final snippets = List<TerminalSnippet>.unmodifiable((body['data'] as List)
        .map(TerminalSnippet.tryParse)
        .whereType<TerminalSnippet>());
    _cacheToken = token;
    _cacheBase = base;
    _cache = snippets;
    final limit = body['limit'];
    return TerminalSnippetLoadResult(TerminalSnippetLoadStatus.ok, snippets,
        limit: limit is int ? limit : null);
  }

  TerminalSnippetLoadResult _failed(String token) => TerminalSnippetLoadResult(
      TerminalSnippetLoadStatus.failed,
      token == _cacheToken ? (_cache ?? const []) : const []);

  void _clear() {
    _cacheToken = null;
    _cacheBase = null;
    _cache = null;
  }
}

final _lineBreaks = RegExp(r'\r\n|[\r\n]');
final _repeatedSpaces = RegExp(r' {2,}');
final _trailingNewlines = RegExp(r'[\r\n]+$');

/// Text for "insert": one line, no Enter. Matches the web console so a
/// multi-line snippet can never run line by line on shells without
/// bracketed paste (Windows PowerShell 5.1, cmd).
String terminalSnippetFillText(String command) {
  if (command.trim().isEmpty) return '';
  return command
      .replaceAll(_lineBreaks, ' ')
      .replaceAll(_repeatedSpaces, ' ')
      .trim();
}

/// Body for "run": inner lines kept, trailing newlines removed; the caller
/// sends exactly one Enter afterwards. Null when there is nothing to run.
String? terminalSnippetRunBody(String command) {
  final body = command.replaceAll(_trailingNewlines, '');
  return body.trim().isEmpty ? null : body;
}

/// Writes a snippet into a terminal. Enter goes as a separate key so it still
/// executes when the remote shell has bracketed paste enabled.
Future<bool> sendTerminalSnippet({
  required String command,
  required bool execute,
  required Future<void> Function(String text) paste,
  required Future<void> Function(String key) sendKey,
}) async {
  if (!execute) {
    final text = terminalSnippetFillText(command);
    if (text.isEmpty) return false;
    await paste(text);
    return true;
  }
  final body = terminalSnippetRunBody(command);
  if (body == null) return false;
  await paste(body);
  await sendKey('\r');
  return true;
}
