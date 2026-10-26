import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../trajectory/trajectory_model.dart';
import 'backend.dart';
import 'session_state.dart';

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
  bool _readingEarlier = false;

  /// Rebuilt items count down from -1, so they never meet live item ids.
  int _nextId = -1;

  /// Forgets the previous session's chat; reads still running for it are dropped.
  void reset() {
    _version++;
    _start = null;
    _pendingMark = null;
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
    try {
      page = await _backend.transcript(null);
      history = await _backend.history(const []);
    } on Object catch (error) {
      notify(BridgeLevel.error, 'Could not restore the chat: $error');
      return;
    }
    if (version != _version) return;
    // What the stored responses used; live requests add to it.
    totals.value = const SessionTotals().add(history.usage);
    final rebuilt = _stored(page.items);
    _pendingMark = null;
    if (branchMarker(history.tree, history.leaf) case final marker?) {
      final path = _pathIndex(history.tree, history.leaf);
      final mark = (marker: marker, path: path, fork: path[marker.fork] ?? -1);
      if (!_placeMark(rebuilt, mark, complete: !page.more)) _pendingMark = mark;
    }
    // Only live items arrived meanwhile; items another restore rebuilt are replaced.
    final arrived = items.value.where(
      (view) =>
          !before.contains(view) && view.entry == null && view.marker == null,
    );
    items.value = [...rebuilt, ...arrived];
    _start = page.start;
    more.value = page.more;
    if (page.items.isEmpty) await loadEarlier();
  }

  /// The branch mark until the chat shows where the branch leaves the others.
  _Mark? _pendingMark;

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
    if (_readingEarlier) return;
    _readingEarlier = true;
    final version = _version;
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
          final shown = [..._stored(page.items), ...items.value];
          if (_pendingMark case final mark?
              when _placeMark(shown, mark, complete: !more.value)) {
            _pendingMark = null;
          }
          items.value = shown;
          return;
        }
      }
      // The oldest pages showed nothing, so the branch leaves before everything shown.
      if (_pendingMark case final mark? when !more.value) {
        final shown = [...items.value];
        _placeMark(shown, mark, complete: true);
        _pendingMark = null;
        items.value = shown;
      }
    } on Object catch (error) {
      reportRead('earlier messages', 'Could not read earlier messages: $error');
    } finally {
      _readingEarlier = false;
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

  /// Each entry's position on the path from the root to `leaf`.
  static Map<String, int> _pathIndex(List<BridgeTreeNode> tree, String? leaf) {
    final parents = {for (final node in tree) node.id: node.parent};
    final path = <String>[];
    for (String? id = leaf; id != null; id = parents[id]) {
      path.add(id);
    }
    return {for (final (index, id) in path.reversed.indexed) id: index};
  }
}

/// Where a branch leaves the others: its mark, each entry's position on the path to the leaf,
/// and the fork's position.
typedef _Mark = ({BranchMarker marker, Map<String, int> path, int fork});
