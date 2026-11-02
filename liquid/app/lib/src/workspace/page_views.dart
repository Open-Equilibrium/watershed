import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/workspace.dart';
import 'follow.dart';
import 'workspace_state.dart';

/// Views read at once: a screenful and more, far below the bridge's window.
const _chunk = 100;

/// Where the caret goes once a Text Block's View is built.
@immutable
class Caret {
  const Caret(this.view, this.offset);

  final String view;

  /// In UTF-16 units, as Dart and the core count text; past the end means the end.
  final int offset;
}

/// An open Page's Views in order, read in windows as the Page scrolls to them, never the
/// whole order at once (client architecture rules 3 and 5). Each change rereads the count
/// and the windows read so far. It also keeps what the Page's Text Blocks share: their
/// styles, for numbering, and where the caret goes next.
class PageViews extends ChangeNotifier {
  PageViews(this.state, this.page);

  final WorkspaceState state;
  final String page;

  /// How many Views the Page shows; null until first read.
  int? get count => _count;
  int? _count;

  /// The caret's next place, taken by the Text Block it names once built.
  final caret = ValueNotifier<Caret?>(null);

  /// The styles Text Blocks read, by Block.
  final styles = ValueNotifier<Map<String, BridgeTextStyle>>(const {});

  final _chunks = <int, List<BridgeView>>{};
  final _loading = <int>{};
  StreamSubscription<BridgeChange>? _followed;
  int _generation = 0;
  bool _disposed = false;
  bool _reading = false;
  bool _again = false;

  void start() {
    _followed = follow(
      state.workspace,
      BridgeResource(kind: BridgeResourceKind.views, page: page),
    ).listen((_) => unawaited(_reread()), onError: _report);
  }

  /// The View at `index`, or null while its window is read.
  BridgeView? at(int index) {
    final chunk = index ~/ _chunk;
    final views = _chunks[chunk];
    if (views == null) {
      unawaited(_load(chunk));
      return null;
    }
    final at = index % _chunk;
    return at < views.length ? views[at] : null;
  }

  /// The position of a read View, or null.
  int? indexOf(String view) {
    for (final MapEntry(key: chunk, value: views) in _chunks.entries) {
      final at = views.indexWhere((shown) => shown.id == view);
      if (at >= 0) return chunk * _chunk + at;
    }
    return null;
  }

  void setStyle(String block, BridgeTextStyle style) {
    if (_disposed || styles.value[block] == style) return;
    styles.value = {...styles.value, block: style};
  }

  Future<void> _load(int chunk) async {
    if (!_loading.add(chunk)) return;
    final generation = _generation;
    try {
      final answer = await state.workspace.query(
        query: query(
          BridgeQueryKind.views,
          page: page,
          start: chunk * _chunk,
          limit: _chunk,
        ),
      );
      if (_disposed || generation != _generation) return;
      _count = answer.count;
      _chunks[chunk] = answer.views;
      notifyListeners();
    } on Object catch (error) {
      _report(error);
    } finally {
      _loading.remove(chunk);
    }
  }

  /// Rereads the count and every window read so far, one reread at a time.
  Future<void> _reread() async {
    if (_reading) {
      _again = true;
      return;
    }
    _reading = true;
    try {
      do {
        _again = false;
        final generation = ++_generation;
        final read = <int, List<BridgeView>>{};
        var count = 0;
        for (final chunk in {0, ..._chunks.keys}) {
          final answer = await state.workspace.query(
            query: query(
              BridgeQueryKind.views,
              page: page,
              start: chunk * _chunk,
              limit: _chunk,
            ),
          );
          count = answer.count;
          read[chunk] = answer.views;
        }
        if (_disposed || generation != _generation) return;
        _count = count;
        _chunks
          ..clear()
          ..addAll(read);
        notifyListeners();
      } while (_again);
    } on Object catch (error) {
      _report(error);
    } finally {
      _reading = false;
    }
  }

  void _report(Object error) {
    if (!_disposed) state.problem.value = '$error';
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_followed?.cancel());
    caret.dispose();
    styles.dispose();
    super.dispose();
  }
}
