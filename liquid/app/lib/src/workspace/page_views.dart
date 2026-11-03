import 'dart:async';

import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import 'follow.dart';
import 'page_editor.dart';
import 'workspace_state.dart';

/// Views read at once: a screenful and more, far below the bridge's window.
const _chunk = 100;

/// Where the caret goes once a Text Block's View is built.
@immutable
class Caret {
  const Caret(this.view, this.offset, {this.text, this.style});

  final String view;

  /// In UTF-16 units, as Dart and the core count text; past the end means the end.
  final int offset;

  /// What the Text Block holds, when the Page's input queue wrote it last, so it can take
  /// the caret before reading its text.
  final String? text;

  /// How that Text Block reads, as the input queue last set it.
  final BridgeTextStyle? style;
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
  final _loading = <int, Future<void>>{};

  /// Windows asked for while a reread ran, which it reads too.
  final _wanted = <int>{};
  StreamSubscription<BridgeChange>? _followed;
  int _generation = 0;
  bool _disposed = false;
  bool _reading = false;
  bool _again = false;

  /// False while the queue of this Page's earlier opening still writes, which its fields
  /// wait for before taking keys.
  final ready = ValueNotifier(true);

  void start() {
    if (state.pageWork(page) case final work?) {
      ready.value = false;
      unawaited(
        work.whenComplete(() {
          if (_disposed) return;
          ready.value = true;
          unawaited(_reread());
        }),
      );
    }
    _followed = follow(
      state.workspace,
      BridgeResource(kind: BridgeResourceKind.views, page: page),
    ).listen((_) => unawaited(_reread()), onError: _report);
  }

  /// The View at `index`, or null outside the Page or while its window is read.
  BridgeView? at(int index) {
    if (index < 0 || index >= (_count ?? index + 1)) return null;
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

  /// The Page's ordered input queue (ADR-0240).
  late final editor = PageEditor(this);

  final _pending = <String, int>{};
  final _sent = <String, int>{};
  final _settled = StreamController<String>.broadcast();

  /// Writes on their way to each Block, and all sent so far, so a read that crossed one is
  /// not shown.
  int pending(String block) => _pending[block] ?? 0;
  int sent(String block) => _sent[block] ?? 0;

  /// The Blocks whose writes have all arrived, each time that happens.
  Stream<String> get settled => _settled.stream;

  /// Has `block`'s Text Block read its text and style again.
  void reread(String block) {
    if (!_disposed) _settled.add(block);
  }

  /// Applies `change` after earlier writes, counting it for `blocks`.
  Future<BridgeApplied> write(
    BridgeMutation change,
    List<String> blocks,
  ) async {
    for (final block in blocks) {
      _pending[block] = pending(block) + 1;
      _sent[block] = sent(block) + 1;
    }
    try {
      return await state.apply(change);
    } finally {
      for (final block in blocks) {
        final left = _pending[block] = pending(block) - 1;
        if (left == 0 && !_disposed) _settled.add(block);
      }
    }
  }

  final _keys = <String, GlobalKey>{};

  /// The key of a View's Text Block, which keeps its state when the Page shows it elsewhere.
  GlobalKey keyFor(String view) =>
      _keys.putIfAbsent(view, () => GlobalKey(debugLabel: view));

  /// Whether the Page has closed.
  bool get closed => _disposed;

  /// Puts the caret's next place, unless the Page has closed.
  void moveCaret(Caret caret) {
    if (!_disposed) this.caret.value = caret;
  }

  void setStyle(String block, BridgeTextStyle style) {
    if (_disposed || styles.value[block] == style) return;
    styles.value = {...styles.value, block: style};
  }

  /// The View at `index`, read first if its window was not.
  Future<BridgeView?> viewAt(int index) async {
    if (index < 0) return null;
    final chunk = index ~/ _chunk;
    if (!_chunks.containsKey(chunk)) await _load(chunk);
    return at(index);
  }

  /// Reads the styles of the Text Blocks above `index` that no shown Text Block reported, as
  /// far back as a numbered run reaches, so its numbers count items out of view.
  Future<void> readRun(int index) async {
    for (var at = index - 1; at >= 0 && !_disposed; at--) {
      final view = await viewAt(at);
      if (view == null) return;
      var style = styles.value[view.block];
      if (style == null) {
        final answer = await state.workspace.query(
          query: query(BridgeQueryKind.text, page: page, block: view.block),
        );
        style = answer.style ?? BridgeTextStyle.paragraph;
        // A Text Block's own read, which may be newer, wins.
        if (styles.value.containsKey(view.block)) {
          style = styles.value[view.block];
        } else {
          setStyle(view.block, style);
        }
      }
      if (style != BridgeTextStyle.numbered || view.boundary) return;
    }
  }

  /// Reads one window; one the Page changed under is read again.
  Future<void> _load(int chunk) =>
      _loading[chunk] ??= _read(chunk)
          .whenComplete(() => _loading.remove(chunk));

  Future<void> _read(int chunk) async {
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
      if (_disposed) return;
      // A reread running now reads this window too, so no order mixes with another.
      if (_reading) {
        _wanted.add(chunk);
        _again = true;
        return;
      }
      if (generation != _generation) {
        // Read again once this read has left the list of running ones.
        scheduleMicrotask(() => unawaited(_load(chunk)));
        return;
      }
      _count = answer.count;
      _chunks[chunk] = answer.views;
      notifyListeners();
    } on Object catch (error) {
      _report(error);
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
        for (final chunk in {0, ..._chunks.keys, ..._wanted}) {
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
        // Windows read across a change may disagree; read them all again first.
        if (_again) continue;
        _wanted.clear();
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
    editor.lingering.dispose();
    ready.dispose();
    unawaited(_settled.close());
    super.dispose();
  }
}
