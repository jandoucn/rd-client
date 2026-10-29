import 'package:flutter/material.dart' hide Dialog;
import 'package:flutter/material.dart' as material show Dialog;

import '../../common.dart';
import '../../models/terminal_model.dart';
import '../../models/terminal_snippet_model.dart';

typedef TerminalSnippetSelected = void Function(String command, bool execute);

/// Searchable list of the account's terminal snippets. Tapping a row inserts
/// the snippet as one line; the run button sends it followed by Enter.
class TerminalSnippetPanel extends StatefulWidget {
  const TerminalSnippetPanel({
    Key? key,
    required this.source,
    required this.onSelected,
    this.tr = translate,
  }) : super(key: key);

  final TerminalSnippetSource source;
  final TerminalSnippetSelected onSelected;
  final String Function(String) tr;

  @override
  State<TerminalSnippetPanel> createState() => _TerminalSnippetPanelState();
}

class _TerminalSnippetPanelState extends State<TerminalSnippetPanel> {
  final _search = TextEditingController();
  late List<TerminalSnippet> _snippets;
  TerminalSnippetLoadStatus? _status;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _snippets = widget.source.cached ?? const [];
    _load();
  }

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    setState(() => _loading = true);
    final result = await widget.source.load();
    if (!mounted) return;
    setState(() {
      _loading = false;
      _status = result.status;
      _snippets = result.status == TerminalSnippetLoadStatus.ok ||
              result.status == TerminalSnippetLoadStatus.failed
          ? result.snippets
          : const [];
    });
  }

  String? _blockingMessage() {
    switch (_status) {
      case TerminalSnippetLoadStatus.notLoggedIn:
      case TerminalSnippetLoadStatus.unauthorized:
        return 'Log in to use terminal snippets';
      case TerminalSnippetLoadStatus.forbidden:
        return 'Terminal snippets are only available to global administrators';
      case TerminalSnippetLoadStatus.unsupported:
        return 'This server does not support terminal snippets';
      default:
        return null;
    }
  }

  List<TerminalSnippet> get _filtered {
    final query = _search.text.trim().toLowerCase();
    if (query.isEmpty) return _snippets;
    return _snippets
        .where((s) =>
            s.name.toLowerCase().contains(query) ||
            s.command.toLowerCase().contains(query))
        .toList();
  }

  Widget _message(String text, {bool retry = false}) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(widget.tr(text), textAlign: TextAlign.center),
            if (retry) ...[
              const SizedBox(height: 12),
              OutlinedButton(onPressed: _load, child: Text(widget.tr('Retry'))),
            ],
          ],
        ),
      ),
    );
  }

  Widget _body() {
    if (_loading && _snippets.isEmpty) {
      return const Center(child: CircularProgressIndicator());
    }
    final blocking = _blockingMessage();
    if (blocking != null) return _message(blocking);
    if (_snippets.isEmpty) {
      return _status == TerminalSnippetLoadStatus.failed
          ? _message('Failed to load terminal snippets', retry: true)
          : _message(
              'No terminal snippets yet, add them on the web terminal page');
    }
    final items = _filtered;
    if (items.isEmpty) return _message('No matching terminal snippets');
    return ListView.builder(
      itemCount: items.length,
      itemBuilder: (context, index) {
        final snippet = items[index];
        return ListTile(
          dense: true,
          title:
              Text(snippet.name, maxLines: 1, overflow: TextOverflow.ellipsis),
          subtitle: Text(snippet.command,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontFamily: 'monospace')),
          onTap: () => widget.onSelected(snippet.command, false),
          trailing: IconButton(
            key: ValueKey('terminal-snippet-run-${snippet.id}'),
            icon: const Icon(Icons.play_arrow),
            tooltip: widget.tr('Run'),
            onPressed: () => widget.onSelected(snippet.command, true),
          ),
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final showOffline = !_loading &&
        _status == TerminalSnippetLoadStatus.failed &&
        _snippets.isNotEmpty;
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
          child: TextField(
            controller: _search,
            decoration: InputDecoration(
              isDense: true,
              prefixIcon: const Icon(Icons.search),
              hintText: widget.tr('Search'),
            ),
            onChanged: (_) => setState(() {}),
          ),
        ),
        if (_loading && _snippets.isNotEmpty)
          const LinearProgressIndicator(minHeight: 2),
        if (showOffline)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Text(
              widget.tr(
                  'Refresh failed, showing the last loaded terminal snippets'),
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
        Expanded(child: _body()),
      ],
    );
  }
}

/// Small translucent button laid over a terminal view.
class TerminalSnippetButton extends StatelessWidget {
  const TerminalSnippetButton({Key? key, required this.onPressed})
      : super(key: key);

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Colors.black.withOpacity(0.35),
      shape: const CircleBorder(),
      clipBehavior: Clip.antiAlias,
      child: IconButton(
        icon: const Icon(Icons.bolt, color: Colors.white, size: 18),
        tooltip: translate('Terminal snippets'),
        constraints: const BoxConstraints.tightFor(width: 32, height: 32),
        padding: EdgeInsets.zero,
        onPressed: onPressed,
      ),
    );
  }
}

/// Opens the picker (dialog on desktop, bottom sheet on mobile) and writes the
/// chosen snippet into [model]. The picker closes before anything is sent, so
/// one choice can never be sent twice.
Future<void> showTerminalSnippetPicker(
    BuildContext context, TerminalModel model) async {
  Widget panel(BuildContext ctx) => TerminalSnippetPanel(
        source: TerminalSnippetRepository.shared,
        onSelected: (command, execute) =>
            Navigator.of(ctx).pop((command, execute)),
      );
  final (String, bool)? picked;
  if (isDesktop || isWebDesktop) {
    picked = await showDialog<(String, bool)>(
      context: context,
      builder: (ctx) => material.Dialog(
        child: SizedBox(
          width: 520,
          height: 480,
          child: Column(
            children: [
              ListTile(
                title: Text(translate('Terminal snippets')),
                trailing: IconButton(
                  icon: const Icon(Icons.close),
                  tooltip: translate('Close'),
                  onPressed: () => Navigator.of(ctx).pop(),
                ),
              ),
              Expanded(child: panel(ctx)),
            ],
          ),
        ),
      ),
    );
  } else {
    picked = await showModalBottomSheet<(String, bool)>(
      context: context,
      isScrollControlled: true,
      builder: (ctx) => Padding(
        padding: EdgeInsets.only(bottom: MediaQuery.of(ctx).viewInsets.bottom),
        child: SafeArea(
          child: SizedBox(
            height: MediaQuery.of(ctx).size.height * 0.6,
            child: Column(
              children: [
                ListTile(title: Text(translate('Terminal snippets'))),
                Expanded(child: panel(ctx)),
              ],
            ),
          ),
        ),
      ),
    );
  }
  if (picked == null) return;
  await sendTerminalSnippet(
    command: picked.$1,
    execute: picked.$2,
    paste: model.pasteText,
    sendKey: model.sendVirtualKey,
  );
}
