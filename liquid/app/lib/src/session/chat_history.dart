import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';
import '../rust/api/history.dart';
import 'backend.dart';
import 'chat_items.dart';
import 'session_graph.dart';

/// A session's chat as it stores it: the current branch, read back page by page from the
/// newest, with a mark where the branch leaves the others.
class ChatHistory {
  ChatHistory(
    this._backend, {
    required this.items,
    required this.totals,
    required this.notify,
    required this.reportRead,
  });

  final SessionBackend _backend;

  /// The chat; restored items go before those that arrive live.
  final ValueNotifier<List<ItemView>> items;

  /// The session's totals, which start with what the stored responses used.
  final ValueNotifier<SessionTotals> totals;
  final void Function(BridgeLevel level, String text) notify;
  final void Function(String what, String? problem) reportRead;

  /// Whether earlier stored entries of the chat remain to be read.
  final more = ValueNotifier(false);

  /// The earliest stored entry the chat covers; the next page ends before it.
  String? _start;

  /// The version whose earlier page is being read; a reset or restore reads for its own.
  int? _readingVersion;

  /// Rebuilt items count down from -1, so they never meet live item ids.
  int _nextId = -1;

  /// Forgets the previous session's chat; reads still running for it are dropped.
  void reset() {
    _version++;
    _start = null;
    _pendingMark = null;
    _ended.clear();
    _forget();
    more.value = false;
  }

  /// Rebuilds the chat from the session's current branch, as the model sees it, and marks
  /// where it leaves the other branches. Items that arrive meanwhile stay after it.
  Future<void> load() {
    final load = _load();
    loading = load;
    return load.whenComplete(() {
      if (identical(loading, load)) loading = null;
    });
  }

  /// A restore in progress; messages wait for it.
  Future<void>? loading;

  Future<void> _load() async {
    final version = ++_version;
    final before = items.value;
    final BridgeTranscript page;
    final BridgeHistory history;
    final List<BridgeTurn> turns;
    try {
      page = await _backend.transcript(null);
      history = await _backend.history();
      turns = await _backend.turns();
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not restore the chat: $error');
      return;
    }
    if (version != _version) return;
    // Earlier pages started during this restore read the cursor it replaces, so they are dropped.
    _version++;
    // What the stored responses used; live requests add to it.
    totals.value = const SessionTotals().add(history.usage);
    final rebuilt = _stored(page.items);
    _pendingMark = null;
    _path = _pathIndex(history.tree, history.leaf);
    _turns = turns;
    // Turns whose end the restore read get their rows from it, not from a later end event.
    _ended
      ..clear()
      ..addAll([
        for (final turn in turns)
          if (turn.endEntry != null) turn.id,
      ]);
    _forget();
    if (branchMarker(history.tree, history.leaf) case final marker?) {
      final mark = (
        marker: marker,
        path: _path,
        fork: _path[marker.fork] ?? -1,
      );
      if (!_placeMark(rebuilt, mark, complete: !page.more)) _pendingMark = mark;
    }
    _placeTurnEnds(rebuilt);
    // Only live items arrived meanwhile; items another restore rebuilt are replaced.
    final arrived = items.value.where(
      (view) =>
          !before.contains(view) &&
          view.entry == null &&
          view.marker == null &&
          view.turnEnd == null,
    );
    items.value = [...rebuilt, ...arrived];
    _start = page.start;
    more.value = page.more;
    if (page.items.isEmpty) await loadEarlier();
  }

  /// The branch mark until the chat shows where the branch leaves the others.
  _Mark? _pendingMark;

  /// Each entry's position on the current branch, and the branch's turns, as last restored.
  Map<String, int> _path = const {};
  List<BridgeTurn> _turns = const [];

  /// Puts a row after each restored turn that changed files, below the turn's last item `views`
  /// show; rows of turns that ended live stay where they are. Returns `views`.
  List<ItemView> _placeTurnEnds(List<ItemView> views) {
    int? at(String? entry) => entry == null ? null : _path[entry];
    final placeable = {
      for (final turn in _turns)
        if (at(turn.startEntry) != null && at(turn.endEntry) != null) turn.id,
    };
    views.removeWhere((view) => placeable.contains(view.turnEnd?.id));
    final places = <(int, BridgeTurn)>[];
    for (final turn in _turns) {
      final (start, end) = (at(turn.startEntry), at(turn.endEntry));
      if (start == null || end == null || !_changedFiles(turn)) continue;
      final last = views.lastIndexWhere((view) {
        final position = at(view.entry) ?? -1;
        return position > start && position < end;
      });
      if (last >= 0) places.add((last + 1, turn));
    }
    for (final (at, turn) in places.reversed) {
      views.insert(at, _turnEnd(turn));
    }
    return views;
  }

  /// Turns whose row was placed, live or by a restore, so a later end event places none twice.
  final _ended = <String>{};

  /// Where the chat stood when each turn with a checkpoint ended live, until its row is placed
  /// or a read that began after its end shows no such turn.
  final _boundaries = <_Boundary>[];

  /// How many turns have ended live; never reset, so each turn end has its own number.
  int _completed = 0;

  /// Gives up the places of the turns that ended live so far.
  void _forget() => _boundaries.clear();

  /// A turn ended live: its row goes here, after its items and before any of the next turn.
  /// `checkpoint` is the turn's checkpoint id, absent when it took none, so it has no place.
  /// Returns how many turns ended live so far, for [endTurns].
  int turnCompleted(String? checkpoint) {
    final ordinal = ++_completed;
    if (checkpoint != null && !_ended.contains(checkpoint)) {
      _boundaries.add((
        ordinal: ordinal,
        turn: checkpoint,
        at: items.value.length,
      ));
    }
    return ordinal;
  }

  /// Places the rows of turns that ended live, each where its turn ended. `turns` were read
  /// after `read` turns had ended live; 0 if that read failed.
  void endTurns(List<BridgeTurn> turns, int read) {
    final views = [...items.value];
    var inserted = 0;
    for (final turn in turns) {
      if (_ended.contains(turn.id)) continue;
      // A turn whose end is not recorded yet keeps its place for a later read.
      final index = _boundaries.indexWhere(
        (boundary) => boundary.turn == turn.id,
      );
      if (index < 0 || turn.endEntry == null) continue;
      final at = _boundaries.removeAt(index).at;
      _ended.add(turn.id);
      if (!_changedFiles(turn)) continue;
      views.insert(at.clamp(0, views.length), _turnEnd(turn));
      inserted++;
      // The row moved the remaining places.
      _shift(1);
    }
    // A turn that ended before the read and is not among its turns took no checkpoint, or its
    // checkpoint failed: its place would otherwise go to a later turn.
    final shown = {for (final turn in turns) turn.id};
    _boundaries.removeWhere(
      (boundary) => boundary.ordinal <= read && !shown.contains(boundary.turn),
    );
    if (inserted > 0) items.value = views;
  }

  /// Moves the places of live turns' rows after `count` items were added before them.
  void _shift(int count) {
    for (var index = 0; index < _boundaries.length; index++) {
      final boundary = _boundaries[index];
      _boundaries[index] = (
        ordinal: boundary.ordinal,
        turn: boundary.turn,
        at: boundary.at + count,
      );
    }
  }

  static bool _changedFiles(BridgeTurn turn) =>
      turn.ended && (turn.files ?? 0) > 0;

  ItemView _turnEnd(BridgeTurn turn) => ItemView(
    id: _nextId--,
    kind: BridgeItemKind.dynamicToolCall,
    turnEnd: turn,
  );

  /// Counts restores and session changes; reads for a replaced chat are dropped.
  int _version = 0;

  /// Puts the mark before the first of `views` past the fork, once they show the fork itself
  /// or nothing earlier remains; whether it did.
  bool _placeMark(List<ItemView> views, _Mark mark, {required bool complete}) {
    int position(ItemView view) => mark.path[view.entry] ?? -1;
    final reachesFork = views.any(
      (view) => view.entry != null && position(view) <= mark.fork,
    );
    if (!reachesFork && !complete) return false;
    final after = views.indexWhere((view) => position(view) > mark.fork);
    views.insert(
      after < 0 ? views.length : after,
      ItemView(
        id: _nextId--,
        kind: BridgeItemKind.dynamicToolCall,
        marker: mark.marker,
      ),
    );
    return true;
  }

  /// Reads the stored entries before the earliest one the chat covers, passing over pages the
  /// chat shows nothing of, such as tool results only.
  Future<void> loadEarlier() async {
    if (_readingVersion == _version) return;
    final version = _version;
    _readingVersion = version;
    try {
      while (more.value && _start != null) {
        final before = _start;
        final page = await _backend.transcript(before);
        reportRead('earlier messages', null);
        if (version != _version) return;
        _start = page.start;
        // A page that does not move back ends the reading, so it cannot repeat forever.
        more.value = page.more && page.start != before;
        if (page.items.isNotEmpty) {
          final shown = _placeTurnEnds([
            ..._stored(page.items),
            ...items.value,
          ]);
          if (_pendingMark case final mark?
              when _placeMark(shown, mark, complete: !more.value)) {
            _pendingMark = null;
          }
          // Counted once the mark is in, which also goes before the live turns.
          _shift(shown.length - items.value.length);
          items.value = shown;
          return;
        }
      }
      // The oldest pages showed nothing, so the branch leaves before everything shown.
      if (_pendingMark case final mark? when !more.value) {
        final shown = [...items.value];
        _placeMark(shown, mark, complete: true);
        _pendingMark = null;
        _shift(shown.length - items.value.length);
        items.value = shown;
      }
    } on Object catch (error) {
      reportRead('earlier messages', 'Could not read earlier messages: $error');
    } finally {
      // A newer chat may have started its own reading meanwhile; that one stays.
      if (_readingVersion == version) _readingVersion = null;
    }
  }

  List<ItemView> _stored(List<BridgeTranscriptItem> items) => [
    for (final item in items)
      ItemView(
          id: _nextId--,
          kind: item.kind,
          tool: item.tool,
          target: item.target,
          entry: item.entry,
        )
        ..text.value = item.text
        ..outcome.value = (
          failed: item.failed,
          durationMs: item.durationMs,
          lines: changedLines(item.added, item.removed),
        ),
  ];

  /// Each entry's position on the path from the root to `leaf`. Stored parents may loop, so the
  /// walk ends at the first entry it meets again.
  static Map<String, int> _pathIndex(List<BridgeTreeNode> tree, String? leaf) {
    final parents = {for (final node in tree) node.id: node.parent};
    final path = <String>[];
    final seen = <String>{};
    for (String? id = leaf; id != null && seen.add(id); id = parents[id]) {
      path.add(id);
    }
    return {for (final (index, id) in path.reversed.indexed) id: index};
  }
}

/// Where a branch leaves the others: its mark, each entry's position on the path to the leaf,
/// and the fork's position.
typedef _Mark = ({BranchMarker marker, Map<String, int> path, int fork});

/// Where a turn with a checkpoint ended live: which turn end it was, its checkpoint id and the
/// number of items before its row.
typedef _Boundary = ({int ordinal, String turn, int at});
