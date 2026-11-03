import 'dart:async';
import 'dart:collection';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import 'page_views.dart';
import 'workspace_state.dart';

/// Text typed at a paragraph's start that formats it, as Notion's Markdown shortcuts do
/// (approved Pages mockups).
const markdownShortcuts = {
  '# ': BridgeTextStyle.heading1,
  '## ': BridgeTextStyle.heading2,
  '### ': BridgeTextStyle.heading3,
  '- ': BridgeTextStyle.bullet,
  '* ': BridgeTextStyle.bullet,
  '1. ': BridgeTextStyle.numbered,
  '[] ': BridgeTextStyle.todo,
  '" ': BridgeTextStyle.quote,
};

/// The style of the Text Block ⏎ starts, as in Notion: lists and to-dos go on, the rest
/// becomes a paragraph.
BridgeTextStyle nextStyle(BridgeTextStyle style) => switch (style) {
  BridgeTextStyle.bullet => BridgeTextStyle.bullet,
  BridgeTextStyle.numbered => BridgeTextStyle.numbered,
  BridgeTextStyle.todo || BridgeTextStyle.done => BridgeTextStyle.todo,
  _ => BridgeTextStyle.paragraph,
};

/// Where `from` and `to` differ: the start, how much of `from` goes and what `to` puts
/// there, never between the halves of a surrogate pair, which the core refuses.
(int, int, String) difference(String from, String to) {
  final shorter = to.length < from.length ? to.length : from.length;
  var start = 0;
  while (start < shorter && to.codeUnitAt(start) == from.codeUnitAt(start)) {
    start++;
  }
  var end = 0;
  while (end < shorter - start &&
      to.codeUnitAt(to.length - 1 - end) ==
          from.codeUnitAt(from.length - 1 - end)) {
    end++;
  }
  bool trail(int at) =>
      at < from.length && (from.codeUnitAt(at) & 0xFC00) == 0xDC00;
  if (start > 0 && trail(start)) start--;
  if (end > 0 && trail(from.length - end)) end--;
  return (
    start,
    from.length - start - end,
    to.substring(start, to.length - end),
  );
}

/// The one edit that turns `from` into `to` in `block`.
BridgeMutation textEdit(String page, String block, String from, String to) {
  final (at, delete, insert) = difference(from, to);
  return mutation(
    BridgeMutationKind.editText,
    page: page,
    block: block,
    at: at,
    delete: delete,
    text: insert,
  );
}

/// A field that a change of the Page's structure starts from.
abstract interface class EditorSource {
  /// The Text Block it shows, or none for an empty Page's first line.
  String? get block;

  /// What the field holds now.
  TextEditingValue get typed;

  /// Shows `text`, what its Text Block holds once the change is done, or for the first line
  /// gives way to the new Text Block.
  void settle(String? text);
}

/// The Text Block that typing goes to, as the editor knows it.
class EditorTarget {
  EditorTarget({
    required this.view,
    required this.block,
    required this.index,
    required this.text,
    required this.caret,
    required this.style,
    this.boundary = false,
  });

  final String view;
  final String block;
  int index;
  String text;
  int caret;
  BridgeTextStyle style;
  bool boundary;
}

/// A key waiting for the change before it, with what the field held when it was pressed.
typedef _Queued = (TextEditingValue typed, LogicalKeyboardKey key);

/// A Page's ordered input queue (ADR-0240). ⏎, ⌫ at a Text Block's start and the first
/// character on an empty Page change the Page's structure, which the core answers within
/// milliseconds; Flutter never waits for it. Until the Text Block holding the caret takes
/// it, what is typed stays in the field it started from and ⏎, ⌫ and ⌦ wait in order;
/// each then takes its turn, and the typing between them goes to the Text Block that held
/// the caret then, also when the field is gone.
class PageEditor {
  PageEditor(this.views);

  final PageViews views;

  WorkspaceState get _state => views.state;
  String get _page => views.page;

  EditorSource? _source;
  bool _gone = false;
  TextEditingValue _latest = TextEditingValue.empty;
  final _queue = Queue<_Queued>();
  Completer<void>? _wake;

  /// Done once everything typed so far is written; quitting waits for it.
  Completer<void>? _writing;

  /// What the editor knows each Text Block holds, from its own writes.
  final _known = <String, String>{};

  /// Whether the Page's structure is changing.
  bool get busy => _source != null;

  /// The View whose Text Block a change started from, with its position: the Page keeps that
  /// Text Block, out of sight once its View is gone, until the change is done, so the keys
  /// typed meanwhile still reach it.
  final lingering = ValueNotifier<(BridgeView, int)?>(null);

  bool isSource(EditorSource source) => identical(_source, source);

  /// Starts `key` in `target`, whose field is `source`: ⏎ or ⌫ at its start.
  void start(EditorSource source, EditorTarget target, LogicalKeyboardKey key) {
    _begin(source);
    lingering.value = (
      BridgeView(
        id: target.view,
        block: target.block,
        boundary: target.boundary,
      ),
      target.index,
    );
    _known[target.block] = target.text;
    _queue.add((_latest, key));
    unawaited(_drive(target, null));
  }

  /// Makes the first Text Block of an empty Page from what `source` holds.
  void first(EditorSource source) {
    _begin(source);
    unawaited(_drive(null, _latest));
  }

  void _begin(EditorSource source) {
    _source = source;
    _gone = false;
    _latest = source.typed;
    _write();
  }

  /// Marks that input waits to be written, so quitting waits for it.
  void _write() {
    if (_writing != null) return;
    final writing = _writing = Completer<void>();
    _state.track(writing.future);
  }

  void _written() {
    _writing?.complete();
    _writing = null;
  }

  /// What the source's field holds after a change there.
  void typed(EditorSource source) {
    if (!isSource(source)) return;
    _latest = source.typed;
    _write();
    _wakeUp();
  }

  /// The source's field is gone; what it held last still counts.
  void left(EditorSource source) {
    if (!isSource(source)) return;
    _gone = true;
    _wakeUp();
  }

  void _wakeUp() {
    _wake?.complete();
    _wake = null;
  }

  Future<void> _sleep() => (_wake ??= Completer()).future;

  /// A key on a field while the structure changes: on the source, ⏎, ⌫ and ⌦ wait their
  /// turn; anywhere, the arrows do nothing; the rest types into the field.
  KeyEventResult queue(EditorSource source, KeyEvent event) {
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter ||
        key == LogicalKeyboardKey.backspace ||
        key == LogicalKeyboardKey.delete) {
      if (isSource(source)) {
        _queue.add((source.typed, key));
        _write();
        _wakeUp();
      }
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowUp ||
        key == LogicalKeyboardKey.arrowDown ||
        key == LogicalKeyboardKey.arrowLeft ||
        key == LogicalKeyboardKey.arrowRight) {
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  Future<BridgeApplied> _apply(BridgeMutation change, List<String> blocks) =>
      views.write(change, blocks);

  Future<void> _drive(EditorTarget? target, TextEditingValue? first) async {
    final source = _source!;
    var prev = first ?? _latest;
    var formatted = first == null;
    try {
      if (first != null) {
        final created = await _apply(
          mutation(
            BridgeMutationKind.addTextBlock,
            page: _page,
            text: first.text,
          ),
          const [],
        );
        target = EditorTarget(
          view: created.view!,
          block: created.block!,
          index: 0,
          text: first.text,
          caret: _caret(first),
          style: BridgeTextStyle.paragraph,
        );
        _known[target.block] = target.text;
      }
      // Where the field's caret was when the target took over, and the target's caret then.
      var boundary = _caret(prev);
      var base = target!.caret;
      while (true) {
        final _Queued? next = _queue.isEmpty ? null : _queue.removeFirst();
        final TextEditingValue snapshot;
        if (next != null) {
          snapshot = next.$1;
        } else {
          // A composition ends before its text moves, unless its field is gone.
          while (!_gone && !_latest.composing.isCollapsed) {
            await _sleep();
          }
          if (_queue.isNotEmpty) continue;
          snapshot = _latest;
        }
        await _route(target!, prev, snapshot, boundary, base);
        prev = snapshot;
        if (next != null) {
          target = await _perform(next.$2, target);
          boundary = _caret(snapshot);
          base = target.caret;
          continue;
        }
        if (_queue.isNotEmpty || _latest.text != prev.text) continue;
        if (!formatted) {
          formatted = true;
          await _shortcut(target);
          boundary = _caret(prev);
          base = target.caret;
          continue;
        }
        _written();
        if (await _handOver(target, prev)) break;
      }
      // The field shows what its own Text Block holds now; the target keeps the caret.
      final block = source.block;
      source.settle(block == null ? null : _known[block]);
      if (block == target.block) views.moveCaret(_caretOf(target));
    } on Object {
      // The problem shows on the Page; the stored text stays as the core has it.
      source.settle(null);
    } finally {
      _source = null;
      _queue.clear();
      _written();
      if (!views.closed) lingering.value = null;
    }
  }

  static int _caret(TextEditingValue value) =>
      value.selection.isValid ? value.selection.baseOffset : value.text.length;

  static Caret _caretOf(EditorTarget target) =>
      Caret(target.view, target.caret, text: target.text);

  /// Offers the caret to the target's Text Block. True once it took it or the Page closed;
  /// false when more input came first, which then takes its turn.
  Future<bool> _handOver(EditorTarget target, TextEditingValue prev) async {
    final offered = _caretOf(target);
    views.moveCaret(offered);
    if (views.closed) return true;
    views.caret.addListener(_wakeUp);
    try {
      while (true) {
        if (views.closed || !identical(views.caret.value, offered)) return true;
        if (_queue.isNotEmpty || _latest.text != prev.text) {
          views.caret.value = null;
          _write();
          return false;
        }
        await _sleep();
      }
    } finally {
      if (!views.closed) views.caret.removeListener(_wakeUp);
    }
  }

  /// Carries what the field gained from `from` to `to` into `target`, at the place that
  /// position `boundary` of the field stands for there, `base`.
  Future<void> _route(
    EditorTarget target,
    TextEditingValue from,
    TextEditingValue to,
    int boundary,
    int base,
  ) async {
    if (from.text == to.text) return;
    final (start, delete, insert) = difference(from.text, to.text);
    final at = (base + start - boundary).clamp(0, target.text.length);
    final end = (at + delete).clamp(at, target.text.length);
    await _replace(target, at, end, insert);
    target.caret = at + insert.length;
  }

  Future<EditorTarget> _replace(
    EditorTarget target,
    int start,
    int end,
    String insert,
  ) async {
    final text = target.text.replaceRange(start, end, insert);
    await _apply(textEdit(_page, target.block, target.text, text), [
      target.block,
    ]);
    _known[target.block] = text;
    return target
      ..text = text
      ..caret = start;
  }

  /// Runs `key` at the target's caret and returns where typing goes next.
  Future<EditorTarget> _perform(
    LogicalKeyboardKey key,
    EditorTarget target,
  ) async {
    if (key == LogicalKeyboardKey.backspace) return _backspace(target);
    if (key == LogicalKeyboardKey.delete) {
      if (target.caret >= target.text.length) return target;
      final at = target.caret;
      final wide =
          at + 1 < target.text.length &&
          (target.text.codeUnitAt(at) & 0xFC00) == 0xD800;
      return _replace(target, at, at + (wide ? 2 : 1), '');
    }
    return _enter(target);
  }

  Future<EditorTarget> _restyle(
    EditorTarget target,
    BridgeTextStyle style,
  ) async {
    await _apply(
      mutation(
        BridgeMutationKind.setTextStyle,
        page: _page,
        block: target.block,
        style: style,
      ),
      [target.block],
    );
    views.setStyle(target.block, style);
    return target..style = style;
  }

  /// ⏎ ends the Text Block at the caret and starts the next, as in Notion; an empty list
  /// item or to-do becomes a paragraph instead.
  Future<EditorTarget> _enter(EditorTarget target) async {
    if (target.text.isEmpty &&
        nextStyle(target.style) != BridgeTextStyle.paragraph) {
      return _restyle(target, BridgeTextStyle.paragraph);
    }
    final at = target.caret;
    final style = nextStyle(target.style);
    final applied = await _apply(
      mutation(
        BridgeMutationKind.splitTextBlock,
        page: _page,
        block: target.block,
        view: target.view,
        at: at,
        style: style,
      ),
      [target.block],
    );
    // At the start a new empty Text Block goes above, taking any boundary, and the text
    // stays in its own.
    if (at == 0) {
      return target
        ..index += 1
        ..boundary = false;
    }
    final tail = target.text.substring(at);
    _known[target.block] = target.text.substring(0, at);
    final created = EditorTarget(
      view: applied.view!,
      block: applied.block!,
      index: target.index + 1,
      text: tail,
      caret: 0,
      style: style,
    );
    _known[created.block] = tail;
    views.setStyle(created.block, style);
    return created;
  }

  /// ⌫ deletes the character before the caret; at a start it removes a divider above, then
  /// the formatting, then joins the Text Block to the one above, as the approved Section
  /// mockups and Notion do.
  Future<EditorTarget> _backspace(EditorTarget target) async {
    if (target.caret > 0) {
      var start = target.caret - 1;
      if (start > 0 && (target.text.codeUnitAt(start) & 0xFC00) == 0xDC00) {
        start--;
      }
      return _replace(target, start, target.caret, '');
    }
    if (target.boundary && target.index > 0) {
      await _apply(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: target.view,
          boundary: false,
        ),
        const [],
      );
      return target..boundary = false;
    }
    if (target.style != BridgeTextStyle.paragraph) {
      return _restyle(target, BridgeTextStyle.paragraph);
    }
    await _state.settled;
    final above = await _above(target);
    if (above == null) return target;
    final answer = await _state.workspace.query(
      query: query(BridgeQueryKind.text, page: _page, block: above.block),
    );
    final aboveText = answer.text ?? '';
    await _apply(
      mutation(
        BridgeMutationKind.joinTextBlock,
        page: _page,
        view: target.view,
      ),
      [above.block, target.block],
    );
    _known.remove(target.block);
    final joined = aboveText + target.text;
    _known[above.block] = joined;
    return EditorTarget(
      view: above.id,
      block: above.block,
      index: target.index - 1,
      text: joined,
      caret: aboveText.length,
      style: answer.style ?? BridgeTextStyle.paragraph,
      boundary: above.boundary,
    );
  }

  /// The View above the target's, as the core orders them now; the Page's own copy may
  /// not have caught up with the changes the queue just made.
  Future<BridgeView?> _above(EditorTarget target) async {
    for (var start = 0; ; start += window) {
      final answer = await _state.workspace.query(
        query: query(BridgeQueryKind.views, page: _page, start: start),
      );
      final at = answer.views.indexWhere((view) => view.id == target.view);
      if (at > 0) return answer.views[at - 1];
      if (at == 0) {
        if (start == 0) return null;
        final before = await _state.workspace.query(
          query: query(
            BridgeQueryKind.views,
            page: _page,
            start: start - 1,
            limit: 1,
          ),
        );
        return before.views.firstOrNull;
      }
      if (answer.views.length < window) return null;
    }
  }

  /// A Markdown shortcut typed at the start of a Page's first line formats its Text Block.
  Future<void> _shortcut(EditorTarget target) async {
    for (final MapEntry(key: typed, value: style)
        in markdownShortcuts.entries) {
      if (!target.text.startsWith(typed)) continue;
      final caret = target.caret;
      await _replace(target, 0, typed.length, '');
      target.caret = (caret - typed.length).clamp(0, target.text.length);
      await _restyle(target, style);
      return;
    }
  }
}
