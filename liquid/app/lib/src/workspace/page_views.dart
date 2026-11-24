import 'dart:async';

import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import '../whiteboard/board_controller.dart';
import 'follow.dart';
import 'media_block.dart';
import 'page_editor.dart';
import 'windowed_list.dart';
import 'workspace_state.dart';

const caretEnd = 1 << 30;

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

  final BridgeBlockKind kind;

  final int offset;

  final String? text;

  final BridgeTextStyle? style;

  final bool slash;
}

const keptViewWindows = 10;

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
      arranged = answer.arranged;
      return (answer.count, answer.views);
    },
    _report,
    kept: keptViewWindows,
  );

  int? get count => _views.count;

  bool arranged = false;

  final arranging = ValueNotifier(false);

  final narrowPreview = ValueNotifier(false);

  String? lastFocused;

  final caret = ValueNotifier<Caret?>(null);

  final pastedCopy = ValueNotifier<({String view, CopiedView copied})?>(null);

  final naming = <String, (String, bool) Function()>{};

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

  final fullBoard = ValueNotifier<BridgeView?>(null);

  final focusBoard = ValueNotifier<String?>(null);

  final cellDrops = <String, CellDrop>{};

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

  final styles = ValueNotifier<Map<String, BridgeTextStyle>>(const {});

  StreamSubscription<BridgeChange>? _followed;
  bool _disposed = false;

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

  BridgeView? peek(int index) => _views.peek(index);

  BridgeView? at(int index) => _views.at(index);

  int? indexOf(String view) => _views.indexWhere((shown) => shown.id == view);

  BridgeView? textFrom(int index, int by) {
    for (var at = index; ; at += by) {
      final view = _views.at(at);
      if (view == null || view.kind == BridgeBlockKind.text) return view;
    }
  }

  BridgeView? caretFrom(int index, int by) {
    for (var at = index; ; at += by) {
      final view = _views.at(at);
      if (view == null ||
          view.kind == BridgeBlockKind.text ||
          view.kind == BridgeBlockKind.code ||
          view.kind == BridgeBlockKind.math) {
        return view;
      }
    }
  }

  late final editor = PageEditor(this);

  final _pending = <String, int>{};
  final _sent = <String, int>{};
  final _settled = StreamController<String>.broadcast();

  int pending(String block) => _pending[block] ?? 0;
  int sent(String block) => _sent[block] ?? 0;

  Stream<String> get settled => _settled.stream;

  void reread(String block) {
    if (!_disposed) _settled.add(block);
  }

  Future<bool> pasteCopy(CopiedView copied, String? after) async {
    try {
      final added = await write(
        mutation(
          BridgeMutationKind.duplicateTable,
          page: page,
          after: after,
          block: copied.table,
          text: '${copied.text} (copy)',
          tabs: copied.tabs,
        ),
        const [],
      );
      pastedCopy.value = (view: added.view!, copied: copied);
      return true;
    } on Object {
      return false;
    }
  }

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

  GlobalKey keyFor(String view) =>
      _keys.putIfAbsent(view, () => GlobalKey(debugLabel: view));

  bool get closed => _disposed;

  void moveCaret(Caret caret) {
    if (!_disposed) this.caret.value = caret;
  }

  void setStyle(String block, BridgeTextStyle style) {
    if (_disposed || styles.value[block] == style) return;
    styles.value = {...styles.value, block: style};
  }

  Future<BridgeView?> viewAt(int index) => _views.load(index);

  Future<void> readRun(int index) async {
    final first = (index ~/ windowItems - (keptViewWindows - 3)) * windowItems;
    for (var at = index - 1; at >= first && at >= 0 && !_disposed; at--) {
      final view = await viewAt(at);
      if (view == null || view.kind != BridgeBlockKind.text) return;
      var style = styles.value[view.block];
      if (style == null) {
        final answer = await state.read(
          query(BridgeQueryKind.text, page: page, block: view.block),
        );
        style = answer.style ?? BridgeTextStyle.paragraph;
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
    pastedCopy.dispose();
    arranging.dispose();
    narrowPreview.dispose();
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

abstract interface class CellDrop {
  bool hover(Offset global);
  void leave();
  Future<void> drop(List<MediaSource> sources, Offset global);
}
