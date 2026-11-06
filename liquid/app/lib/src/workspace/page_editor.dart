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

/// What `---` typed alone in a Text Block becomes: a Section boundary (approved Section
/// mockups).
const sectionBreak = '---';

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

  /// Collapses a selection to its end, as a queued key that replaces it is replayed.
  void collapse();

  /// Whether it has the keyboard focus, which a person may move elsewhere meanwhile.
  bool get focused;
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

/// A key waiting for the change before it, with what its field held when it was pressed.
/// A key from another field also names that field, its Text Block and what the field before
/// held last, all of which is written before the key.
class _Queued {
  _Queued(this.typed, this.key, {this.source, this.target, this.left});

  final TextEditingValue typed;
  final LogicalKeyboardKey key;
  final EditorSource? source;
  final EditorTarget? target;
  final TextEditingValue? left;
}

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

  /// Every field the running change took input from; what they held belongs to the queue.
  final _owned = <EditorSource>{};

  /// Whether `source`'s input belongs to the running change, also after typing moved on.
  bool owns(EditorSource source) => _owned.contains(source);

  /// Starts `key` in `target`, whose field is `source`: ⏎ or ⌫ at its start. While a change
  /// runs from another field, `source` becomes the field typing goes to, and its key waits
  /// its turn behind what the field before held.
  void start(EditorSource source, EditorTarget target, LogicalKeyboardKey key) {
    if (busy) {
      final left = _latest;
      _source = source;
      _owned.add(source);
      _gone = false;
      _latest = source.typed;
      _linger(target);
      _known[target.block] = target.text;
      _queue.add(
        _Queued(_latest, key, source: source, target: target, left: left),
      );
      _write();
      _wakeUp();
      return;
    }
    _begin(source);
    _linger(target);
    _known[target.block] = target.text;
    _queue.add(_Queued(_latest, key));
    _state.trackPage(_page, _drive(target, null));
  }

  void _linger(EditorTarget target) {
    lingering.value = (
      BridgeView(
        id: target.view,
        block: target.block,
        boundary: target.boundary,
        // The queue changes only Text Blocks.
        kind: BridgeBlockKind.text,
      ),
      target.index,
    );
  }

  /// Makes the first Text Block of an empty Page from what `source` holds.
  void first(EditorSource source) {
    _begin(source);
    _state.trackPage(_page, _drive(null, _latest));
  }

  void _begin(EditorSource source) {
    _source = source;
    _owned.add(source);
    _gone = false;
    _latest = source.typed;
    _state.finishing.add(_finish);
    _write();
  }

  /// Liquid quits: a composition still open is taken as it stands.
  void _finish() => _wakeUp();

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

  /// A key on the source's field while the structure changes: ⏎, ⌫ and ⌦ wait their turn,
  /// the arrows do nothing, and the rest types into the field.
  KeyEventResult queue(EditorSource source, KeyEvent event) {
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter ||
        key == LogicalKeyboardKey.backspace ||
        key == LogicalKeyboardKey.delete) {
      _queue.add(_Queued(source.typed, key));
      source.collapse();
      _write();
      _wakeUp();
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
    var source = _source!;
    final sources = [source];
    var prev = first ?? _latest;
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
        await _markdown(target, '');
      }
      // Where the field's caret was when the target took over, and the target's caret then.
      var boundary = _caret(prev);
      var base = target!.caret;
      int place(int field) =>
          (base + field - boundary).clamp(0, target!.text.length);
      while (true) {
        final _Queued? next = _queue.isEmpty ? null : _queue.removeFirst();
        if (next?.left case final left?) {
          // What the field before still held goes in before the other field's key.
          final before = target!.text;
          await _route(target, prev, left, boundary, base);
          await _markdown(target, before);
          source = next!.source!;
          sources.add(source);
          target = next.target!;
          prev = next.typed;
          boundary = _caret(prev);
          base = target.caret = _caret(prev);
        }
        final TextEditingValue snapshot;
        if (next != null) {
          snapshot = next.typed;
        } else {
          // A composition ends before its text moves, unless its field is gone or Liquid
          // quits.
          while (!_gone && !_state.quitting && !_latest.composing.isCollapsed) {
            await _sleep();
          }
          if (_queue.isNotEmpty) continue;
          snapshot = _latest;
        }
        final before = target!.text;
        await _route(target, prev, snapshot, boundary, base);
        prev = snapshot;
        if (await _markdown(target, before)) {
          boundary = _caret(prev);
          base = target.caret;
        }
        if (next != null) {
          // The key acts where the field's caret or selection was when it was pressed.
          final selection = snapshot.selection;
          final key = next.key;
          if (selection.isValid) {
            final (start, end) = (place(selection.start), place(selection.end));
            if (end > start) {
              await _replace(target, start, end, '');
            } else {
              target.caret = start;
            }
          }
          final replaced = selection.isValid && !selection.isCollapsed;
          if (!(replaced &&
              (key == LogicalKeyboardKey.backspace ||
                  key == LogicalKeyboardKey.delete))) {
            target = await _perform(key, target);
          }
          boundary = selection.isValid ? selection.end : _caret(snapshot);
          base = target.caret;
          continue;
        }
        if (_queue.isNotEmpty || _latest.text != prev.text) continue;
        _written();
        // The field it started from shows its own Text Block's text before the caret moves,
        // so a long one leaves the target in view.
        final block = source.block;
        if (!_gone && block != null && block != target.block) {
          source.settle(_known[block]);
          prev = _latest = source.typed;
          boundary = _caret(prev);
          base = target.caret;
        }
        if (await _handOver(target, prev)) break;
      }
      for (final settled in sources) {
        final block = settled.block;
        settled.settle(block == null ? null : _known[block]);
        if (block == target.block && settled.focused) {
          views.moveCaret(_caretOf(target));
        }
        // Each Text Block reads its text and style again, now that the queue let go.
        if (block != null) views.reread(block);
      }
      views.reread(target.block);
    } on Object {
      // The problem shows on the Page; the stored text stays as the core has it.
      for (final settled in sources) {
        settled.settle(null);
      }
    } finally {
      _source = null;
      _owned.clear();
      _queue.clear();
      _state.finishing.remove(_finish);
      _written();
      if (!views.closed) lingering.value = null;
    }
  }

  static int _caret(TextEditingValue value) =>
      value.selection.isValid ? value.selection.baseOffset : value.text.length;

  static Caret _caretOf(EditorTarget target) =>
      Caret(target.view, target.caret, text: target.text, style: target.style);

  /// Offers the caret to the target's Text Block. True once it took it or the Page closed;
  /// false when more input came first, which then takes its turn.
  Future<bool> _handOver(EditorTarget target, TextEditingValue prev) async {
    // A person who moved to another field meanwhile keeps the caret there.
    if (!_gone && !(_source?.focused ?? true)) return true;
    final offered = _caretOf(target);
    views.moveCaret(offered);
    if (views.closed) return true;
    // A target never built, as when the Page scrolled far away, leaves the caret where it is.
    var expired = false;
    final timer = Timer(const Duration(seconds: 1), () {
      expired = true;
      _wakeUp();
    });
    views.caret.addListener(_wakeUp);
    try {
      while (true) {
        if (views.closed || !identical(views.caret.value, offered)) return true;
        if (expired) {
          views.caret.value = null;
          return true;
        }
        if (_queue.isNotEmpty || _latest.text != prev.text) {
          views.caret.value = null;
          _write();
          return false;
        }
        await _sleep();
      }
    } finally {
      timer.cancel();
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
      // A whole character as a person sees it, as Flutter's own ⌦ deletes.
      final next = target.text.substring(target.caret).characters.first;
      return _replace(target, target.caret, target.caret + next.length, '');
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
      // A whole character as a person sees it, as Flutter's own ⌫ deletes.
      final last = target.text.substring(0, target.caret).characters.last;
      return _replace(target, target.caret - last.length, target.caret, '');
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
    // Only Text Blocks join.
    if (above == null || above.kind != BridgeBlockKind.text) return target;
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

  /// A Markdown shortcut just typed at a paragraph's start formats it, and `---` typed
  /// alone starts a Section, as typing in a Text Block does; true if one applied.
  Future<bool> _markdown(EditorTarget target, String before) async {
    if (await _sectionBreak(target, before)) return true;
    if (target.style != BridgeTextStyle.paragraph) return false;
    for (final MapEntry(key: typed, value: style)
        in markdownShortcuts.entries) {
      if (!target.text.startsWith(typed) ||
          before.startsWith(typed) ||
          target.caret != typed.length) {
        continue;
      }
      await _replace(target, 0, typed.length, '');
      await _restyle(target, style);
      return true;
    }
    return false;
  }

  /// `---` typed alone starts a Section, in any Text Block but the Page's first, where a
  /// boundary already exists (approved Section mockups).
  Future<bool> _sectionBreak(EditorTarget target, String before) async {
    if (target.text == sectionBreak &&
        before != sectionBreak &&
        target.index > 0 &&
        !target.boundary) {
      await _replace(target, 0, target.text.length, '');
      await _apply(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: target.view,
          boundary: true,
        ),
        const [],
      );
      target.boundary = true;
      return true;
    }
    return false;
  }
}
