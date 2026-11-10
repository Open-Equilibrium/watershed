import 'dart:async';

import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import 'follow.dart';
import 'page_editor.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

/// Where the caret goes once a Text Block's View is built.
@immutable
class Caret {
  const Caret(
    this.view,
    this.offset, {
    this.text,
    this.style,
    this.slash = false,
  });

  final String view;

  /// In UTF-16 units, as Dart and the core count text; past the end means the end.
  final int offset;

  /// What the Text Block holds, when the Page's input queue wrote it last, so it can take
  /// the caret before reading its text.
  final String? text;

  /// How that Text Block reads, as the input queue last set it.
  final BridgeTextStyle? style;

  /// Whether an empty Page's first line made that Text Block from `/` alone, handing over
  /// what was typed after it.
  final bool slash;
}

/// An open Page's Views in order, read in windows as the Page scrolls to them, and reread
/// after each change. It also keeps what the Page's Text Blocks share: their styles, for
/// numbering, and where the caret goes next.
class PageViews extends ChangeNotifier {
  PageViews(this.state, this.page) {
    _views.addListener(notifyListeners);
  }

  final WorkspaceState state;
  final String page;

  late final _views = WindowedList<BridgeView>((start, limit) async {
    final answer = await state.workspace.query(
      query: query(
        BridgeQueryKind.views,
        page: page,
        start: start,
        limit: limit,
      ),
    );
    return (answer.count, answer.views);
  }, _report);

  /// How many Views the Page shows; null until first read.
  int? get count => _views.count;

  /// The caret's next place, taken by the Text Block it names once built.
  final caret = ValueNotifier<Caret?>(null);

  /// What a line that a Table took the place of holds, by the Table's View: its name,
  /// read when the Table takes the caret, and whether it was written already.
  final naming = <String, (String, bool) Function()>{};

  /// The styles Text Blocks read, by Block.
  final styles = ValueNotifier<Map<String, BridgeTextStyle>>(const {});

  StreamSubscription<BridgeChange>? _followed;
  bool _disposed = false;

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
          unawaited(_views.reread());
        }),
      );
    }
    _followed = follow(
      state.workspace,
      BridgeResource(kind: BridgeResourceKind.views, page: page),
    ).listen((_) => unawaited(_views.reread()), onError: _report);
  }

  /// The View at `index`, or null outside the Page or while its window is read.
  BridgeView? at(int index) => _views.at(index);

  /// The position of a read View, or null.
  int? indexOf(String view) => _views.indexWhere((shown) => shown.id == view);

  /// The nearest Text Block's View from `index` on, going by `by`, past other kinds; null
  /// if none is read there.
  BridgeView? textFrom(int index, int by) {
    for (var at = index; ; at += by) {
      final view = _views.at(at);
      if (view == null || view.kind == BridgeBlockKind.text) return view;
    }
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
  Future<BridgeView?> viewAt(int index) => _views.load(index);

  /// Reads the styles of the Text Blocks above `index` that no shown Text Block reported, as
  /// far back as a numbered run reaches, so its numbers count items out of view.
  Future<void> readRun(int index) async {
    for (var at = index - 1; at >= 0 && !_disposed; at--) {
      final view = await viewAt(at);
      // A numbered run ends at a Block of another kind.
      if (view == null || view.kind != BridgeBlockKind.text) return;
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

  void _report(Object error) {
    if (!_disposed) state.problem.value = '$error';
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_followed?.cancel());
    _views.dispose();
    caret.dispose();
    styles.dispose();
    editor.lingering.dispose();
    ready.dispose();
    unawaited(_settled.close());
    super.dispose();
  }
}
