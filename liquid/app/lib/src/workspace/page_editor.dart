import 'dart:async';
import 'dart:collection';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../rust/api/workspace.dart';
import 'page_views.dart';
import 'workspace_state.dart';

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

const codeShortcut = '```';

bool codeTyped(String text, String before, int caret) =>
    text.startsWith(codeShortcut) &&
    !before.startsWith(codeShortcut) &&
    caret == codeShortcut.length;

const sectionBreak = '---';

(String, BridgeTextStyle)? shortcutTyped(
  String text,
  String before,
  int caret,
  BridgeTextStyle style,
) {
  if (style != BridgeTextStyle.paragraph) return null;
  for (final MapEntry(key: typed, value: style) in markdownShortcuts.entries) {
    if (text.startsWith(typed) &&
        !before.startsWith(typed) &&
        caret == typed.length) {
      return (typed, style);
    }
  }
  return null;
}

bool startsSection(
  String text,
  String before, {
  required int index,
  required bool boundary,
}) => text == sectionBreak && before != sectionBreak && index > 0 && !boundary;

BridgeTextStyle nextStyle(BridgeTextStyle style) => switch (style) {
  BridgeTextStyle.bullet => BridgeTextStyle.bullet,
  BridgeTextStyle.numbered => BridgeTextStyle.numbered,
  BridgeTextStyle.todo || BridgeTextStyle.done => BridgeTextStyle.todo,
  _ => BridgeTextStyle.paragraph,
};

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

abstract interface class EditorSource {
  String? get block;

  TextEditingValue get typed;

  void settle(String? text);

  void collapse();

  bool get focused;
}

class EditorTarget {
  EditorTarget({
    required this.view,
    required this.block,
    required this.index,
    required this.text,
    required this.caret,
    required this.style,
    this.boundary = false,
    this.slash = false,
  });

  final String view;
  final String block;
  int index;
  String text;
  int caret;
  BridgeTextStyle style;
  bool boundary;

  final bool slash;
}

class _Queued {
  _Queued(this.typed, this.key, {this.source, this.target, this.left});

  final TextEditingValue typed;
  final LogicalKeyboardKey key;
  final EditorSource? source;
  final EditorTarget? target;
  final TextEditingValue? left;
}

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

  Completer<void>? _writing;

  final _known = <String, String>{};

  bool get busy => _source != null;

  final lingering = ValueNotifier<(BridgeView, int)?>(null);

  bool isSource(EditorSource source) => identical(_source, source);

  final _owned = <EditorSource>{};

  bool owns(EditorSource source) => _owned.contains(source);

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
        title: '',
        boundary: target.boundary,
        kind: BridgeBlockKind.text,
      ),
      target.index,
    );
  }

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

  void _finish() => _wakeUp();

  void _write() {
    if (_writing != null) return;
    final writing = _writing = Completer<void>();
    _state.track(writing.future);
  }

  void _written() {
    _writing?.complete();
    _writing = null;
  }

  void typed(EditorSource source) {
    if (!isSource(source)) return;
    _latest = source.typed;
    _write();
    _wakeUp();
  }

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
          slash: first.text == '/',
        );
        _known[target.block] = target.text;
        await _markdown(target, '');
      }
      var boundary = _caret(prev);
      var base = target!.caret;
      int place(int field) =>
          (base + field - boundary).clamp(0, target!.text.length);
      while (true) {
        final _Queued? next = _queue.isEmpty ? null : _queue.removeFirst();
        if (next?.left case final left?) {
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
        if (block != null) views.reread(block);
      }
      views.reread(target.block);
    } on Object {
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

  static Caret _caretOf(EditorTarget target) => Caret(
    target.view,
    target.caret,
    text: target.text,
    style: target.style,
    slash: target.slash,
  );

  Future<bool> _handOver(EditorTarget target, TextEditingValue prev) async {
    if (!_gone && !(_source?.focused ?? true)) return true;
    final offered = _caretOf(target);
    views.moveCaret(offered);
    if (views.closed) return true;
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

  Future<EditorTarget> _perform(
    LogicalKeyboardKey key,
    EditorTarget target,
  ) async {
    if (key == LogicalKeyboardKey.backspace) return _backspace(target);
    if (key == LogicalKeyboardKey.delete) {
      if (target.caret >= target.text.length) return target;
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

  Future<EditorTarget> _backspace(EditorTarget target) async {
    if (target.caret > 0) {
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
    if (above == null || above.kind != BridgeBlockKind.text) return target;
    final answer = await _state.read(
      query(BridgeQueryKind.text, page: _page, block: above.block),
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

  Future<BridgeView?> _above(EditorTarget target) async {
    final near = await _state.read(
      query(
        BridgeQueryKind.views,
        page: _page,
        start: target.index > 0 ? target.index - 1 : 0,
        limit: 2,
      ),
    );
    switch (near.views) {
      case [final view, ...] when target.index == 0 && view.id == target.view:
        return null;
      case [final above, final view]
          when target.index > 0 && view.id == target.view:
        return above;
    }
    for (var start = 0; ; start += window) {
      final answer = await _state.read(
        query(BridgeQueryKind.views, page: _page, start: start),
      );
      final at = answer.views.indexWhere((view) => view.id == target.view);
      if (at > 0) return answer.views[at - 1];
      if (at == 0) {
        if (start == 0) return null;
        final before = await _state.read(
          query(BridgeQueryKind.views, page: _page, start: start - 1, limit: 1),
        );
        return before.views.firstOrNull;
      }
      if (answer.views.length < window) return null;
    }
  }

  Future<bool> _markdown(EditorTarget target, String before) async {
    if (await _sectionBreak(target, before)) return true;
    final shortcut = shortcutTyped(
      target.text,
      before,
      target.caret,
      target.style,
    );
    if (shortcut == null) return false;
    final (typed, style) = shortcut;
    await _replace(target, 0, typed.length, '');
    await _restyle(target, style);
    return true;
  }

  Future<bool> _sectionBreak(EditorTarget target, String before) async {
    if (startsSection(
      target.text,
      before,
      index: target.index,
      boundary: target.boundary,
    )) {
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
