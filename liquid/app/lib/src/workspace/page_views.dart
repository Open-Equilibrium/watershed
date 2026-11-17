import 'dart:async';

import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import '../whiteboard/board_controller.dart';
import 'follow.dart';
import 'media_block.dart';
import 'page_editor.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

/// A caret offset that means a Block's end.
const caretEnd = 1 << 30;

/// Where the caret goes once a Text or Code Block's View is built.
@immutable
class Caret {
  const Caret(
    this.view,
    this.offset, {
    this.kind = BridgeBlockKind.text,
    this.text,
    this.style,
    this.slash = false,
  });

  final String view;

  /// The kind of Block that takes it, so a View whose Block just turned into another kind
  /// hands the caret to its new Block, not to the one still built.
  final BridgeBlockKind kind;

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

/// How many windows of a Page's Views stay read: a long Page holds and rereads only those
/// last used, not all it was scrolled through (client architecture rules 3 and 5).
const keptViewWindows = 10;

/// An open Page's Views in order, read in windows as the Page scrolls to them, and reread
/// after each change. It also keeps what the Page's Text Blocks share: their styles, for
/// numbering, and where the caret goes next.
class PageViews extends ChangeNotifier {
  PageViews(this.state, this.page) {
    _views.addListener(notifyListeners);
  }

  final WorkspaceState state;
  final String page;

  late final _views = WindowedList<BridgeView>(
    (start, limit) async {
      final answer = await state.read(
        query(BridgeQueryKind.views, page: page, start: start, limit: limit),
      );
      return (answer.count, answer.views);
    },
    _report,
    kept: keptViewWindows,
  );

  /// How many Views the Page shows; null until first read.
  int? get count => _views.count;

  /// The caret's next place, taken by the Text Block it names once built.
  final caret = ValueNotifier<Caret?>(null);

  /// What a line that a Table took the place of holds, by the Table's View: its name,
  /// read when the Table takes the caret, and whether it was written already.
  final naming = <String, (String, bool) Function()>{};

  /// Each Whiteboard's board, shared by the Block and its full-screen view, with how many
  /// of them use it; the last to go releases it, its subscription and caches with it.
  final _boards = <String, (BoardController, int)>{};

  BoardController boardFor(BridgeView view) {
    final (board, users) = _boards[view.id] ?? (BoardController(this, view), 0);
    _boards[view.id] = (board, users + 1);
    return board;
  }

  void releaseBoard(BridgeView view) {
    final held = _boards[view.id];
    if (held == null) return;
    final (board, users) = held;
    if (users > 1) {
      _boards[view.id] = (board, users - 1);
    } else {
      _boards.remove(view.id);
      board.dispose();
    }
  }

  /// The Whiteboard shown full screen over the Page, if any.
  final fullBoard = ValueNotifier<BridgeView?>(null);

  /// The View of a new Whiteboard, which takes the focus once built.
  final focusBoard = ValueNotifier<String?>(null);

  /// The Blocks that take dropped files into a cell, by their View, as a Table's file
  /// cells do.
  final cellDrops = <String, CellDrop>{};

  /// Files going in, each shown after the View it follows until its Media Block is in.
  final uploads = ValueNotifier<List<PendingUpload>>(const []);

  void addUpload(PendingUpload upload) {
    if (!_disposed) uploads.value = [...uploads.value, upload];
  }

  void removeUpload(PendingUpload upload) {
    if (!_disposed) {
      uploads.value = [
        for (final shown in uploads.value)
          if (shown != upload) shown,
      ];
    }
  }

  /// The styles Text Blocks read, by Block.
  final styles = ValueNotifier<Map<String, BridgeTextStyle>>(const {});

  StreamSubscription<BridgeChange>? _followed;
  bool _disposed = false;

  /// False while the queue of this Page's earlier opening still writes and its Views are
  /// read again after it, which its fields wait for before taking keys.
  final ready = ValueNotifier(true);

  void start() {
    if (state.pageWork(page) case final work?) {
      ready.value = false;
      unawaited(
        work.whenComplete(() async {
          if (_disposed) return;
          await _views.reread();
          if (!_disposed) ready.value = true;
        }),
      );
    }
    _followed = follow(
      state.workspace,
      BridgeResource(kind: BridgeResourceKind.views, page: page),
    ).listen((_) => unawaited(_views.reread()), onError: _report);
  }

  /// The View at `index` if its window is kept, without reading it.
  BridgeView? peek(int index) => _views.peek(index);

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

  /// The nearest View that takes the caret from `index` on, going by `by`: a Text or Code
  /// Block's, past other kinds; null if none is read there.
  BridgeView? caretFrom(int index, int by) {
    for (var at = index; ; at += by) {
      final view = _views.at(at);
      if (view == null ||
          view.kind == BridgeBlockKind.text ||
          view.kind == BridgeBlockKind.code) {
        return view;
      }
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

  /// Applies `change` after earlier writes, in its `chain` if given, counting it for `blocks`.
  Future<BridgeApplied> write(
    BridgeMutation change,
    List<String> blocks, {
    EditChain? chain,
  }) async {
    for (final block in blocks) {
      _pending[block] = pending(block) + 1;
      _sent[block] = sent(block) + 1;
    }
    try {
      return await state.apply(change, chain: chain);
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
  /// far back as a numbered run reaches, so its numbers count items out of view; at most
  /// through the windows that fit beside the shown ones, which a longer run would push out.
  Future<void> readRun(int index) async {
    final first = (index ~/ windowItems - (keptViewWindows - 3)) * windowItems;
    for (var at = index - 1; at >= first && at >= 0 && !_disposed; at--) {
      final view = await viewAt(at);
      // A numbered run ends at a Block of another kind.
      if (view == null || view.kind != BridgeBlockKind.text) return;
      var style = styles.value[view.block];
      if (style == null) {
        final answer = await state.read(
          query(BridgeQueryKind.text, page: page, block: view.block),
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
    if (!_disposed) state.readFailed(error);
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_followed?.cancel());
    _views.dispose();
    caret.dispose();
    styles.dispose();
    uploads.dispose();
    for (final (board, _) in _boards.values) {
      board.dispose();
    }
    fullBoard.dispose();
    focusBoard.dispose();
    editor.lingering.dispose();
    ready.dispose();
    unawaited(_settled.close());
    super.dispose();
  }
}

/// A Block's cells that take dropped files, as a Table's file cells do: [hover] says
/// whether one lies under the pointer, marking it, [leave] unmarks it, and [drop] adds the
/// files to the cell there.
abstract interface class CellDrop {
  bool hover(Offset global);
  void leave();
  Future<void> drop(List<MediaSource> sources, Offset global);
}
