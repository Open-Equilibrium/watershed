import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../agent/input_lane.dart';
import '../controls.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';
import 'diff_settings.dart';

/// Keys of the Diff tab, for the status bar (approved mockup).
String diffKeys() =>
    '${Fixed.previousOrNextFile.display} file · ${Fixed.foldFolder.display} folder · '
    '${Fixed.previousOrNextHunk.display} hunk · ${Fixed.unifiedOrSplit.display} unified/split · '
    '${Fixed.hideWhitespace.display} whitespace';

/// The changes of the latest run against the project before it: a folder tree of changed files
/// and the selected file's diff, unified or split. Up and Down select files, Left and Right fold
/// and unfold the selected file's folders, ⌥Up and ⌥Down move between the file's changes.
class DiffView extends StatefulWidget {
  const DiffView({
    super.key,
    required this.state,
    required this.settings,
    this.input,
  });

  final SessionState state;

  /// How the tab shows the changes, kept across restarts.
  final DiffSettingsController settings;

  /// The input area, floating over the diff.
  final Widget? input;

  @override
  State<DiffView> createState() => _DiffViewState();
}

class _DiffViewState extends State<DiffView> {
  List<BridgeFileChange> _changes = const [];
  List<FileRow> _rows = const [];
  BridgeFileChange? _selected;
  List<DiffLine> _lines = const [];

  /// Why the selected file shows no lines, if it shows none.
  String? _note;

  static const _textless =
      'No text changes to show, as for a binary file or a new file mode.';
  bool _loaded = false;
  final _focus = FocusNode(debugLabel: 'diff');

  /// The folded folders, by path.
  final _folded = <String>{};

  /// Whether a patch is loading; a selection made meanwhile loads after it.
  bool _loading = false;

  /// Scrolls the selected file's lines.
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    widget.state.changed.addListener(_show);
    widget.state.ran.addListener(_show);
    // Opening the tab reads the changes again, so it shows the project as it is now.
    unawaited(widget.state.readChanges());
    // It also shows the last list at once and moves the keyboard to it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _focus.requestFocus();
      _show();
    });
  }

  @override
  void dispose() {
    widget.state.changed.removeListener(_show);
    widget.state.ran.removeListener(_show);
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _show() {
    final changes = widget.state.changed.value;
    if (changes == null) return;
    setState(() {
      _changes = changes;
      _rows = fileTree(changes);
      _loaded = true;
    });
    final files = [for (final row in _rows) ?row.change];
    final keep = files
        .where((change) => change.path == _selected?.path)
        .firstOrNull;
    unawaited(_select(keep ?? files.firstOrNull));
  }

  Future<void> _select(BridgeFileChange? change) async {
    // The selection moves at once, so quick arrow presses each move on from it.
    setState(() {
      _selected = change;
      _lines = const [];
      _note = null;
    });
    if (_loading) return;
    _loading = true;
    try {
      // Selections passed while a patch loaded are never read.
      while (mounted) {
        final wanted = _selected;
        if (wanted == null) return;
        // Why the file shows no lines: a diff that could not be read, such as one over Git's
        // output limit, or a change without text.
        String? note;
        var lines = const <DiffLine>[];
        final what = 'the changes of ${wanted.path}';
        try {
          lines = parsePatch(await widget.state.patch(wanted.path));
          widget.state.reportRead(what, null);
          if (lines.isEmpty) note = _textless;
        } on Object catch (error) {
          // The pane names the file above it; the notification names it itself.
          note = 'Could not show the changes: $error';
          widget.state.reportRead(
            what,
            'Could not show the changes of ${wanted.shown}: $error',
          );
        }
        if (!mounted || _selected != wanted) continue;
        setState(() {
          _lines = lines;
          _note = note;
        });
        // Another file starts at its top.
        if (_scroll.hasClients) _scroll.jumpTo(0);
        return;
      }
    } finally {
      _loading = false;
    }
  }

  /// Whether a folded folder hides `row`.
  bool _hidden(FileRow row) => _folded.any(
    (folder) =>
        row.folder.startsWith('$folder/') ||
        (row.change != null && row.folder == folder),
  );

  /// Selects the next file in `step`'s direction from the selected one, passing files a
  /// folded folder hides.
  void _move(int step) {
    final files = [
      for (final row in _rows)
        if (row.change != null) row,
    ];
    final index = files.indexWhere(
      (row) => row.change!.path == _selected?.path,
    );
    for (
      var next = index + step;
      next >= 0 && next < files.length;
      next += step
    ) {
      if (!_hidden(files[next])) return unawaited(_select(files[next].change));
    }
  }

  void _toggle(String folder) => setState(() {
    if (!_folded.remove(folder)) _folded.add(folder);
  });

  /// The folder that holds the selected file; empty at the top.
  String get _selectedFolder =>
      _rows
          .where((row) => row.change?.path == _selected?.path)
          .firstOrNull
          ?.folder ??
      '';

  void _fold() {
    if (_selectedFolder.isNotEmpty) {
      setState(() => _folded.add(_selectedFolder));
    }
  }

  /// Unfolds every folder around the selected file, so it shows again.
  void _unfold() {
    final folder = _selectedFolder;
    setState(
      () => _folded.removeWhere(
        (folded) => folder == folded || folder.startsWith('$folded/'),
      ),
    );
  }

  DiffLayout get _layout => widget.settings.value.layout;

  /// The rows each hunk starts at, in the shown layout.
  List<int> get _hunks => _layout == DiffLayout.split
      ? [
          for (final (index, row) in splitRows(_lines).indexed)
            if (row.hunk != null) index,
        ]
      : [
          for (final (index, line) in _lines.indexed)
            if (line.kind == DiffKind.hunk) index,
        ];

  /// Scrolls to the previous or next change in the selected file.
  void _jump(int step) {
    if (!_scroll.hasClients) return;
    final position = _scroll.position;
    final offsets = [for (final row in _hunks) row * _LineRow.height];
    final now = position.pixels;
    final target = step > 0
        ? offsets.where((offset) => offset > now + 1).firstOrNull
        : offsets.where((offset) => offset < now - 1).lastOrNull;
    if (target == null) return;
    _scroll.jumpTo(
      target.clamp(position.minScrollExtent, position.maxScrollExtent),
    );
  }

  /// Uses `settings` at once and keeps them for the next start.
  Future<void> _change(DiffSettings settings) async {
    final saving = widget.settings.change(settings);
    setState(() {});
    try {
      await saving;
    } on FileSystemException catch (error) {
      widget.state.notify(
        BridgeLevel.warning,
        'Could not save the Diff settings: ${error.message}',
      );
    }
  }

  void _showLayout(DiffLayout layout) =>
      unawaited(_change(widget.settings.value.copyWith(layout: layout)));

  void _switchLayout() => _showLayout(
    _layout == DiffLayout.unified ? DiffLayout.split : DiffLayout.unified,
  );

  /// Leaves out or brings back the changes that are only whitespace.
  void _switchWhitespace() => unawaited(
    _change(
      widget.settings.value.copyWith(
        hideWhitespace: !widget.settings.value.hideWhitespace,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) => CallbackShortcuts(
    bindings: {
      ...Fixed.previousOrNextFile.bind([() => _move(-1), () => _move(1)]),
      ...Fixed.foldFolder.bind([_fold, _unfold]),
      ...Fixed.previousOrNextHunk.bind([() => _jump(-1), () => _jump(1)]),
      ...Fixed.unifiedOrSplit.bind([_switchLayout]),
      ...Fixed.hideWhitespace.bind([_switchWhitespace]),
    },
    child: Focus(
      focusNode: _focus,
      child: ValueListenableBuilder(
        valueListenable: widget.state.changesProblem,
        builder: (context, problem, _) => problem == null
            ? _content(context)
            : InputLane(
                input: widget.input,
                child: Center(
                  child: Text(
                    problem,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ),
              ),
      ),
    ),
  );

  Widget _content(BuildContext context) {
    final ws = context.ws;
    if (!_loaded) {
      return InputLane(input: widget.input, child: const SizedBox.expand());
    }
    if (_changes.isEmpty) {
      return InputLane(
        input: widget.input,
        child: Center(
          child: Text(
            !widget.state.ran.value
                ? 'No run since Liquid opened this session'
                : widget.settings.value.hideWhitespace
                ? 'No changes in this run with whitespace hidden; '
                      '${Fixed.hideWhitespace.display} shows them'
                : 'No changes in this run',
            style: Theme.of(context).textTheme.bodySmall,
          ),
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
                  '${_changes.length} file${_changes.length == 1 ? '' : 's'} · '
                  '+$added −$removed',
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ),
              for (final row in _rows)
                if (!_hidden(row))
                  _FileTile(
                    row: row,
                    selected: row.change?.path == _selected?.path,
                    folded: _folded.contains(row.folder),
                    onTap: _select,
                    onFold: _toggle,
                  ),
            ],
          ),
        ),
        Expanded(
          child: InputLane(
            input: widget.input,
            child: _Patch(
              change: _selected,
              lines: _lines,
              note: _note,
              settings: widget.settings.value,
              scroll: _scroll,
              bottom: widget.input == null ? 0 : inputClearance,
              onLayout: _showLayout,
              onWhitespace: _switchWhitespace,
            ),
          ),
        ),
      ],
    );
  }
}

class _FileTile extends StatelessWidget {
  const _FileTile({
    required this.row,
    required this.selected,
    required this.folded,
    required this.onTap,
    required this.onFold,
  });

  final FileRow row;
  final bool selected;

  /// Whether this folder row is folded.
  final bool folded;
  final ValueChanged<BridgeFileChange> onTap;
  final ValueChanged<String> onFold;

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
      null => Icon(
        folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
        size: 14,
        color: ws.textFaint,
      ),
    };
    return InkWell(
      canRequestFocus: false,
      onTap: change == null ? () => onFold(row.folder) : () => onTap(change),
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
                  fontSize: WsMetrics.fsCompact,
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
  const _Patch({
    required this.change,
    required this.lines,
    required this.note,
    required this.settings,
    required this.scroll,
    required this.bottom,
    required this.onLayout,
    required this.onWhitespace,
  });

  final BridgeFileChange? change;
  final List<DiffLine> lines;

  /// Why the file shows no lines.
  final String? note;
  final DiffSettings settings;
  final ScrollController scroll;

  /// Room below the last line for the floating input area.
  final double bottom;
  final ValueChanged<DiffLayout> onLayout;
  final VoidCallback onWhitespace;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final change = this.change;
    final note = this.note;
    if (change == null) return const SizedBox.shrink();
    final split = settings.layout == DiffLayout.split;
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
              Expanded(
                child: Text(
                  'compared with the project before the run',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ),
              _Toggle(
                label: 'Hide whitespace',
                on: settings.hideWhitespace,
                onTap: onWhitespace,
              ),
              const SizedBox(width: WsMetrics.space3),
              Segments(
                options: const [
                  (DiffLayout.unified, 'Unified'),
                  (DiffLayout.split, 'Split'),
                ],
                selected: settings.layout,
                onSelect: onLayout,
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
            child: Text(change.shown, style: monoStyle(context)),
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
              child: note != null
                  ? Center(
                      child: Text(
                        note,
                        style: Theme.of(context).textTheme.bodySmall,
                      ),
                    )
                  : LayoutBuilder(
                      builder: (context, constraints) {
                        final char = measureText('M', monoStyle(context)).width;
                        final widest = lines.fold(
                          0,
                          (widest, line) => math.max(widest, line.text.length),
                        );
                        // Split shows each side as wide as the unified view's code.
                        final code = widest * char + WsMetrics.space6;
                        final rows = split
                            ? splitRows(lines)
                            : const <SplitRow>[];
                        return SingleChildScrollView(
                          scrollDirection: Axis.horizontal,
                          child: SizedBox(
                            width: math.max(
                              constraints.maxWidth,
                              split
                                  ? 2 * (_LineRow.side + code)
                                  : _LineRow.gutter + code,
                            ),
                            height: constraints.maxHeight,
                            child: ListView.builder(
                              controller: scroll,
                              padding: EdgeInsets.only(bottom: bottom),
                              itemExtent: _LineRow.height,
                              itemCount: split ? rows.length : lines.length,
                              itemBuilder: (context, index) => split
                                  ? _SplitLine(row: rows[index])
                                  : _LineRow(line: lines[index]),
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

/// A switch shown as a button that stays pressed while on (approved mockup).
class _Toggle extends StatelessWidget {
  const _Toggle({required this.label, required this.on, required this.onTap});

  final String label;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    // Its state is told to assistive technology, not only drawn.
    return Semantics(
      toggled: on,
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          child: Ink(
            height: 28,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: on ? ws.accentSubtle : null,
              border: on ? null : Border.all(color: ws.borderStrong),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Center(
              widthFactor: 1,
              child: Text(
                label,
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: on ? ws.accent : ws.textMuted,
                  fontWeight: on ? WsMetrics.fwMedium : WsMetrics.fwRegular,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The colors, sign and number of one side of a line.
({Color? background, Widget number, Widget sign}) _lineParts(
  BuildContext context,
  DiffLine line,
  int? value,
) {
  final ws = context.ws;
  final (background, gutter, sign) = switch (line.kind) {
    DiffKind.added => (ws.diffAdd, ws.diffAddStrong, '+'),
    DiffKind.removed => (ws.diffDel, ws.diffDelStrong, '−'),
    DiffKind.hunk => (ws.subtle, ws.subtle, ''),
    DiffKind.context => (null, null, ''),
  };
  return (
    background: background,
    number: Container(
      width: 44,
      color: gutter,
      padding: const EdgeInsets.only(right: WsMetrics.space4),
      alignment: Alignment.centerRight,
      // Muted, as faint lacks contrast on the strong diff colors.
      child: Text(
        value?.toString() ?? '',
        style: TextStyle(
          color: ws.textMuted,
          fontSize: WsMetrics.fsSmall,
          fontFeatures: const [FontFeature.tabularFigures()],
        ),
      ),
    ),
    sign: SizedBox(
      width: 20,
      child: Text(
        sign,
        textAlign: TextAlign.center,
        style: TextStyle(
          color: line.kind == DiffKind.added ? ws.success : ws.danger,
        ),
      ),
    ),
  );
}

/// A line's code, colored lightly, with a faint ␍ for a Windows line end; a hunk
/// header in the faint small style.
Widget _code(BuildContext context, DiffLine line) {
  final faint = context.ws.textFaint;
  if (line.kind == DiffKind.hunk) {
    return Text(
      line.text,
      style: monoStyle(
        context,
        color: faint,
      ).copyWith(fontSize: WsMetrics.fsSmall),
    );
  }
  final code = line.text.endsWith('\r')
      ? line.text.substring(0, line.text.length - 1)
      : null;
  final shown = highlight(context, code ?? line.text);
  return Text.rich(
    code == null
        ? shown
        : TextSpan(
            style: shown.style,
            children: [
              ...?shown.children,
              TextSpan(
                text: '␍',
                style: TextStyle(color: faint),
              ),
            ],
          ),
    maxLines: 1,
    overflow: TextOverflow.clip,
    softWrap: false,
  );
}

class _LineRow extends StatelessWidget {
  const _LineRow({required this.line});

  static const height = 22.0;

  /// Width of the two line numbers and the sign before the code.
  static const gutter = 44.0 + 44.0 + 20.0;

  /// Width of one side's line number and sign in the split view.
  static const side = 44.0 + 20.0;

  final DiffLine line;

  @override
  Widget build(BuildContext context) {
    final old = _lineParts(context, line, line.oldNumber);
    final now = _lineParts(context, line, line.newNumber);
    return Container(
      color: old.background,
      child: Row(
        children: [
          old.number,
          now.number,
          old.sign,
          Expanded(child: _code(context, line)),
        ],
      ),
    );
  }
}

/// One row of the split view: the line before the run on the left, after it on the right.
class _SplitLine extends StatelessWidget {
  const _SplitLine({required this.row});

  final SplitRow row;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (row.hunk case final hunk?) {
      return Container(
        color: ws.subtle,
        padding: const EdgeInsets.only(left: _LineRow.side),
        alignment: Alignment.centerLeft,
        child: _code(context, hunk),
      );
    }
    Widget side(DiffLine? line, {required bool before}) {
      // A side without a line, beside one only the other side has.
      if (line == null) return Expanded(child: ColoredBox(color: ws.subtle));
      final parts = _lineParts(
        context,
        line,
        before ? line.oldNumber : line.newNumber,
      );
      return Expanded(
        child: Container(
          color: parts.background,
          child: Row(
            children: [
              parts.number,
              parts.sign,
              Expanded(child: _code(context, line)),
            ],
          ),
        ),
      );
    }

    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        side(row.left, before: true),
        VerticalDivider(width: 1, color: ws.border),
        side(row.right, before: false),
      ],
    );
  }
}

final _token = RegExp(
  r'(//.*$)|("(?:[^"\\]|\\.)*"|'
  r"'(?:[^'\\]|\\.)*')"
  r'|\b(import|from|export|const|let|var|async|await|return|if|else|function|fn|pub|use|mut|impl|struct|enum|match|class|new)\b|\b(\d[\d_.]*)\b',
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
