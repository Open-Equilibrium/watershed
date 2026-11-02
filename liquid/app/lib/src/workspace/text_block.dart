import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'follow.dart';
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

bool _listed(BridgeTextStyle style) =>
    nextStyle(style) != BridgeTextStyle.paragraph;

/// The one edit that turns `from` into `to` in `block`: the changed middle, never between the
/// halves of a surrogate pair, which the core refuses.
BridgeMutation textEdit(String page, String block, String from, String to) {
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
  return mutation(
    BridgeMutationKind.editText,
    page: page,
    block: block,
    at: start,
    delete: from.length - start - end,
    text: to.substring(start, to.length - end),
  );
}

/// One Text Block on its Page, with the divider above it when it starts a Section. It
/// follows only its own Block (client architecture rule 6); typing shows at once and
/// reaches the core afterwards (rule 7).
class TextBlock extends StatefulWidget {
  const TextBlock({
    super.key,
    required this.views,
    required this.view,
    required this.index,
  });

  final PageViews views;
  final BridgeView view;

  /// Its position on the Page.
  final int index;

  @override
  State<TextBlock> createState() => _TextBlockState();
}

class _TextBlockState extends State<TextBlock> {
  final _text = TextEditingController();

  /// Keeps the field, its text input included, when a new style wraps it differently.
  final _field = GlobalKey(debugLabel: 'Text Block field');
  late final _focus = FocusNode(
    debugLabel: 'Text Block',
    onKeyEvent: (_, event) => _key(event),
  );

  /// The text the core holds as far as this Block knows, its own edits included.
  String? _stored;
  BridgeTextStyle _style = BridgeTextStyle.paragraph;

  /// Edits sent but not yet applied, and all sent so far, so a read that crossed one is
  /// not shown.
  int _pending = 0;
  int _sent = 0;

  /// While the text changes from the core or a key, not from typing.
  bool _quiet = false;

  /// While ⏎'s split is on its way: what is typed meanwhile goes to the new Text Block.
  bool _splitting = false;

  /// Reads asked for, so only the latest shows.
  int _reads = 0;
  StreamSubscription<BridgeChange>? _followed;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    widget.views.caret.addListener(_takeCaret);
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
  }

  @override
  void dispose() {
    unawaited(_followed?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// Shows the stored text and style, unless an edit of this Block is on its way.
  Future<void> _read() async {
    final read = ++_reads;
    // A read asked while an edit is on its way may miss it, whenever it answers.
    final clean = _pending == 0;
    final sent = _sent;
    final BridgeAnswer answer;
    try {
      answer = await _state.workspace.query(
        query: query(BridgeQueryKind.text, page: _page, block: _block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    if (!mounted || read != _reads) return;
    final style = answer.style ?? BridgeTextStyle.paragraph;
    widget.views.setStyle(_block, style);
    if (style == BridgeTextStyle.numbered) {
      unawaited(widget.views.readRun(widget.index).catchError(_report));
    }
    if (style != _style) setState(() => _style = style);
    final text = answer.text;
    if (text == null || !clean || _pending > 0 || sent != _sent) return;
    // Editable once the text is known, so nothing typed before is lost.
    if (_stored == null) setState(() {});
    _stored = text;
    if (_text.text != text && !_composing) {
      final offset = _text.selection.baseOffset.clamp(0, text.length);
      _set(text, offset);
    }
    _takeCaret();
  }

  bool get _composing => !_text.value.composing.isCollapsed;

  void _set(String text, int offset) {
    _quiet = true;
    _text.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: offset),
    );
    _quiet = false;
  }

  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null || caret.view != widget.view.id || _stored == null) {
      return;
    }
    widget.views.caret.value = null;
    _focus.requestFocus();
    final offset = caret.offset.clamp(0, _text.text.length);
    if (caret.insert.isEmpty) {
      _text.selection = TextSelection.collapsed(offset: offset);
      return;
    }
    final text = _text.text.replaceRange(offset, offset, caret.insert);
    _set(text, offset + caret.insert.length);
    _send(text);
  }

  /// Sends what typing changed, or applies a shortcut typed at the start.
  void _typed() {
    final stored = _stored;
    if (_quiet || _splitting || stored == null || _composing) return;
    final text = _text.text;
    if (text == stored) return;
    if (_style == BridgeTextStyle.paragraph) {
      for (final MapEntry(key: typed, value: style)
          in markdownShortcuts.entries) {
        if (text.startsWith(typed) &&
            !stored.startsWith(typed) &&
            _text.selection.baseOffset == typed.length) {
          final rest = text.substring(typed.length);
          _set(rest, 0);
          _send(rest);
          _restyle(style);
          return;
        }
      }
    }
    // A boundary already exists at the Page's start (approved Section mockups).
    if (text == sectionBreak && widget.index > 0 && !widget.view.boundary) {
      _set('', 0);
      _send('');
      _apply(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: widget.view.id,
          boundary: true,
        ),
      );
      return;
    }
    _send(text);
  }

  /// Sends the change from the stored text to `text` as one edit.
  void _send(String text) {
    final stored = _stored!;
    if (text == stored) return;
    _stored = text;
    _apply(textEdit(_page, _block, stored, text));
  }

  /// Applies `change` after earlier ones; a refusal shows the stored text again (rule 7).
  Future<BridgeApplied?> _apply(BridgeMutation change) async {
    _pending++;
    _sent++;
    try {
      return await _state.apply(change);
    } on Object {
      return null;
    } finally {
      _pending--;
      if (mounted && _pending == 0) unawaited(_read());
    }
  }

  void _restyle(BridgeTextStyle style) {
    setState(() => _style = style);
    widget.views.setStyle(_block, style);
    _apply(
      mutation(
        BridgeMutationKind.setTextStyle,
        page: _page,
        block: _block,
        style: style,
      ),
    );
  }

  KeyEventResult _key(KeyEvent event) {
    if (event is KeyUpEvent || _composing || _stored == null) {
      return KeyEventResult.ignored;
    }
    // A second ⏎ before the first split arrived is dropped rather than typed as a line.
    if (_splitting) {
      return event.logicalKey == LogicalKeyboardKey.enter
          ? KeyEventResult.handled
          : KeyEventResult.ignored;
    }
    final keys = HardwareKeyboard.instance;
    final plain =
        !keys.isShiftPressed &&
        !keys.isMetaPressed &&
        !keys.isControlPressed &&
        !keys.isAltPressed;
    final selection = _text.selection;
    if (!plain || !selection.isValid) return KeyEventResult.ignored;
    final at = selection.baseOffset;
    final collapsed = selection.isCollapsed;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        _enter();
        return KeyEventResult.handled;
      case LogicalKeyboardKey.backspace when collapsed && at == 0:
        unawaited(_backspace());
        return KeyEventResult.handled;
      case LogicalKeyboardKey.arrowUp when collapsed && _onLine(first: true):
      case LogicalKeyboardKey.arrowLeft when collapsed && at == 0:
        return _move(-1) ? KeyEventResult.handled : KeyEventResult.ignored;
      case LogicalKeyboardKey.arrowDown when collapsed && _onLine(first: false):
      case LogicalKeyboardKey.arrowRight
          when collapsed && at == _text.text.length:
        return _move(1) ? KeyEventResult.handled : KeyEventResult.ignored;
      default:
        return KeyEventResult.ignored;
    }
  }

  /// Whether the caret is on the text's first or last visual line.
  bool _onLine({required bool first}) {
    RenderEditable? editable;
    void find(RenderObject object) {
      if (object is RenderEditable) {
        editable = object;
      } else {
        object.visitChildren(find);
      }
    }

    final root = _focus.context?.findRenderObject();
    if (root == null) return false;
    find(root);
    final found = editable;
    if (found == null) return false;
    final caret = found
        .getLocalRectForCaret(TextPosition(offset: _text.selection.baseOffset))
        .center
        .dy;
    final edge = found
        .getLocalRectForCaret(
          TextPosition(offset: first ? 0 : _text.text.length),
        )
        .center
        .dy;
    return (caret - edge).abs() < 1;
  }

  /// Puts the caret into the neighbouring Text Block: at the end above, at the start below.
  bool _move(int by) {
    final neighbour = widget.views.at(widget.index + by);
    if (neighbour == null) return false;
    widget.views.caret.value = Caret(neighbour.id, by < 0 ? 1 << 30 : 0);
    return true;
  }

  /// ⏎ ends this Text Block and starts the next at the caret, as in Notion. An empty list
  /// item or to-do becomes a paragraph instead.
  void _enter() {
    final selection = _text.selection;
    var text = _text.text;
    if (!selection.isCollapsed) {
      text = text.replaceRange(selection.start, selection.end, '');
      _set(text, selection.start);
      _send(text);
    }
    final at = selection.start;
    if (text.isEmpty && _listed(_style)) {
      _restyle(BridgeTextStyle.paragraph);
      return;
    }
    final split = _apply(
      mutation(
        BridgeMutationKind.splitTextBlock,
        page: _page,
        block: _block,
        view: widget.view.id,
        at: at,
        style: nextStyle(_style),
      ),
    );
    // At the start the text stays here, below a new empty Text Block, and typing goes on
    // here.
    if (at == 0) return;
    final head = text.substring(0, at);
    _stored = head;
    _set(head, at);
    _splitting = true;
    unawaited(
      split.then((applied) {
        _splitting = false;
        if (!mounted) return;
        final typed = _text.text;
        final extra = typed.startsWith(head)
            ? typed.substring(head.length)
            : '';
        if (typed != head) _set(head, head.length);
        if (applied?.view case final view?) {
          widget.views.caret.value = Caret(view, 0, insert: extra);
        }
      }),
    );
  }

  /// ⌫ at the start removes a divider above, then the formatting, then joins this Text
  /// Block to the one above, as the approved Section mockups and Notion do.
  Future<void> _backspace() async {
    if (widget.view.boundary && widget.index > 0) {
      await _apply(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: widget.view.id,
          boundary: false,
        ),
      );
      return;
    }
    if (_style != BridgeTextStyle.paragraph) {
      _restyle(BridgeTextStyle.paragraph);
      return;
    }
    final above = widget.index > 0 ? widget.views.at(widget.index - 1) : null;
    if (above == null) return;
    await _state.settled;
    final BridgeAnswer answer;
    try {
      answer = await _state.workspace.query(
        query: query(BridgeQueryKind.text, page: _page, block: above.block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    final joined = await _apply(
      mutation(
        BridgeMutationKind.joinTextBlock,
        page: _page,
        view: widget.view.id,
      ),
    );
    if (joined != null) {
      widget.views.caret.value = Caret(above.id, answer.text?.length ?? 0);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (size, line, weight, top) = switch (_style) {
      BridgeTextStyle.heading1 => (
        WsMetrics.fsH1,
        WsMetrics.lhH1,
        WsMetrics.fwSemibold,
        WsMetrics.space5,
      ),
      BridgeTextStyle.heading2 => (
        WsMetrics.fsH2,
        WsMetrics.lhH2,
        WsMetrics.fwSemibold,
        WsMetrics.space4,
      ),
      BridgeTextStyle.heading3 => (
        WsMetrics.fsH3,
        WsMetrics.lhH3,
        WsMetrics.fwSemibold,
        WsMetrics.space3,
      ),
      _ => (WsMetrics.fsProse, WsMetrics.lhProse, WsMetrics.fwRegular, 0.0),
    };
    final done = _style == BridgeTextStyle.done;
    Widget text = TextField(
      key: _field,
      controller: _text,
      readOnly: _stored == null,
      focusNode: _focus,
      maxLines: null,
      keyboardType: TextInputType.multiline,
      style: TextStyle(
        fontSize: size,
        height: line / size,
        fontWeight: weight,
        color: done ? ws.textFaint : ws.text,
        decoration: done ? TextDecoration.lineThrough : null,
      ),
      decoration: const InputDecoration.collapsed(hintText: null),
    );
    Widget marker(Widget child) => SizedBox(
      width: 24,
      height: line,
      child: Align(alignment: Alignment.centerLeft, child: child),
    );
    text = switch (_style) {
      BridgeTextStyle.bullet => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          marker(Text('•', style: TextStyle(fontSize: size))),
          Expanded(child: text),
        ],
      ),
      BridgeTextStyle.numbered => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          marker(
            ValueListenableBuilder(
              valueListenable: widget.views.styles,
              builder: (context, styles, _) =>
                  Text('${_number(styles)}.', style: TextStyle(fontSize: size)),
            ),
          ),
          Expanded(child: text),
        ],
      ),
      BridgeTextStyle.todo || BridgeTextStyle.done => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          marker(
            Semantics(
              checked: done,
              label: 'Done',
              child: GestureDetector(
                onTap: () => _restyle(
                  done ? BridgeTextStyle.todo : BridgeTextStyle.done,
                ),
                child: Container(
                  key: const Key('to-do'),
                  width: 16,
                  height: 16,
                  decoration: BoxDecoration(
                    color: done ? ws.accent : null,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                    border: done ? null : Border.all(color: ws.borderStrong),
                  ),
                  child: done
                      ? Icon(LucideIcons.check, size: 12, color: ws.onAccent)
                      : null,
                ),
              ),
            ),
          ),
          Expanded(child: text),
        ],
      ),
      BridgeTextStyle.quote => Container(
        padding: const EdgeInsets.only(left: 14),
        decoration: BoxDecoration(
          border: Border(left: BorderSide(color: ws.text, width: 3)),
        ),
        child: text,
      ),
      _ => text,
    };
    return Padding(
      padding: EdgeInsets.only(top: top, bottom: WsMetrics.space1),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The divider `---` leaves; tile edges show it in Arrange mode instead.
          if (widget.view.boundary && widget.index > 0)
            Container(
              key: const Key('section-divider'),
              height: 1,
              margin: const EdgeInsets.symmetric(vertical: WsMetrics.space4),
              color: ws.border,
            ),
          text,
        ],
      ),
    );
  }

  /// Its number in the run of numbered items it ends.
  int _number(Map<String, BridgeTextStyle> styles) {
    if (widget.view.boundary) return 1;
    var number = 1;
    for (var index = widget.index - 1; index >= 0; index--) {
      final above = widget.views.at(index);
      if (above == null || styles[above.block] != BridgeTextStyle.numbered) {
        break;
      }
      number++;
      // A run ends at its Section's start.
      if (above.boundary) break;
    }
    return number;
  }
}
