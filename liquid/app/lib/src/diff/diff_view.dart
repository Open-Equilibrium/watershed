import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_lines.dart';
import 'diff_model.dart';
import 'diff_settings.dart';
import 'file_tile.dart';
import 'patch_pane.dart';
import 'revert_dialog.dart';
import 'turn_header.dart';
import 'turn_scope.dart';

String diffKeys() =>
    '${Fixed.turnMenu.display} turn · ${Fixed.previousOrNextFile.display} file · '
    '${Fixed.foldFolder.display} folder · ${Fixed.previousOrNextHunk.display} hunk · '
    '${Fixed.unifiedOrSplit.display} unified/split · ${Fixed.hideWhitespace.display} whitespace';

class DiffView extends StatefulWidget {
  const DiffView({
    super.key,
    required this.state,
    required this.settings,
    this.parse = parsePatchInBackground,
  });

  final SessionState state;

  final DiffSettingsController settings;

  final PatchParser parse;

  @override
  State<DiffView> createState() => _DiffViewState();
}

class _DiffViewState extends State<DiffView> {
  List<BridgeFileChange> _changes = const [];
  List<FileRow> _rows = const [];
  BridgeFileChange? _selected;
  List<DiffLine> _lines = const [];

  List<String> _widest = const [];

  String? _note;
  String? _shownPath;
  String? _shownComparison;

  static const _textless =
      'No text changes to show, as for a binary file or a new file mode.';
  bool _loaded = false;
  final _focus = FocusNode(debugLabel: 'diff');

  final _folded = <String>{};

  bool _loading = false;
  bool _requested = false;

  final _scroll = ScrollController();

  final _turnMenu = GlobalKey<PopupMenuButtonState<DiffScope>>();

  @override
  void initState() {
    super.initState();
    widget.state.changed.addListener(_show);
    widget.state.shown.addListener(_rebuild);
    widget.state.shownScope.addListener(_rebuild);
    unawaited(widget.state.readChanges());
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _focus.requestFocus();
      _show();
    });
  }

  @override
  void dispose() {
    widget.state.changed.removeListener(_show);
    widget.state.shown.removeListener(_rebuild);
    widget.state.shownScope.removeListener(_rebuild);
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _rebuild() => setState(() {});

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
    setState(() {
      _selected = change;
      if (change?.path != _shownPath || _comparison != _shownComparison) {
        _lines = const [];
        _widest = const [];
        _note = null;
      }
    });
    if (_loading) {
      _requested = true;
      return;
    }
    _loading = true;
    try {
      while (mounted) {
        final wanted = _selected;
        final comparison = _comparison;
        if (wanted == null) return;
        _requested = false;
        String? note;
        var lines = const <DiffLine>[];
        try {
          lines = await widget.parse(await widget.state.patch(wanted.path));
          if (lines.isEmpty) note = _textless;
        } on Object catch (error) {
          note = 'Could not show the changes: $error';
        }
        if (!mounted ||
            _requested ||
            _selected != wanted ||
            _comparison != comparison) {
          continue;
        }
        final keepPosition =
            _shownPath == wanted.path && _shownComparison == comparison;
        setState(() {
          _shownPath = wanted.path;
          _shownComparison = comparison;
          _lines = lines;
          _widest = widestLines(lines);
          _note = note;
        });
        if (!keepPosition && _scroll.hasClients) _scroll.jumpTo(0);
        return;
      }
    } finally {
      _loading = false;
    }
  }

  String get _comparison {
    final shown = widget.state.shown.value;
    return '${widget.state.shownScope.value.runtimeType} ${shown?.from.number}..'
        '${shown?.to.number} ${widget.state.hideWhitespace.value}';
  }

  bool _hidden(FileRow row) => _folded.any(
    (folder) =>
        row.folder.startsWith('$folder/') ||
        (row.change != null && row.folder == folder),
  );

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

  void _unfold() {
    final folder = _selectedFolder;
    setState(
      () => _folded.removeWhere(
        (folded) => folder == folded || folder.startsWith('$folded/'),
      ),
    );
  }

  DiffLayout get _layout => widget.settings.value.layout;

  List<int> get _hunks => _layout == DiffLayout.split
      ? [
          for (final (index, row) in splitRows(_lines).indexed)
            if (row.hunk != null) index,
        ]
      : [
          for (final (index, line) in _lines.indexed)
            if (line.kind == DiffKind.hunk) index,
        ];

  void _jump(int step) {
    if (!_scroll.hasClients) return;
    final position = _scroll.position;
    final height = UnifiedLineRow.heightIn(MediaQuery.textScalerOf(context));
    final offsets = [for (final row in _hunks) row * height];
    final now = position.pixels;
    final target = step > 0
        ? offsets.where((offset) => offset > now + 1).firstOrNull
        : offsets.where((offset) => offset < now - 1).lastOrNull;
    if (target == null) return;
    _scroll.jumpTo(
      target.clamp(position.minScrollExtent, position.maxScrollExtent),
    );
  }

  void _change(DiffSettings settings) {
    unawaited(widget.settings.change(settings));
    setState(() {});
  }

  void _showLayout(DiffLayout layout) =>
      _change(widget.settings.value.copyWith(layout: layout));

  void _switchLayout() => _showLayout(
    _layout == DiffLayout.unified ? DiffLayout.split : DiffLayout.unified,
  );

  void _switchWhitespace() => _change(
    widget.settings.value.copyWith(
      hideWhitespace: !widget.settings.value.hideWhitespace,
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
      ...Fixed.turnMenu.bind([() => _turnMenu.currentState?.showButtonMenu()]),
    },
    child: Focus(
      focusNode: _focus,
      child: ValueListenableBuilder(
        valueListenable: widget.state.git,
        builder: (context, git, _) => ListenableBuilder(
          listenable: Listenable.merge([
            git.status,
            widget.state.changesProblem,
          ]),
          builder: (context, _) {
            final repository = git.status.value?.repository ?? false;
            final problem = widget.state.changesProblem.value;
            return problem == null
                ? _content(context, repository)
                : _empty(context, repository, problem.title, problem.text);
          },
        ),
      ),
    ),
  );

  Widget? _header(bool repository) {
    final shown = widget.state.shown.value;
    final scope = widget.state.shownScope.value;
    if (shown == null && !repository) return null;
    final files = widget.state.changed.value;
    return TurnHeader(
      menuKey: _turnMenu,
      turns: widget.state.turns.value,
      scope: scope,
      allLines:
          scope is AllTurns &&
              files != null &&
              widget.state.changesProblem.value == null
          ? (
              added: files.fold(0, (sum, file) => sum + file.added),
              removed: files.fold(0, (sum, file) => sum + file.removed),
            )
          : null,
      repository: repository,
      onSelect: (scope) => unawaited(widget.state.showTurns(scope)),
      onRevert: switch ((scope, shown)) {
        (OneTurn() || LatestTurn(), final shown?) => () => unawaited(
          confirmRevert(context, widget.state, shown.to.number),
        ),
        _ => null,
      },
    );
  }

  Widget _empty(
    BuildContext context,
    bool repository,
    String title,
    String text,
  ) {
    final header = _header(repository);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (header != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space7,
              WsMetrics.space5,
              WsMetrics.space7,
              0,
            ),
            child: header,
          ),
        Expanded(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    title,
                    style: Theme.of(context).textTheme.labelMedium
                        ?.copyWith(fontWeight: WsMetrics.fwMedium),
                  ),
                  const SizedBox(height: WsMetrics.space2),
                  Text(
                    text,
                    textAlign: TextAlign.center,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _content(BuildContext context, bool repository) {
    final ws = context.ws;
    if (!_loaded) {
      return const SizedBox.expand();
    }
    if (_changes.isEmpty) {
      final scope = widget.state.shownScope.value;
      final title = switch (scope) {
        UncommittedChanges() => 'No uncommitted changes',
        BranchChanges() => 'No changes on this branch',
        AllTurns() => 'No changes in these turns',
        _ => 'No changes in this turn',
      };
      return widget.settings.value.hideWhitespace
          ? _empty(
              context,
              repository,
              '$title with whitespace hidden',
              '${Fixed.hideWhitespace.display} shows them.',
            )
          : _empty(context, repository, title, switch (scope) {
              UncommittedChanges() =>
                'The checkout is the same as its last commit.',
              BranchChanges() => 'The branch is the same as its base.',
              _ => 'The project was the same at its end as at its start.',
            });
    }
    final added = _changes.fold(0, (sum, change) => sum + change.added);
    final removed = _changes.fold(0, (sum, change) => sum + change.removed);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          constraints: const BoxConstraints(maxHeight: 200),
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: ws.border)),
          ),
          child: ListView(
            shrinkWrap: true,
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
                  FileTile(
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
          child: PatchPane(
            header: _header(repository),
            change: _selected,
            lines: _lines,
            widest: _widest,
            note: _note,
            settings: widget.settings.value,
            scroll: _scroll,
            onLayout: _showLayout,
            onWhitespace: _switchWhitespace,
          ),
        ),
      ],
    );
  }
}
