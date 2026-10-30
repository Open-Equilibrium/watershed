import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../agent/input_lane.dart';
import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'compare_view.dart';
import 'context_details.dart';
import 'context_track.dart';
import 'filter_bar.dart';
import 'inspector.dart';
import 'overview.dart';
import 'step_parts.dart';
import 'track_cell.dart';
import 'trajectory_model.dart';
import 'tree_view.dart';

/// The tree starts open when the Trajectory is at least this wide, in about a 1280 px window.
const _treeOpenWidth = 1000.0;

/// Keys of the Trajectory tab, for the status bar.
String trajectoryKeys(ShortcutSettings keys) => [
  '${Fixed.previousOrNextStep.display} step',
  '${Fixed.foldTurn.display} collapse',
  keys.hint(Command.openStep, 'open'),
  keys.hint(Command.branch, 'branch'),
  keys.hint(Command.compare, 'compare'),
].where((part) => part.isNotEmpty).join(' · ');

/// Keys while comparing two branches.
String compareKeys(ShortcutSettings keys) => [
  '${Fixed.moveInTree.display} tree',
  keys.hint(Command.selectBranch, 'select branch'),
  keys.hint(Command.compare, 'leave compare'),
].where((part) => part.isNotEmpty).join(' · ');

/// The Trajectory tab (approved mockups 2 and 3 and the approved context track): one overview
/// on a shared time axis, the steps grouped by turn with their context and an inspector, and
/// the session tree. It reads the session's
/// stored history, so it looks the same during the run, after it and after a restart.
class TrajectoryView extends StatefulWidget {
  const TrajectoryView({
    super.key,
    required this.state,
    required this.onBranched,
    required this.comparing,
    this.input,
  });

  final SessionState state;

  /// Called after branching, so the input area can take the new branch's prompt.
  final VoidCallback onBranched;

  /// Whether two branches are compared, for the keys the status bar shows.
  final ValueNotifier<bool> comparing;

  /// The input area, floating over the steps.
  final Widget? input;

  @override
  State<TrajectoryView> createState() => _TrajectoryViewState();
}

class _TrajectoryViewState extends State<TrajectoryView> {
  final _focus = FocusNode(debugLabel: 'trajectory');
  final _scroll = ScrollController();
  BridgeHistory? _history;
  SessionGraph _graph = sessionGraph(const [], null);

  /// The tip of the shown branch; `null` shows the branch the next prompt continues.
  String? _shown;
  String? _selected;
  bool _inspecting = false;
  InspectorTab _tab = InspectorTab.summary;

  /// What the model saw, loaded when a response's inspector opens.
  late final _details = ContextDetails(widget.state.context)
    ..addListener(() => setState(() {}));
  final _folded = <int>{};
  ContextTrack _track = contextTrack(const []);

  /// Whether the context track fits the branch instead of the model's window.
  bool _fit = false;

  /// Whether the session tree is open; `null` opens it in windows at least 1280 px wide.
  bool? _treeOpen;
  bool _equalWidth = false;

  /// The range the handles limit the list to.
  (double, double)? _range;
  (double, double)? _window;

  /// What the filters let through, and whether their row shows.
  StepFilter _filter = const StepFilter();
  bool _filtering = false;
  final _filterText = TextEditingController();
  final _filterFocus = FocusNode(debugLabel: 'filter steps');

  /// What the list shows, or the range limiting it, as the overview's brush.
  final _brush = ValueNotifier<Brush?>(null);

  /// The lines the list shows, as last built.
  List<_Line> _shownLines = const [];
  bool _comparing = false;
  List<String> _picked = const [];
  String? _cursor;

  /// The history read in progress.
  Future<void>? _loading;
  bool _again = false;

  List<BridgeRecord> get _records =>
      _history?.trajectories.firstOrNull?.records ?? const [];

  @override
  void initState() {
    super.initState();
    widget.state.stored.addListener(_reload);
    widget.state.models.addListener(_modelsChanged);
    _scroll.addListener(_updateBrush);
    _reload();
    // Opening the tab moves the keyboard to it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    widget.state.stored.removeListener(_reload);
    widget.state.models.removeListener(_modelsChanged);
    _details.dispose();
    _focus.dispose();
    _filterText.dispose();
    _filterFocus.dispose();
    _brush.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// The models' context windows arrived.
  void _modelsChanged() => setState(() {});

  List<String> get _leaves {
    if (_comparing) return _picked;
    final shown = _shown;
    return shown == null ? const [] : [shown];
  }

  /// Reads the history again; changes during a read cause one more read, never a queue. The
  /// future completes once the history is current.
  Future<void> _reload() {
    if (_loading case final loading?) {
      _again = true;
      return loading;
    }
    return _loading = _read();
  }

  Future<void> _read() async {
    // `_loading` is cleared in the same synchronous step as the last check of `_again`, so no
    // caller waits for a read that missed its change.
    try {
      do {
        _again = false;
        final leaves = _leaves;
        final BridgeHistory history;
        try {
          history = await widget.state.history(leaves);
        } on Object catch (error) {
          // The last history stays.
          if (mounted) {
            widget.state.reportRead(
              'history',
              'Could not read the session history: $error',
            );
          }
          return;
        }
        widget.state.reportRead('history', null);
        if (!mounted) return;
        final graph = sessionGraph(history.tree, history.leaf);
        String grown(String tip) => graph.branchOf(tip)?.tip ?? tip;
        setState(() {
          _history = history;
          _graph = graph;
          _track = contextTrack(
            history.trajectories.firstOrNull?.records ?? const [],
          );
          if (_shown case final shown?) _shown = grown(shown);
          _picked = [for (final tip in _picked) grown(tip)];
        });
        // A shown tip that grew is read again up to its new tip.
        if (!listEquals(_leaves, leaves)) _again = true;
      } while (_again && mounted);
    } finally {
      _loading = null;
    }
  }

  Future<void> _show(String? tip) {
    setState(() {
      _shown = tip;
      _range = null;
      _window = null;
    });
    return _reload();
  }

  /// The tip of the branch the ledger shows, if it is a tip.
  String? get _shownTip => _shown ?? _history?.leaf;

  Branch? get _shownBranch =>
      _graph.branches.where((branch) => branch.tip == _shownTip).firstOrNull;

  /// Whether step `index` lies in the range the handles limit the list to.
  bool _inRange(int index) {
    final range = _range;
    if (range == null) return true;
    final (start, end) = stepExtent(_records, index, equalWidth: _equalWidth);
    return start <= range.$2 && end >= range.$1;
  }

  /// Which steps match the filters; `null` while none is on.
  List<bool>? _matches() => _filter.count == 0
      ? null
      : [for (final record in _records) _filter.matches(record)];

  /// The steps ↑ and ↓ move between: shown ones, and only matches while filtering.
  List<int> _visibleSteps() {
    final matches = _matches();
    return [
      for (final turn in turnGroups(_records))
        if (!_folded.contains(turn.turn))
          for (var index = turn.first; index <= turn.last; index++)
            if (_inRange(index) && (matches?[index] ?? true)) index,
    ];
  }

  /// Marks the steps the list shows on the overview, or the range limiting it.
  void _updateBrush() {
    final records = _records;
    final range = _range;
    if (range != null) {
      final inside = [
        for (var index = 0; index < records.length; index++)
          if (_inRange(index)) index,
      ];
      _brush.value = Brush(
        range.$1,
        range.$2,
        inside.isEmpty
            ? 'Range: no steps'
            : 'Range: steps ${inside.first + 1}–${inside.last + 1}',
        range: true,
      );
      return;
    }
    if (!_scroll.hasClients) {
      _brush.value = null;
      return;
    }
    final position = _scroll.position;
    final top = position.pixels;
    final bottom = top + position.viewportDimension - _clearance;
    int? first;
    int? last;
    var y = 0.0;
    for (final line in _shownLines) {
      final height = line is _InspectorLine
          ? inspectorHeight
          : WsMetrics.rowTable;
      if (line is _StepLine && y + height > top && y < bottom) {
        first ??= line.index;
        last = line.index;
      }
      y += height;
      if (y >= bottom) break;
    }
    if (first == null || last == null) {
      _brush.value = null;
      return;
    }
    _brush.value = Brush(
      stepExtent(records, first, equalWidth: _equalWidth).$1,
      stepExtent(records, last, equalWidth: _equalWidth).$2,
      'Showing steps ${first + 1}–${last + 1}',
    );
  }

  /// The top of line `line` in the list.
  double _lineTop(int line) {
    var y = 0.0;
    for (final shown in _shownLines.take(line)) {
      y += shown is _InspectorLine ? inspectorHeight : WsMetrics.rowTable;
    }
    return y;
  }

  /// Scrolls the list so the step at `at` on the axis comes first, as dragging the brush asks.
  void _scrollTo(double at) {
    final index = stepAt(_records, at, equalWidth: _equalWidth);
    if (index == null || !_scroll.hasClients) return;
    final line = _shownLines.indexWhere(
      (line) => line is _StepLine && line.index == index,
    );
    if (line < 0) return;
    _scroll.jumpTo(math.min(_lineTop(line), _scroll.position.maxScrollExtent));
  }

  /// Selects the step at `at` on the axis, expanding its turn and leaving a range that hides it.
  void _jumpTo(double at) {
    final index = stepAt(_records, at, equalWidth: _equalWidth);
    if (index == null) return;
    _focus.requestFocus();
    setState(() {
      if (!_inRange(index)) _range = null;
      _folded.remove(_records[index].turn);
      _selected = _records[index].entry;
    });
    _reveal();
  }

  /// Opens the filter row with its text field focused.
  void _openFilter() {
    setState(() => _filtering = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _filterFocus.requestFocus();
    });
  }

  int get _selectedIndex =>
      _records.indexWhere((record) => record.entry == _selected);

  void _move(int step) {
    final visible = _visibleSteps();
    if (visible.isEmpty) return;
    final current = _selectedIndex;
    final int target;
    if (current < 0) {
      target = step > 0 ? visible.first : visible.last;
    } else if (step > 0) {
      target = visible.firstWhere(
        (i) => i > current,
        orElse: () => visible.last,
      );
    } else {
      target = visible.lastWhere(
        (i) => i < current,
        orElse: () => visible.first,
      );
    }
    setState(() => _selected = _records[target].entry);
    _reveal();
  }

  /// Scrolls the selected step and its open inspector into view without animation; when both do
  /// not fit, the step's row stays at the top, so the inspector's tabs show.
  void _reveal() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) return;
      final line = _lines().indexWhere(
        (line) => line is _StepLine && line.index == _selectedIndex,
      );
      if (line < 0) return;
      final top = line * WsMetrics.rowTable;
      final bottom =
          top + WsMetrics.rowTable + (_inspecting ? inspectorHeight : 0);
      final position = _scroll.position;
      // The floating input area covers the bottom of the viewport.
      final visible = position.viewportDimension - _clearance;
      if (top < position.pixels) {
        _scroll.jumpTo(top);
      } else if (bottom > position.pixels + visible) {
        _scroll.jumpTo(
          math.min(math.min(bottom - visible, top), position.maxScrollExtent),
        );
      }
    });
  }

  void _fold(bool fold) {
    final index = _selectedIndex;
    if (index < 0) return;
    final turn = _records[index].turn;
    setState(() => fold ? _folded.add(turn) : _folded.remove(turn));
  }

  /// Collapses or expands one turn; with ⌥ held, every turn.
  void _toggleTurn(int turn) {
    if (HardwareKeyboard.instance.isAltPressed) {
      _foldAll();
      return;
    }
    setState(() => _folded.remove(turn) || _folded.add(turn));
  }

  bool get _allFolded {
    final turns = {for (final record in _records) record.turn};
    return turns.isNotEmpty && _folded.containsAll(turns);
  }

  void _foldAll() {
    final turns = {for (final record in _records) record.turn};
    setState(() {
      if (_folded.containsAll(turns)) {
        _folded.clear();
      } else {
        _folded.addAll(turns);
      }
    });
  }

  /// Room below the last row for the floating input area.
  double get _clearance => widget.input == null ? 0 : inputClearance;

  bool get _busy => switch (widget.state.status.value) {
    RunStatus.running || RunStatus.waitingForYou => true,
    _ => false,
  };

  /// Continues the session before the selected step, so the step and everything after it can
  /// go differently on a new branch. A prompt branches at itself. pi continues before a
  /// prompt and hands it back to the input area for editing, so a response right after its
  /// prompt is redone from there too.
  Future<void> _branch() async {
    final index = _selectedIndex;
    if (index < 0 || _comparing) return;
    if (_busy) {
      widget.state.notify(BridgeLevel.info, 'Branch once the agent is idle.');
      return;
    }
    final at = _records[index].kind == BridgeRecordKind.user
        ? index
        : index - 1;
    if (at < 0) return;
    final from = _records[at];
    // At a prompt the agent continues before it; elsewhere at the entry itself.
    final expected = from.kind == BridgeRecordKind.user
        ? _history?.tree
              .where((node) => node.id == from.entry)
              .firstOrNull
              ?.parent
        : from.entry;
    try {
      // A branch not made said why or waits for one already running.
      if (!await widget.state.branch(from.entry)) return;
    } on Object catch (error) {
      widget.state.notify(
        BridgeLevel.error,
        'The session could not branch at step ${index + 1}: $error',
      );
      return;
    }
    bool moved;
    try {
      moved = (await widget.state.history()).leaf == expected;
    } on Object {
      moved = false;
    }
    if (!mounted) return;
    if (!moved) {
      widget.state.notify(
        BridgeLevel.error,
        'The session could not branch at step ${index + 1}.',
      );
      return;
    }
    setState(() {
      _inspecting = false;
      _selected = null;
    });
    _show(null);
    widget.onBranched();
  }

  void _toggleCompare() {
    if (_comparing) {
      widget.comparing.value = false;
      setState(() => _comparing = false);
      _reload();
      return;
    }
    final branches = _graph.branches;
    final first = _graph.branchOf(_shownTip);
    final other = branches.where((branch) => branch != first).lastOrNull;
    if (first == null || other == null) {
      widget.state.notify(BridgeLevel.info, 'Comparing needs a second branch.');
      return;
    }
    widget.comparing.value = true;
    setState(() {
      _comparing = true;
      _picked = _ordered([first.tip, other.tip]);
      _cursor = other.tip;
      _range = null;
      _window = null;
    });
    _reload();
  }

  /// Tips in tree order, so A stays on the left.
  List<String> _ordered(List<String> tips) => [
    for (final branch in _graph.branches)
      if (tips.contains(branch.tip)) branch.tip,
  ];

  void _pick(String tip) {
    if (_picked.contains(tip)) return;
    setState(() => _picked = _ordered([_picked.last, tip]));
    _reload();
  }

  /// Moves through the session tree's steps; while comparing, only the tree's cursor moves.
  void _stepTree(int step) {
    final rows = _graph.rows;
    if (rows.isEmpty) return;
    final current = rows.indexWhere(
      (row) => row.node.id == (_comparing ? _cursor : _selected),
    );
    final next = current < 0
        ? (step > 0 ? 0 : rows.length - 1)
        : (current + step).clamp(0, rows.length - 1);
    if (_comparing) {
      setState(() => _cursor = rows[next].node.id);
    } else {
      _jump(rows[next]);
    }
  }

  /// Selects a tree row's step, expanding its turn and showing its branch when the ledger
  /// shows another one.
  void _jump(GraphRow row) {
    setState(() {
      _selected = row.node.id;
      for (final record in _records) {
        if (record.entry == row.node.id) _folded.remove(record.turn);
      }
    });
    if (_records.any((record) => record.entry == row.node.id)) {
      _reveal();
      return;
    }
    final tip = row.branch?.tip ?? _graph.branchOf(row.node.id)?.tip;
    if (tip != null) _show(tip).then((_) => _reveal());
  }

  /// Compares the branch of the tree's cursor with the other picked branch.
  void _pickCursor() {
    final tip = _graph.branchOf(_cursor)?.tip;
    if (tip != null) _pick(tip);
  }

  void _onRow(GraphRow row) {
    _focus.requestFocus();
    if (_comparing) {
      setState(() => _cursor = row.node.id);
      _pickCursor();
    } else {
      _jump(row);
    }
  }

  void _escape() {
    if (_range != null || _window != null) {
      setState(() {
        _range = null;
        _window = null;
      });
    } else if (_inspecting) {
      setState(() => _inspecting = false);
    }
  }

  List<_Line> _lines() {
    final lines = <_Line>[
      if (_range == null && _records.any((record) => record.context != null))
        _StartLine(),
    ];
    for (final turn in turnGroups(_records)) {
      // A range shows only its steps and their turns.
      final steps = [
        for (var index = turn.first; index <= turn.last; index++)
          if (_inRange(index)) index,
      ];
      if (steps.isEmpty) continue;
      lines.add(_TurnLine(turn));
      if (_folded.contains(turn.turn)) continue;
      for (final index in steps) {
        lines.add(_StepLine(index));
        if (_inspecting && _records[index].entry == _selected) {
          lines.add(_InspectorLine(index));
        }
      }
    }
    return lines;
  }

  @override
  Widget build(BuildContext context) {
    final history = _history;
    final comparing =
        _comparing && history != null && history.trajectories.length > 1;
    final keys = Keymap.of(context);
    final records = _records;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The toolbar and the filter row take their own keys, so typing a filter never
        // triggers the list's.
        _Toolbar(
          equalWidth: _equalWidth,
          comparing: _comparing,
          summary: comparing ? _compareSummary(history) : _summary(),
          range: comparing || _range == null ? null : _axisLabel(_range!),
          onClearRange: () => setState(() => _range = null),
          filters: _filter.count,
          onFilter: () =>
              _filtering ? setState(() => _filtering = false) : _openFilter(),
          onEqualWidth: (value) => setState(() {
            _equalWidth = value;
            _range = null;
            _window = null;
          }),
          onCompare: _toggleCompare,
        ),
        if (_filtering && !comparing)
          FilterBar(
            filter: _filter,
            tools: [
              ...{
                for (final record in records)
                  if (record.kind == BridgeRecordKind.tool) record.label,
              },
            ],
            text: _filterText,
            textFocus: _filterFocus,
            onChanged: (filter) => setState(() => _filter = filter),
          ),
        Expanded(
          child: CallbackShortcuts(
            bindings: {
              ...keys.bind({
                Command.foldAll: _foldAll,
                Command.openStep: () {
                  if (_selected == null) return;
                  setState(() => _inspecting = !_inspecting);
                  _reveal();
                },
                Command.branch: _branch,
                Command.compare: _toggleCompare,
                Command.selectBranch: () {
                  if (_comparing) _pickCursor();
                },
                Command.closeStep: _escape,
                Command.filterSteps: _openFilter,
              }),
              ...Fixed.previousOrNextStep.bind([
                () => _move(-1),
                () => _move(1),
              ]),
              ...Fixed.foldTurn.bind([() => _fold(true), () => _fold(false)]),
              ...Fixed.moveInTree.bind([
                () => _stepTree(-1),
                () => _stepTree(1),
              ]),
            },
            child: Focus(
              focusNode: _focus,
              autofocus: true,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (comparing) _compareOverview(history) else _overview(),
                  Expanded(
                    child: LayoutBuilder(
                      builder: (context, constraints) => Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          Expanded(
                            child: InputLane(
                              input: widget.input,
                              child: comparing
                                  ? CompareView(
                                      a: history.trajectories[0],
                                      b: history.trajectories[1],
                                      comparison: _comparison(history),
                                      branchA: _branchFor(_picked[0]),
                                      branchB: _branchFor(_picked[1]),
                                      bottom: _clearance,
                                    )
                                  : _ledger(),
                            ),
                          ),
                          SessionTreeView(
                            graph: _graph,
                            selected: _comparing ? _cursor : _selected,
                            onRow: _onRow,
                            shown: _comparing ? _picked.toSet() : {?_shownTip},
                            picked: _comparing ? _picked.toSet() : null,
                            open:
                                _treeOpen ??
                                constraints.maxWidth >= _treeOpenWidth,
                            onOpen: (open) => setState(() => _treeOpen = open),
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }

  Branch _branchFor(String tip) => _graph.branches.firstWhere(
    (branch) => branch.tip == tip,
    orElse: () => Branch(tip: tip, ref: '?', name: '', lane: 0),
  );

  /// The shown branch, its steps, its duration and how many steps match the filters.
  String _summary() {
    final branch = _shownBranch;
    final matches = _matches();
    final matching = matches?.where((match) => match).length;
    return [
      branch == null ? 'new branch' : 'branch ${branch.ref}',
      '${_records.length} steps',
      formatDuration(_history?.trajectories.firstOrNull?.durationMs ?? 0),
      if (matching != null)
        '$matching of ${_records.length} match${matching == 1 ? 'es' : ''}',
    ].join(' · ');
  }

  Widget _overview() {
    final records = _records;
    final selected = _selectedIndex;
    final steps = segments(records, equalWidth: _equalWidth);
    return Overview(
      tracks: [
        for (final (label, lane) in [
          ('Model', OverviewLane.model),
          ('Tools', OverviewLane.tools),
          ('You', OverviewLane.you),
        ])
          OverviewTrack(label, [
            for (final segment in steps)
              if (segment.lane == lane) segment,
          ]),
      ],
      span: span(records, equalWidth: _equalWidth),
      equalWidth: _equalWidth,
      window: _window,
      onWindow: (window) => setState(() => _window = window),
      brush: _brush,
      onRange: (range) => setState(() => _range = range),
      onPan: _scrollTo,
      onJump: _jumpTo,
      matches: _matches(),
      mark: selected < 0
          ? null
          : _equalWidth
          ? selected.toDouble()
          : records[selected].startMs.toDouble(),
    );
  }

  /// The comparison of the shown history, computed once per history.
  final _comparisons = Expando<Comparison>();

  Comparison _comparison(BridgeHistory history) =>
      _comparisons[history] ??= compareBranches(
        history.trajectories[0].records,
        history.trajectories[1].records,
      );

  /// The compared branches and where they part.
  String _compareSummary(BridgeHistory history) {
    final shared = _comparison(history).shared;
    return [
      '${_branchFor(_picked[0]).ref} ↔ ${_branchFor(_picked[1]).ref} on one time axis',
      if (shared > 0) '◆ branch point after step $shared',
    ].join(' · ');
  }

  Widget _compareOverview(BridgeHistory history) {
    final a = history.trajectories[0].records;
    final b = history.trajectories[1].records;
    final comparison = _comparison(history);
    final shared = comparison.shared;
    final branchPoint = shared == 0
        ? null
        : _equalWidth
        ? shared.toDouble()
        : (a[shared - 1].startMs + (a[shared - 1].durationMs ?? 0)).toDouble();
    // Each branch's own steps start at the branch point, without the idle time before them.
    List<Segment> own(List<BridgeRecord> records, int idle) => [
      for (final segment in segments(records, equalWidth: _equalWidth))
        if (_equalWidth || segment.index < shared || idle == 0)
          segment
        else
          Segment(
            segment.index,
            segment.lane,
            segment.start - idle,
            segment.end - idle,
            segment.firstToken == null ? null : segment.firstToken! - idle,
          ),
    ];
    return Overview(
      tracks: [
        OverviewTrack(_branchFor(_picked[0]).ref, own(a, comparison.idleA)),
        OverviewTrack(_branchFor(_picked[1]).ref, own(b, comparison.idleB)),
      ],
      span: _equalWidth
          ? math.max(a.length, b.length).toDouble()
          : math.max(
              span(a, equalWidth: false) - comparison.idleA,
              span(b, equalWidth: false) - comparison.idleB,
            ),
      equalWidth: _equalWidth,
      window: _window,
      onWindow: (window) => setState(() => _window = window),
      mark: branchPoint,
      legend: true,
    );
  }

  String _axisLabel((double, double) range) => _equalWidth
      ? 'steps ${range.$1.floor() + 1}–${range.$2.ceil()}'
      : '${(range.$1 / 1000).toStringAsFixed(1)}–'
            '${(range.$2 / 1000).toStringAsFixed(1)} s';

  /// The window of the latest request's model, when the agent reports it.
  int? _contextWindow() {
    final models = widget.state.models.value;
    for (final record in _records.reversed) {
      if (record.kind != BridgeRecordKind.message) continue;
      if (contextWindowOf(record.model, models) case final window?) {
        return window;
      }
    }
    return null;
  }

  Widget _ledger() {
    final records = _records;
    final lines = _shownLines = _lines();
    final matches = _matches();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _updateBrush();
    });
    final track = _track;
    final window = _contextWindow();
    final fit = _fit || window == null;
    final scale = fit
        ? track.levels.fold(track.start.tokens, (most, level) {
            return math.max(most, level.tokens);
          })
        : window;
    final compaction = fit ? null : compactionPoint(window);
    final firstContext = records
        .where((record) => record.context != null)
        .firstOrNull
        ?.context;
    // What each line's track shows; collapsed turns and the inspector break the line.
    int? valueAt(int line) => line < 0 || line >= lines.length
        ? null
        : switch (lines[line]) {
            _StartLine() => track.start.tokens,
            _TurnLine(:final turn) =>
              _folded.contains(turn.turn) ? null : track.before(turn).tokens,
            _StepLine(:final index) => track.levels[index].tokens,
            _InspectorLine() => null,
          };
    return LayoutBuilder(
      builder: (context, constraints) {
        final trackWidth = _trackWidth(constraints.maxWidth);
        Widget cell(
          int line,
          ContextLevel level, {
          bool bar = true,
          bool collapsed = false,
        }) => TrackCell(
          level: level,
          scale: math.max(scale, 1).toDouble(),
          previous: valueAt(line - 1),
          next: valueAt(line + 1),
          compaction: compaction,
          window: window,
          bar: bar,
          collapsed: collapsed,
        );
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _LedgerHeader(
              trackWidth: trackWidth,
              allFolded: _allFolded,
              onFoldAll: _foldAll,
              fit: window == null ? null : _fit,
              onFit: (value) => setState(() => _fit = value),
            ),
            Expanded(
              child: ListView.builder(
                controller: _scroll,
                padding: EdgeInsets.only(bottom: _clearance),
                itemCount: lines.length,
                itemBuilder: (context, index) => switch (lines[index]) {
                  _StartLine() => _LineRow(
                    key: const ValueKey('start'),
                    title: 'Start',
                    summary: [
                      'system prompt',
                      if (firstContext?.sections.contains('project_context') ??
                          false)
                        'AGENTS.md',
                      '${firstContext?.tools.length ?? 0} tools',
                      '≈${formatCompact(track.start.tokens)}',
                    ].join(' · '),
                    trackWidth: trackWidth,
                    track: cell(index, track.start, bar: false),
                  ),
                  _TurnLine(:final turn) => _TurnRow(
                    turn: turn,
                    before: track.before(turn),
                    after: track.levels[turn.last],
                    folded: _folded.contains(turn.turn),
                    trackWidth: trackWidth,
                    track: _folded.contains(turn.turn)
                        ? cell(index, track.levels[turn.last], collapsed: true)
                        : cell(index, track.before(turn), bar: false),
                    onTap: () => _toggleTurn(turn.turn),
                  ),
                  _StepLine(index: final step) => Opacity(
                    opacity: matches?[step] ?? true ? 1 : 0.35,
                    child: _StepRow(
                      key: ValueKey('step-${records[step].entry}'),
                      number: step + 1,
                      record: records[step],
                      request: records[step].kind == BridgeRecordKind.message
                          ? requestNumber(records, step)
                          : null,
                      level: track.levels[step],
                      trackWidth: trackWidth,
                      track: cell(
                        index,
                        track.levels[step],
                        bar: records[step].kind != BridgeRecordKind.approval,
                      ),
                      selected: records[step].entry == _selected,
                      onTap: () {
                        _focus.requestFocus();
                        final entry = records[step].entry;
                        setState(() {
                          _inspecting = entry == _selected
                              ? !_inspecting
                              : false;
                          _selected = entry;
                        });
                      },
                    ),
                  ),
                  _InspectorLine(index: final step) => _inspector(step, window),
                },
              ),
            ),
          ],
        );
      },
    );
  }

  /// The inspector of `step`, with what its request held.
  Widget _inspector(int step, int? window) {
    final records = _records;
    final request = requestOf(records, step);
    return Inspector(
      detail: request == null ? null : _details.detailFor(records[request]),
      unread: request == null ? null : _details.unread(records[request].entry),
      records: records,
      index: step,
      window: window,
      tab: _tab,
      canBranch: !_busy,
      onTab: (tab) => setState(() => _tab = tab),
      onBranch: _branch,
    );
  }
}

sealed class _Line {}

/// What the first request held before any message.
class _StartLine extends _Line {}

class _TurnLine extends _Line {
  _TurnLine(this.turn);

  final TurnGroup turn;
}

class _StepLine extends _Line {
  _StepLine(this.index);

  final int index;
}

class _InspectorLine extends _Line {
  _InspectorLine(this.index);

  final int index;
}

/// One row (approved look mockup): the view switch on the left, the summary and Compare on the
/// right.
class _Toolbar extends StatelessWidget {
  const _Toolbar({
    required this.equalWidth,
    required this.comparing,
    required this.summary,
    required this.range,
    required this.onClearRange,
    required this.filters,
    required this.onFilter,
    required this.onEqualWidth,
    required this.onCompare,
  });

  final bool equalWidth;
  final bool comparing;
  final String summary;

  /// The range limiting the list, while there is one.
  final String? range;
  final VoidCallback onClearRange;

  /// How many filters are on.
  final int filters;
  final VoidCallback onFilter;
  final ValueChanged<bool> onEqualWidth;
  final VoidCallback onCompare;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final keys = Keymap.of(context);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Container(
      height: 40,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      child: Row(
        children: [
          Segments(
            options: const [(false, 'Real time'), (true, 'Equal width')],
            selected: equalWidth,
            onSelect: onEqualWidth,
          ),
          if (!comparing) ...[
            const SizedBox(width: WsMetrics.space3),
            // Return and space press it rather than the view's own keys.
            Shortcuts(
              shortcuts: activateKeys,
              child: GhostButton(
                icon: LucideIcons.listFilter,
                label: filters == 0 ? 'Filter' : 'Filter · $filters',
                shortcut: keys.label(Command.filterSteps),
                onPressed: onFilter,
              ),
            ),
          ],
          Expanded(
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (range case final range?) ...[
                  Container(
                    key: const Key('range'),
                    padding: const EdgeInsets.only(left: WsMetrics.space3),
                    decoration: BoxDecoration(
                      color: ws.accentSubtle,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      children: [
                        Text(
                          'Range $range',
                          style: small.copyWith(color: ws.accent),
                        ),
                        Tooltip(
                          message: 'Clear the range',
                          child: InkWell(
                            onTap: onClearRange,
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                            child: Padding(
                              padding: const EdgeInsets.all(WsMetrics.space2),
                              child: Icon(
                                LucideIcons.x,
                                size: 12,
                                color: ws.accent,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space4),
                ],
                Flexible(
                  child: Text(
                    summary,
                    style: small,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space4),
          if (comparing)
            // Clicking it, return or space leave the comparison, as its key does.
            Shortcuts(
              shortcuts: activateKeys,
              child: InkWell(
                onTap: onCompare,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                child: Container(
                  height: 28,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                  ),
                  decoration: BoxDecoration(
                    color: ws.accentSubtle,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: Row(
                    children: [
                      Text(
                        'Comparing',
                        style: TextStyle(
                          color: ws.accent,
                          fontSize: WsMetrics.fsCompact,
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      Hint(keys.label(Command.compare)),
                    ],
                  ),
                ),
              ),
            )
          else
            SecondaryButton(
              icon: LucideIcons.arrowLeftRight,
              label: 'Compare',
              shortcut: keys.label(Command.compare),
              onPressed: onCompare,
            ),
        ],
      ),
    );
  }
}

/// Widths of the ledger's fixed columns: number, kind, time, adds, tokens in and out.
const _fixed = (
  number: 28.0,
  kind: 72.0,
  time: 56.0,
  adds: 56.0,
  tokens: 48.0,
  out: 44.0,
);

/// The context track's width in a ledger `width` wide: up to 180, and none when the step
/// would keep less than 160.
double _trackWidth(double width) {
  final rest =
      width -
      2 * WsMetrics.space5 -
      _fixed.number -
      _fixed.kind -
      _fixed.time -
      _fixed.adds -
      _fixed.tokens -
      _fixed.out -
      7 * WsMetrics.space3 -
      160;
  return rest < 120 ? 0 : math.min(rest, 180);
}

/// The ledger's columns: number, kind, step, time, adds, context track, tokens in and out;
/// a `span` widget takes kind through adds, as turn lines do.
Widget _columns(
  List<Widget> cells, {
  required double trackWidth,
  Widget? span,
}) {
  Widget fixed(double width, Widget child, {bool right = true}) => SizedBox(
    width: width,
    child: Align(
      alignment: right ? Alignment.centerRight : Alignment.centerLeft,
      child: child,
    ),
  );
  const gap = SizedBox(width: WsMetrics.space3);
  return Row(
    children: [
      SizedBox(width: _fixed.number, child: cells[0]),
      gap,
      if (span != null)
        Expanded(child: span)
      else ...[
        fixed(_fixed.kind, cells[1], right: false),
        gap,
        Expanded(child: cells[2]),
        gap,
        fixed(_fixed.time, cells[3]),
        gap,
        fixed(_fixed.adds, cells[4]),
      ],
      if (trackWidth > 0) ...[
        gap,
        SizedBox(
          width: trackWidth,
          height: WsMetrics.rowTable,
          child: cells[5],
        ),
      ],
      gap,
      fixed(_fixed.tokens, cells[6]),
      gap,
      fixed(_fixed.out, cells[7]),
    ],
  );
}

class _LedgerHeader extends StatelessWidget {
  const _LedgerHeader({
    required this.trackWidth,
    required this.allFolded,
    required this.onFoldAll,
    required this.fit,
    required this.onFit,
  });

  final double trackWidth;
  final bool allFolded;
  final VoidCallback onFoldAll;

  /// Whether the track fits the branch; `null` without a known window, which it then fits.
  final bool? fit;
  final ValueChanged<bool> onFit;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final fit = this.fit;
    return Container(
      height: 34,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: _columns([
        // In the column of the turns' chevrons (approved look mockup); return and space
        // press it rather than the view's own keys.
        Center(
          child: Tooltip(
            message: allFolded ? 'Expand all turns' : 'Collapse all turns',
            child: Shortcuts(
              shortcuts: activateKeys,
              child: InkWell(
                onTap: onFoldAll,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                child: SizedBox(
                  width: 24,
                  height: 24,
                  child: Icon(
                    allFolded
                        ? LucideIcons.chevronsUpDown
                        : LucideIcons.chevronsDownUp,
                    size: 14,
                    color: ws.textMuted,
                  ),
                ),
              ),
            ),
          ),
        ),
        for (final label in ['Kind', 'Step', 'Time', 'Adds'])
          Text(label, style: style),
        Row(
          children: [
            Flexible(
              child: Text(
                'Context',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: style,
              ),
            ),
            if (fit != null) ...[
              const SizedBox(width: WsMetrics.space2),
              Segments(
                options: const [(false, 'Window'), (true, 'Fit')],
                selected: fit,
                onSelect: onFit,
                dense: true,
              ),
            ],
          ],
        ),
        Text('In', style: style),
        Text('Out', style: style),
      ], trackWidth: trackWidth),
    );
  }
}

/// A line with a summary on it, such as a turn's: a title, the summary and a rule up to the
/// context track.
class _LineRow extends StatelessWidget {
  const _LineRow({
    super.key,
    required this.title,
    required this.summary,
    required this.trackWidth,
    required this.track,
    this.lead = const SizedBox.shrink(),
    this.onTap,
  });

  final String title;
  final String summary;
  final double trackWidth;
  final Widget track;

  /// What the number column shows, such as a chevron.
  final Widget lead;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      canRequestFocus: false,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: _columns(
          [
            lead,
            for (var i = 1; i < 5; i++) const SizedBox.shrink(),
            track,
            const SizedBox.shrink(),
            const SizedBox.shrink(),
          ],
          trackWidth: trackWidth,
          span: Row(
            children: [
              Text(
                title,
                style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
              ),
              const SizedBox(width: WsMetrics.space3),
              // The summary keeps its width and the rule takes the rest, at least 16.
              Expanded(
                child: LayoutBuilder(
                  builder: (context, constraints) => Row(
                    children: [
                      ConstrainedBox(
                        constraints: BoxConstraints(
                          maxWidth: math.max(0, constraints.maxWidth - 24),
                        ),
                        child: Text(
                          summary,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: ws.textFaint,
                            fontSize: WsMetrics.fsSmall,
                            fontFeatures: const [FontFeature.tabularFigures()],
                          ),
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      Expanded(
                        child: Container(height: 1, color: ws.borderStrong),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// A turn's line (approved look mockup): its chevron, prompt, duration and what it added to
/// the context; a click collapses or expands it, ⌥-click every turn.
class _TurnRow extends StatelessWidget {
  const _TurnRow({
    required this.turn,
    required this.before,
    required this.after,
    required this.folded,
    required this.trackWidth,
    required this.track,
    required this.onTap,
  });

  final TurnGroup turn;

  /// The context before the turn's first step and after its last.
  final ContextLevel before;
  final ContextLevel after;
  final bool folded;
  final double trackWidth;
  final Widget track;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final added = after.tokens - before.tokens;
    return _LineRow(
      title: turn.turn == 0 ? 'Setup' : 'Turn ${turn.turn}',
      summary: [
        if (turn.prompt.isNotEmpty) turn.prompt,
        formatDuration(turn.durationMs),
        '${added < 0 ? '−' : '+'}${formatCompact(added.abs())} → '
            '${after.exact ? '' : '≈'}${formatCompact(after.tokens)}',
      ].join(' · '),
      trackWidth: trackWidth,
      track: track,
      onTap: onTap,
      lead: Center(
        child: Icon(
          folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
          key: ValueKey('chevron-${turn.turn}'),
          size: 14,
          color: context.ws.textFaint,
        ),
      ),
    );
  }
}

class _StepRow extends StatelessWidget {
  const _StepRow({
    super.key,
    required this.number,
    required this.record,
    required this.request,
    required this.level,
    required this.trackWidth,
    required this.track,
    required this.selected,
    required this.onTap,
  });

  final int number;
  final BridgeRecord record;

  /// A response's request number on the branch.
  final int? request;

  /// The context after the step.
  final ContextLevel level;
  final double trackWidth;
  final Widget track;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final figures = TextStyle(
      color: ws.textMuted,
      fontSize: WsMetrics.fsCompact,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    final usage = record.usage;
    final duration = record.durationMs;
    final request = this.request;
    return InkWell(
      canRequestFocus: false,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
        decoration: BoxDecoration(
          color: selected ? ws.selected : null,
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: _columns([
          Text('$number', style: figures.copyWith(color: ws.textFaint)),
          KindTag(record.kind),
          // The result gives way before the step's name loses more than half the column.
          LayoutBuilder(
            builder: (context, constraints) => Row(
              children: [
                Flexible(child: StepText(record)),
                if (request != null) ...[
                  const SizedBox(width: WsMetrics.space2),
                  Text(
                    '#$request',
                    style: figures.copyWith(color: ws.textFaint),
                  ),
                ],
                const SizedBox(width: WsMetrics.space3),
                ConstrainedBox(
                  constraints: BoxConstraints(
                    maxWidth: constraints.maxWidth / 2,
                  ),
                  child: OutcomeText(record.outcome),
                ),
              ],
            ),
          ),
          Text(
            record.kind == BridgeRecordKind.user
                ? '+0 s'
                : duration == null
                ? ''
                : '+${formatDuration(duration)}',
            style: figures,
          ),
          Text(
            record.kind == BridgeRecordKind.approval
                ? ''
                : '${level.addedExact ? '' : '≈'}+${formatCompact(level.added)}',
            style: figures,
          ),
          track,
          Text(
            usage == null ? '' : formatCompact(promptTokens(usage)),
            style: figures,
          ),
          Text(
            usage == null ? '' : formatCompact(usage.output),
            style: figures,
          ),
        ], trackWidth: trackWidth),
      ),
    );
  }
}
