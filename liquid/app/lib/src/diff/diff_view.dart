import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';

/// The changes of the latest run against the project before it: a folder tree of changed files
/// and the selected file's unified diff. Up and Down select files.
class DiffView extends StatefulWidget {
  const DiffView({super.key, required this.state});

  final SessionState state;

  @override
  State<DiffView> createState() => _DiffViewState();
}

class _DiffViewState extends State<DiffView> {
  List<BridgeFileChange> _changes = const [];
  List<FileRow> _rows = const [];
  BridgeFileChange? _selected;
  List<DiffLine> _lines = const [];
  bool _loaded = false;
  final _focus = FocusNode(debugLabel: 'diff');

  @override
  void initState() {
    super.initState();
    widget.state.status.addListener(_reloadWhenIdle);
    _reload();
    // Opening the tab moves the keyboard to it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    widget.state.status.removeListener(_reloadWhenIdle);
    _focus.dispose();
    super.dispose();
  }

  void _reloadWhenIdle() {
    if (widget.state.status.value == RunStatus.idle) _reload();
  }

  /// Reports why changes cannot be read, such as a diff over Git's output limit.
  Future<T?> _read<T>(Future<T> Function() read) async {
    try {
      return await read();
    } on Object catch (error) {
      widget.state.notify(
        BridgeLevel.error,
        'Could not show the changes: $error',
      );
      return null;
    }
  }

  Future<void> _reload() async {
    final changes = await _read(widget.state.changes);
    if (!mounted || changes == null) return;
    setState(() {
      _changes = changes;
      _rows = fileTree(changes);
      _loaded = true;
    });
    final files = [for (final row in _rows) ?row.change];
    final keep = files
        .where((change) => change.path == _selected?.path)
        .firstOrNull;
    await _select(keep ?? files.firstOrNull);
  }

  Future<void> _select(BridgeFileChange? change) async {
    // The selection moves at once, so quick arrow presses each move on from it.
    setState(() {
      _selected = change;
      _lines = const [];
    });
    final patch = change == null
        ? ''
        : await _read(() => widget.state.patch(change.path));
    // A newer selection replaced this one while its patch loaded.
    if (!mounted || patch == null || _selected != change) return;
    setState(() => _lines = parsePatch(patch));
  }

  void _move(int step) {
    final files = [for (final row in _rows) ?row.change];
    final index = files.indexWhere((change) => change.path == _selected?.path);
    final next = index + step;
    if (next >= 0 && next < files.length) _select(files[next]);
  }

  @override
  Widget build(BuildContext context) => CallbackShortcuts(
    bindings: {
      const SingleActivator(LogicalKeyboardKey.arrowDown): () => _move(1),
      const SingleActivator(LogicalKeyboardKey.arrowUp): () => _move(-1),
    },
    child: Focus(focusNode: _focus, child: _content(context)),
  );

  Widget _content(BuildContext context) {
    final ws = context.ws;
    if (!_loaded) return const SizedBox.shrink();
    if (_changes.isEmpty) {
      return Center(
        child: Text(
          'No changes in this run',
          style: Theme.of(context).textTheme.bodySmall,
        ),
      );
    }
    final added = _changes.fold(0, (sum, change) => sum + change.added);
    final removed = _changes.fold(0, (sum, change) => sum + change.removed);
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          width: 264,
          decoration: BoxDecoration(
            border: Border(right: BorderSide(color: ws.border)),
          ),
          child: ListView(
            padding: const EdgeInsets.all(WsMetrics.space3),
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  WsMetrics.space3,
                  WsMetrics.space1,
                  0,
                  WsMetrics.space2,
                ),
                child: Text(
                  '${_changes.length} files · +$added −$removed',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
              for (final row in _rows)
                _FileTile(
                  row: row,
                  selected: row.change?.path == _selected?.path,
                  onTap: _select,
                ),
            ],
          ),
        ),
        Expanded(
          child: _Patch(change: _selected, lines: _lines),
        ),
      ],
    );
  }
}

class _FileTile extends StatelessWidget {
  const _FileTile({
    required this.row,
    required this.selected,
    required this.onTap,
  });

  final FileRow row;
  final bool selected;
  final ValueChanged<BridgeFileChange> onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final change = row.change;
    Widget letter(String mark, Color color) => Text(
      mark,
      style: TextStyle(
        color: color,
        fontSize: WsMetrics.fsSmall,
        fontWeight: WsMetrics.fwSemibold,
      ),
    );
    final lead = switch (change?.status) {
      BridgeChangeStatus.added => letter('A', ws.success),
      BridgeChangeStatus.deleted => letter('D', ws.danger),
      BridgeChangeStatus.modified => letter('M', ws.warning),
      null => Icon(LucideIcons.chevronDown, size: 14, color: ws.textFaint),
    };
    return InkWell(
      canRequestFocus: false,
      onTap: change == null ? null : () => onTap(change),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      child: Container(
        height: WsMetrics.rowNav,
        padding: EdgeInsets.only(
          left: WsMetrics.space3 + row.depth * 16,
          right: WsMetrics.space3,
        ),
        decoration: BoxDecoration(
          color: selected ? ws.selected : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          children: [
            SizedBox(width: 16, child: lead),
            if (change == null) ...[
              const SizedBox(width: WsMetrics.space1),
              Icon(LucideIcons.folder, size: 14, color: ws.textFaint),
            ],
            const SizedBox(width: WsMetrics.space2),
            Expanded(
              child: Text(
                row.name,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: selected ? ws.text : ws.textMuted,
                  fontSize: 13,
                ),
              ),
            ),
            if (change != null)
              Text.rich(
                TextSpan(
                  children: [
                    if (change.added > 0)
                      TextSpan(
                        text: '+${change.added} ',
                        style: TextStyle(color: ws.success),
                      ),
                    if (change.removed > 0)
                      TextSpan(
                        text: '−${change.removed}',
                        style: TextStyle(color: ws.danger),
                      ),
                  ],
                ),
                style: const TextStyle(fontSize: WsMetrics.fsSmall),
              ),
          ],
        ),
      ),
    );
  }
}

class _Patch extends StatelessWidget {
  const _Patch({required this.change, required this.lines});

  final BridgeFileChange? change;
  final List<DiffLine> lines;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final change = this.change;
    if (change == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space7,
        WsMetrics.space5,
        WsMetrics.space7,
        0,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Text(
                'Changes in this run',
                style: Theme.of(context).textTheme.labelMedium
                    ?.copyWith(fontWeight: WsMetrics.fwSemibold),
              ),
              const SizedBox(width: WsMetrics.space4),
              Text(
                'compared with the project before the run',
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ],
          ),
          const SizedBox(height: WsMetrics.space4),
          Container(
            height: 36,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: const BorderRadius.vertical(
                top: Radius.circular(WsMetrics.radiusLg),
              ),
            ),
            alignment: Alignment.centerLeft,
            child: Text(change.path, style: monoStyle(context)),
          ),
          Expanded(
            child: DecoratedBox(
              decoration: BoxDecoration(
                border: Border.all(color: ws.border),
                borderRadius: const BorderRadius.vertical(
                  bottom: Radius.circular(WsMetrics.radiusLg),
                ),
              ),
              // Long lines scroll sideways together, so no change is out of reach.
              child: LayoutBuilder(
                builder: (context, constraints) {
                  final char = (TextPainter(
                    text: TextSpan(text: 'M', style: monoStyle(context)),
                    textDirection: TextDirection.ltr,
                  )..layout()).width;
                  final widest = lines.fold(
                    0,
                    (widest, line) => math.max(widest, line.text.length),
                  );
                  return SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    child: SizedBox(
                      width: math.max(
                        constraints.maxWidth,
                        _LineRow.gutter + widest * char + WsMetrics.space6,
                      ),
                      height: constraints.maxHeight,
                      child: ListView.builder(
                        itemCount: lines.length,
                        itemBuilder: (context, index) =>
                            _LineRow(line: lines[index]),
                      ),
                    ),
                  );
                },
              ),
            ),
          ),
          const SizedBox(height: WsMetrics.space4),
        ],
      ),
    );
  }
}

class _LineRow extends StatelessWidget {
  const _LineRow({required this.line});

  /// Width of the two line numbers and the sign before the code.
  static const gutter = 44.0 + 44.0 + 20.0;

  final DiffLine line;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (background, gutter, sign) = switch (line.kind) {
      DiffKind.added => (ws.diffAdd, ws.diffAddStrong, '+'),
      DiffKind.removed => (ws.diffDel, ws.diffDelStrong, '−'),
      DiffKind.hunk => (ws.subtle, ws.subtle, ''),
      DiffKind.context => (null, null, ''),
    };
    final number = TextStyle(
      color: ws.textFaint,
      fontSize: WsMetrics.fsSmall,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    Widget cell(int? value) => Container(
      width: 44,
      color: gutter,
      padding: const EdgeInsets.only(right: WsMetrics.space4),
      alignment: Alignment.centerRight,
      child: Text(value?.toString() ?? '', style: number),
    );
    return Container(
      height: 22,
      color: background,
      child: Row(
        children: [
          cell(line.oldNumber),
          cell(line.newNumber),
          SizedBox(
            width: 20,
            child: Text(
              sign,
              textAlign: TextAlign.center,
              style: TextStyle(
                color: line.kind == DiffKind.added ? ws.success : ws.danger,
              ),
            ),
          ),
          Expanded(
            child: line.kind == DiffKind.hunk
                ? Text(
                    line.text,
                    style: monoStyle(
                      context,
                      color: ws.textFaint,
                    ).copyWith(fontSize: WsMetrics.fsSmall),
                  )
                : Text.rich(
                    highlight(context, line.text),
                    maxLines: 1,
                    overflow: TextOverflow.clip,
                    softWrap: false,
                  ),
          ),
        ],
      ),
    );
  }
}

final _token = RegExp(
  r'(//.*$)|("(?:[^"\\]|\\.)*"|'
  r"'(?:[^'\\]|\\.)*')"
  r'|\b(import|from|export|const|let|var|async|await|return|if|else|function|fn|pub|use|let|mut|impl|struct|enum|match|class|new)\b|\b(\d[\d_.]*)\b',
);

/// Light syntax colors for common languages; unknown text stays plain.
TextSpan highlight(BuildContext context, String code) {
  final ws = context.ws;
  final base = monoStyle(context);
  final spans = <TextSpan>[];
  var start = 0;
  for (final match in _token.allMatches(code)) {
    if (match.start > start) {
      spans.add(TextSpan(text: code.substring(start, match.start)));
    }
    final color = match.group(1) != null
        ? ws.synCom
        : match.group(2) != null
        ? ws.synStr
        : match.group(3) != null
        ? ws.synKw
        : ws.synNum;
    spans.add(
      TextSpan(
        text: match.group(0),
        style: TextStyle(color: color),
      ),
    );
    start = match.end;
  }
  if (start < code.length) spans.add(TextSpan(text: code.substring(start)));
  return TextSpan(style: base, children: spans);
}
