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

/// Keys of the Diff panel, for the status bar.
String diffKeys() =>
    '${Fixed.turnMenu.display} turn · ${Fixed.previousOrNextFile.display} file · '
    '${Fixed.foldFolder.display} folder · ${Fixed.previousOrNextHunk.display} hunk · '
    '${Fixed.unifiedOrSplit.display} unified/split · ${Fixed.hideWhitespace.display} whitespace';

/// What a turn changed between its checkpoints, or all turns, or the uncommitted changes: a
/// folder tree of changed files and the selected file's diff, unified or split. Up and Down
/// select files, Left and Right fold and unfold the selected file's folders, ⌥Up and ⌥Down move
/// between the file's changes, T chooses the turn.
class DiffView extends StatefulWidget {
  const DiffView({
    super.key,
    required this.state,
    required this.settings,
    this.parse = parsePatchInBackground,
  });

  final SessionState state;

  /// How the panel shows the changes, kept across restarts.
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

  /// The shown lines likely widest, which set how far the lines scroll sideways.
  List<String> _widest = const [];

  /// Why the selected file shows no lines, if it shows none.
  String? _note;
  String? _shownPath;
  String? _shownComparison;

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

  /// Opens the scope menu from the keyboard.
  final _turnMenu = GlobalKey<PopupMenuButtonState<DiffScope>>();

  @override
  void initState() {
    super.initState();
    widget.state.changed.addListener(_show);
    widget.state.shown.addListener(_rebuild);
    widget.state.shownScope.addListener(_rebuild);
    // Opening the panel reads the turns again, so it shows the latest one.
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
    widget.state.shown.removeListener(_rebuild);
    widget.state.shownScope.removeListener(_rebuild);
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// Another turn shows its name, also when it changed the same files.
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
    // The selection moves at once, so quick arrow presses each move on from it.
    setState(() {
      _selected = change;
      if (change?.path != _shownPath || _comparison != _shownComparison) {
        _lines = const [];
        _widest = const [];
        _note = null;
      }
    });
    if (_loading) return;
    _loading = true;
    try {
      // Selections passed while a patch loaded are never read.
      while (mounted) {
        final wanted = _selected;
        final comparison = _comparison;
        if (wanted == null) return;
        // Why the file shows no lines: a diff that could not be read, such as one over Git's
        // output limit, or a change without text.
        String? note;
        var lines = const <DiffLine>[];
        try {
          lines = await widget.parse(await widget.state.patch(wanted.path));
          if (lines.isEmpty) note = _textless;
        } on Object catch (error) {
          note = 'Could not show the changes: $error';
        }
        // A newer selection or comparison reads again.
        if (!mounted || _selected != wanted || _comparison != comparison) {
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
        // Another file starts at its top.
        if (!keepPosition && _scroll.hasClients) _scroll.jumpTo(0);
        return;
      }
    } finally {
      _loading = false;
    }
  }

  /// What the shown patch compares: the turns or the uncommitted changes, and whether whitespace
  /// is hidden.
  String get _comparison {
    final shown = widget.state.shown.value;
    return '${widget.state.shownScope.value.runtimeType} ${shown?.from.number}..'
        '${shown?.to.number} ${widget.state.hideWhitespace.value}';
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

  /// Uses `settings` at once and keeps them for the next start.
  void _change(DiffSettings settings) {
    unawaited(widget.settings.change(settings));
    setState(() {});
  }

  void _showLayout(DiffLayout layout) =>
      _change(widget.settings.value.copyWith(layout: layout));

  void _switchLayout() => _showLayout(
    _layout == DiffLayout.unified ? DiffLayout.split : DiffLayout.unified,
  );

  /// Leaves out or brings back the changes that are only whitespace.
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

  /// The scope menu and what the shown turns are, once a turn ended or in a repository.
  Widget? _header(bool repository) {
    final shown = widget.state.shown.value;
    final scope = widget.state.shownScope.value;
    if (shown == null && !repository) return null;
    final files = widget.state.changed.value;
    return TurnHeader(
      menuKey: _turnMenu,
      turns: widget.state.turns.value,
      scope: scope,
      // What all turns changed is known once they were read.
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
      // Reverting applies to the turn shown, not to all turns.
      onRevert: switch ((scope, shown)) {
        (OneTurn() || LatestTurn(), final shown?) => () => unawaited(
          confirmRevert(context, widget.state, shown.to.number),
        ),
        _ => null,
      },
    );
  }

  /// Says why nothing is shown, below the scope menu when there is one.
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
        // The file list sits above the patch, across the Diff panel; a long list scrolls.
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
