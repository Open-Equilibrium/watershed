import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

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
import 'inspector.dart';
import 'overview.dart';
import 'step_parts.dart';
import 'trajectory_model.dart';
import 'tree_view.dart';

/// Keys of the Trajectory tab, for the status bar.
String trajectoryKeys(ShortcutSettings keys) => [
  '${Fixed.previousOrNextStep.display} step',
  '${Fixed.foldTurn.display} fold',
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

/// The Trajectory tab (approved mockups 2 and 3): one overview on a shared time axis, the
/// steps grouped by turn with an inspector, and the session tree. It reads the session's
/// stored history, so it looks the same during the run, after it and after a restart.
class TrajectoryView extends StatefulWidget {
  const TrajectoryView({
    super.key,
    required this.state,
    required this.onBranched,
    required this.comparing,
  });

  final SessionState state;

  /// Called after branching, so the input area can take the new branch's prompt.
  final VoidCallback onBranched;

  /// Whether two branches are compared, for the keys the status bar shows.
  final ValueNotifier<bool> comparing;

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
  bool _equalWidth = false;
  (double, double)? _range;
  (double, double)? _window;
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
    _reload();
    // Opening the tab moves the keyboard to it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    widget.state.stored.removeListener(_reload);
    _details.dispose();
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

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

  List<int> _visibleSteps() => [
    for (final turn in turnGroups(_records))
      if (!_folded.contains(turn.turn))
        for (var index = turn.first; index <= turn.last; index++) index,
  ];

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

  /// Scrolls the selected step and its open inspector into view without animation.
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
      if (top < position.pixels) {
        _scroll.jumpTo(top);
      } else if (bottom > position.pixels + position.viewportDimension) {
        _scroll.jumpTo(
          math.min(
            bottom - position.viewportDimension,
            position.maxScrollExtent,
          ),
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
    if (index < 0 || _busy || _comparing) return;
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
      await widget.state.branch(from.entry);
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
    if (first == null || other == null) return;
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

  /// Selects a tree row's step, showing its branch when the ledger shows another one.
  void _jump(GraphRow row) {
    setState(() => _selected = row.node.id);
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
    final lines = <_Line>[];
    for (final turn in turnGroups(_records)) {
      lines.add(_TurnLine(turn));
      if (_folded.contains(turn.turn)) continue;
      for (var index = turn.first; index <= turn.last; index++) {
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
    return CallbackShortcuts(
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
        }),
        ...Fixed.previousOrNextStep.bind([() => _move(-1), () => _move(1)]),
        ...Fixed.foldTurn.bind([() => _fold(true), () => _fold(false)]),
        ...Fixed.moveInTree.bind([() => _stepTree(-1), () => _stepTree(1)]),
      },
      child: Focus(
        focusNode: _focus,
        autofocus: true,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Toolbar(
              equalWidth: _equalWidth,
              comparing: _comparing,
              onEqualWidth: (value) => setState(() {
                _equalWidth = value;
                _range = null;
                _window = null;
              }),
              onFold: _foldAll,
              onCompare: _toggleCompare,
            ),
            if (comparing) _compareOverview(history) else _overview(),
            Expanded(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Expanded(
                    child: comparing
                        ? CompareView(
                            a: history.trajectories[0],
                            b: history.trajectories[1],
                            comparison: _comparison(history),
                            branchA: _branchFor(_picked[0]),
                            branchB: _branchFor(_picked[1]),
                          )
                        : _ledger(),
                  ),
                  SessionTreeView(
                    graph: _graph,
                    selected: _comparing ? _cursor : _selected,
                    onRow: _onRow,
                    picked: _comparing ? _picked.toSet() : null,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Branch _branchFor(String tip) => _graph.branches.firstWhere(
    (branch) => branch.tip == tip,
    orElse: () => Branch(tip: tip, ref: '?', name: '', lane: 0),
  );

  Widget _overview() {
    final records = _records;
    final branch = _shownBranch;
    final range = _range;
    final focused = range == null
        ? null
        : inRange(records, range, equalWidth: _equalWidth);
    final selected = _selectedIndex;
    final steps = segments(records, equalWidth: _equalWidth);
    return Overview(
      summary: [
        branch == null ? 'new branch' : 'branch ${branch.ref}',
        '${records.length} steps',
        formatDuration(_history?.trajectories.firstOrNull?.durationMs ?? 0),
      ].join(' · '),
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
      range: range,
      rangeLabel: range == null
          ? null
          : '${_axisLabel(range)} · ${focused!.where((on) => on).length} steps',
      onRange: (range) => setState(() => _range = range),
      onWindow: (window) => setState(() => _window = window),
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
      summary: [
        '${_branchFor(_picked[0]).ref} ↔ ${_branchFor(_picked[1]).ref} on one time axis',
        if (shared > 0) '◆ branch point after step $shared',
      ].join(' · '),
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
      range: null,
      rangeLabel: null,
      onRange: (_) {},
      onWindow: (window) => setState(() => _window = window),
      mark: branchPoint,
      legend: true,
    );
  }

  String _axisLabel((double, double) range) => _equalWidth
      ? 'steps ${range.$1.floor() + 1}–${range.$2.ceil()}'
      : '${(range.$1 / 1000).toStringAsFixed(1)}–'
            '${(range.$2 / 1000).toStringAsFixed(1)} s';

  Widget _ledger() {
    final records = _records;
    final lines = _lines();
    final range = _range;
    final focused = range == null
        ? null
        : inRange(records, range, equalWidth: _equalWidth);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const _LedgerHeader(),
        Expanded(
          child: ListView.builder(
            controller: _scroll,
            itemCount: lines.length,
            itemBuilder: (context, index) => switch (lines[index]) {
              _TurnLine(:final turn) => _TurnRow(
                turn: turn,
                folded: _folded.contains(turn.turn),
                onTap: () => setState(
                  () => _folded.contains(turn.turn)
                      ? _folded.remove(turn.turn)
                      : _folded.add(turn.turn),
                ),
              ),
              _StepLine(index: final step) => Opacity(
                opacity: focused == null || focused[step] ? 1 : 0.4,
                child: _StepRow(
                  key: ValueKey('step-${records[step].entry}'),
                  number: step + 1,
                  record: records[step],
                  selected: records[step].entry == _selected,
                  onTap: () {
                    _focus.requestFocus();
                    final entry = records[step].entry;
                    setState(() {
                      _inspecting = entry == _selected ? !_inspecting : false;
                      _selected = entry;
                    });
                  },
                ),
              ),
              _InspectorLine(index: final step) => Inspector(
                detail: _details.detailFor(records[step]),
                unread: _details.unread(records[step].entry),
                records: records,
                index: step,
                tab: _tab,
                canBranch: !_busy,
                onTab: (tab) => setState(() => _tab = tab),
                onBranch: _branch,
              ),
            },
          ),
        ),
      ],
    );
  }
}

sealed class _Line {}

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

class _Toolbar extends StatelessWidget {
  const _Toolbar({
    required this.equalWidth,
    required this.comparing,
    required this.onEqualWidth,
    required this.onFold,
    required this.onCompare,
  });

  final bool equalWidth;
  final bool comparing;
  final ValueChanged<bool> onEqualWidth;
  final VoidCallback onFold;
  final VoidCallback onCompare;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 44,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      child: Row(
        children: [
          Container(
            padding: const EdgeInsets.all(2),
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              children: [
                for (final (label, value) in [
                  ('Real time', false),
                  ('Equal width', true),
                ])
                  GestureDetector(
                    onTap: () => onEqualWidth(value),
                    child: Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 10,
                        vertical: 2,
                      ),
                      decoration: value == equalWidth
                          ? BoxDecoration(
                              color: ws.canvas,
                              borderRadius: BorderRadius.circular(
                                WsMetrics.radiusSm,
                              ),
                              boxShadow: [
                                BoxShadow(color: ws.border, spreadRadius: 1),
                              ],
                            )
                          : null,
                      child: Text(
                        label,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: value == equalWidth ? ws.text : ws.textMuted,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space2),
          TextButton(
            onPressed: onFold,
            style: TextButton.styleFrom(
              foregroundColor: ws.textMuted,
              textStyle: const TextStyle(fontSize: WsMetrics.fsCompact),
            ),
            child: Row(
              children: [
                const Text('Fold turns'),
                const SizedBox(width: WsMetrics.space3),
                Hint(Keymap.of(context).label(Command.foldAll)),
              ],
            ),
          ),
          const Spacer(),
          if (comparing)
            Container(
              height: 28,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
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
                  Hint(Keymap.of(context).label(Command.compare)),
                ],
              ),
            )
          else
            SecondaryButton(
              label: 'Compare',
              shortcut: Keymap.of(context).label(Command.compare),
              onPressed: onCompare,
            ),
        ],
      ),
    );
  }
}

/// The ledger's columns: number, kind, step, time, tokens in, out and thinking.
Widget _columns(List<Widget> cells) => Row(
  children: [
    SizedBox(width: 28, child: cells[0]),
    const SizedBox(width: WsMetrics.space3),
    SizedBox(
      width: 84,
      child: Align(alignment: Alignment.centerLeft, child: cells[1]),
    ),
    const SizedBox(width: WsMetrics.space3),
    Expanded(child: cells[2]),
    for (final (index, width) in [
      (3, 64.0),
      (4, 60.0),
      (5, 50.0),
      (6, 50.0),
    ]) ...[
      const SizedBox(width: WsMetrics.space3),
      SizedBox(
        width: width,
        child: Align(alignment: Alignment.centerRight, child: cells[index]),
      ),
    ],
  ],
);

class _LedgerHeader extends StatelessWidget {
  const _LedgerHeader();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Container(
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: _columns([
        for (final label in ['#', 'Kind', 'Step', 'Time', 'In', 'Out', 'Think'])
          Text(label, style: style),
      ]),
    );
  }
}

class _TurnRow extends StatelessWidget {
  const _TurnRow({
    required this.turn,
    required this.folded,
    required this.onTap,
  });

  final TurnGroup turn;
  final bool folded;
  final VoidCallback onTap;

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
        child: Row(
          children: [
            SizedBox(
              width: 14,
              child: Icon(
                folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
                size: 14,
                color: ws.textFaint,
              ),
            ),
            const SizedBox(width: WsMetrics.space2),
            Text(
              turn.turn == 0 ? 'Setup' : 'Turn ${turn.turn}',
              style: const TextStyle(fontWeight: WsMetrics.fwSemibold),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                turn.prompt,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(color: ws.textMuted),
              ),
            ),
            const SizedBox(width: WsMetrics.space4),
            Text(
              [
                formatDuration(turn.durationMs),
                for (final tool in turn.tools.entries)
                  '${tool.key} ${tool.value}',
              ].join(' · '),
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ],
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
    required this.selected,
    required this.onTap,
  });

  final int number;
  final BridgeRecord record;
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
          Row(
            children: [
              Flexible(child: StepText(record)),
              const SizedBox(width: WsMetrics.space3),
              OutcomeText(record.outcome),
            ],
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
            usage == null ? '' : formatCount(promptTokens(usage)),
            style: figures,
          ),
          Text(usage == null ? '' : formatCount(usage.output), style: figures),
          Text(
            usage == null ? '' : formatCount(usage.reasoning),
            style: figures,
          ),
        ]),
      ),
    );
  }
}
