import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import 'model.dart';

const delimiters = {'bold': '**', 'italic': '*', 'strike': '~~', 'code': '`'};

typedef Selection = ({Pos base, Pos extent});
typedef Composing = ({int para, int at, String text});

/// A Section edited as one text: one selection over all its Paragraphs, one
/// IME connection, and a lazily built list, so only visible Paragraphs lay out.
class SectionEditor extends StatefulWidget {
  const SectionEditor({super.key, required this.model});

  final SectionModel model;

  @override
  State<SectionEditor> createState() => SectionEditorState();
}

class SectionEditorState extends State<SectionEditor> with TextInputClient implements DeltaTextInputClient {
  SectionModel get model => widget.model;
  final focus = FocusNode();
  final scroll = ScrollController();
  final selection = ValueNotifier<Selection>((base: (para: 0, offset: 0), extent: (para: 0, offset: 0)));

  /// At the end of a styled run the caret has two stops: inside types with
  /// the run's marks, outside without them.
  final inside = ValueNotifier(true);
  final repaint = ValueNotifier(0);
  Map<String, Object>? pending;
  Composing? composing;
  final views = <int, ParagraphViewState>{};
  TextInputConnection? _connection;
  TextEditingValue _ime = TextEditingValue.empty;
  double? _goalX;

  Pos get caret => selection.value.extent;
  bool get collapsed => compare(selection.value.base, selection.value.extent) == 0;
  (Pos, Pos) get ordered {
    final s = selection.value;
    return compare(s.base, s.extent) <= 0 ? (s.base, s.extent) : (s.extent, s.base);
  }

  @override
  void initState() {
    super.initState();
    focus.addListener(_onFocus);
    model.addListener(_onModel);
  }

  @override
  void dispose() {
    model.removeListener(_onModel);
    _connection?.close();
    focus.dispose();
    scroll.dispose();
    super.dispose();
  }

  void _onModel() {
    final c = caret;
    if (c.para >= model.paras.length) {
      final last = model.paras.length - 1;
      select((para: last, offset: model.paras[last].text.length));
    }
    setState(() {});
  }

  void _onFocus() {
    if (focus.hasFocus) {
      _connection ??= TextInput.attach(
        this,
        const TextInputConfiguration(
          enableDeltaModel: true,
          inputType: TextInputType.multiline,
          inputAction: TextInputAction.newline,
          enableSuggestions: false,
          autocorrect: false,
        ),
      );
      _connection!.show();
      _syncIme();
    } else {
      _connection?.close();
      _connection = null;
    }
  }

  Pos _snap(Pos p) {
    final text = model.paras[p.para].text;
    final o = p.offset;
    return o > 0 && o < text.length && _isLow(text.codeUnitAt(o)) && _isHigh(text.codeUnitAt(o - 1))
        ? (para: p.para, offset: o - 1)
        : p;
  }

  void select(Pos base, [Pos? extent, bool keepGoal = false]) {
    selection.value = (base: _snap(base), extent: _snap(extent ?? base));
    pending = null;
    inside.value = true;
    if (!keepGoal) _goalX = null;
    _syncIme();
  }

  bool get _atEndEdge {
    final p = model.paras[caret.para];
    final o = caret.offset;
    if (o == 0) return false;
    final before = p.marksAt(o - 1);
    final after = p.marksAt(o);
    return before.keys.any((k) => growing.contains(k) && !after.containsKey(k));
  }

  Map<String, Object> typingMarks() {
    if (pending != null) return pending!;
    final p = model.paras[caret.para];
    final o = caret.offset;
    final before = o > 0 ? p.marksAt(o - 1) : const <String, Object>{};
    final after = p.marksAt(o);
    return {
      for (final e in before.entries)
        if (growing.contains(e.key) ? (inside.value || after.containsKey(e.key)) : after[e.key] == e.value)
          e.key: e.value,
    };
  }

  // Editing.

  void typeText(String text) {
    var at = caret;
    final marks = typingMarks();
    if (!collapsed) {
      final (a, b) = ordered;
      at = model.delete(a, b);
    }
    model.insert(at, text, marks);
    select((para: at.para, offset: at.offset + text.length));
  }

  void enter({bool soft = false}) {
    if (soft) return typeText('\n');
    var at = caret;
    if (!collapsed) {
      final (a, b) = ordered;
      at = model.delete(a, b);
    }
    final p = model.paras[at.para];
    if (p.text.isEmpty && p.block.type != 'paragraph') {
      model.setBlock(at.para, paragraph);
      return select(at);
    }
    select(model.split(at));
  }

  void backspace() {
    if (!collapsed) {
      final (a, b) = ordered;
      return select(model.delete(a, b));
    }
    final c = caret;
    final p = model.paras[c.para];
    if (c.offset > 0) {
      final from = _previous(p.text, c.offset);
      return select(model.delete((para: c.para, offset: from), c));
    }
    if (p.block.type != 'paragraph') {
      model.setBlock(c.para, paragraph);
      return select(c);
    }
    if (c.para == 0) return;
    final above = (para: c.para - 1, offset: model.paras[c.para - 1].text.length);
    select(model.delete(above, c));
  }

  void deleteForward() {
    if (!collapsed) return backspace();
    final c = caret;
    final p = model.paras[c.para];
    if (c.offset < p.text.length) {
      select(model.delete(c, (para: c.para, offset: _next(p.text, c.offset))));
    } else if (c.para + 1 < model.paras.length) {
      select(model.delete(c, (para: c.para + 1, offset: 0)));
    }
  }

  void toggle(String name) {
    if (collapsed) {
      final marks = Map.of(typingMarks());
      if (marks.remove(name) == null) marks[name] = true;
      pending = marks;
      repaint.value++;
      return;
    }
    final (a, b) = ordered;
    model.toggle(a, b, name);
  }

  void clearMarks() {
    if (collapsed) {
      pending = {};
      return;
    }
    final (a, b) = ordered;
    for (final name in [...delimiters.keys, 'link']) {
      model.toggle(a, b, name);
      model.toggle(a, b, name);
    }
  }

  static int _previous(String text, int offset) =>
      offset >= 2 && _isLow(text.codeUnitAt(offset - 1)) && _isHigh(text.codeUnitAt(offset - 2))
      ? offset - 2
      : offset - 1;
  static int _next(String text, int offset) =>
      offset + 1 < text.length && _isHigh(text.codeUnitAt(offset)) && _isLow(text.codeUnitAt(offset + 1))
      ? offset + 2
      : offset + 1;
  static bool _isHigh(int u) => u >= 0xD800 && u <= 0xDBFF;
  static bool _isLow(int u) => u >= 0xDC00 && u <= 0xDFFF;

  // Moving.

  Pos _step(Pos p, int by) {
    final text = model.paras[p.para].text;
    if (by < 0) {
      if (p.offset > 0) return (para: p.para, offset: _previous(text, p.offset));
      if (p.para > 0) return (para: p.para - 1, offset: model.paras[p.para - 1].text.length);
      return p;
    }
    if (p.offset < text.length) return (para: p.para, offset: _next(text, p.offset));
    if (p.para + 1 < model.paras.length) return (para: p.para + 1, offset: 0);
    return p;
  }

  void move(int by, {bool extend = false}) {
    if (!extend && collapsed && _atEndEdge) {
      if (by > 0 && inside.value) {
        inside.value = false;
        return;
      }
      if (by < 0 && !inside.value) {
        inside.value = true;
        return;
      }
    }
    if (!extend && !collapsed) {
      final (a, b) = ordered;
      return select(by < 0 ? a : b);
    }
    final next = _step(caret, by);
    if (extend) {
      select(selection.value.base, next);
    } else {
      select(next);
      if (by < 0 && _atEndEdge) inside.value = false;
    }
  }

  void moveLine(int by, {bool extend = false}) {
    final view = views[caret.para];
    final rp = view?.renderParagraph;
    Pos? target;
    if (view != null && rp != null) {
      final rendered = view.toRendered(caret.offset, inside.value);
      final rect = rp.getOffsetForCaret(TextPosition(offset: rendered), Rect.zero);
      final line = rp.getFullHeightForCaret(TextPosition(offset: rendered));
      _goalX ??= rect.dx;
      final y = rect.dy + line / 2 + by * line;
      if (y >= 0 && y < rp.size.height) {
        target = (para: caret.para, offset: view.toModel(rp.getPositionForOffset(Offset(_goalX!, y)).offset).$1);
      } else {
        final index = caret.para + by;
        if (index >= 0 && index < model.paras.length) {
          final neighbour = views[index]?.renderParagraph;
          final offset = neighbour == null
              ? (by < 0 ? model.paras[index].text.length : 0)
              : views[index]!
                    .toModel(
                      neighbour.getPositionForOffset(Offset(_goalX!, by < 0 ? neighbour.size.height - 1 : 1)).offset,
                    )
                    .$1;
          target = (para: index, offset: offset);
        }
      }
    }
    if (target == null) return;
    extend ? select(selection.value.base, target, true) : select(target, null, true);
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent || composing != null) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    final meta = keys.isMetaPressed || keys.isControlPressed;
    final shift = keys.isShiftPressed;
    final key = event.logicalKey;
    if (meta) {
      switch (key) {
        case LogicalKeyboardKey.keyA:
          final last = model.paras.length - 1;
          select((para: 0, offset: 0), (para: last, offset: model.paras[last].text.length));
        case LogicalKeyboardKey.keyB:
          toggle('bold');
        case LogicalKeyboardKey.keyI:
          toggle('italic');
        case LogicalKeyboardKey.keyE:
          toggle('code');
        case LogicalKeyboardKey.backslash:
          clearMarks();
        default:
          return KeyEventResult.ignored;
      }
      return KeyEventResult.handled;
    }
    switch (key) {
      case LogicalKeyboardKey.arrowLeft:
        move(-1, extend: shift);
      case LogicalKeyboardKey.arrowRight:
        move(1, extend: shift);
      case LogicalKeyboardKey.arrowUp:
        moveLine(-1, extend: shift);
      case LogicalKeyboardKey.arrowDown:
        moveLine(1, extend: shift);
      case LogicalKeyboardKey.backspace:
        backspace();
      case LogicalKeyboardKey.delete:
        deleteForward();
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        enter(soft: shift);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  // Pointer.

  (Pos, bool)? hit(Offset global) {
    ParagraphViewState? nearest;
    var distance = double.infinity;
    for (final view in views.values) {
      final box = view.context.findRenderObject() as RenderBox?;
      if (box == null || !box.attached) continue;
      final local = box.globalToLocal(global);
      final d = local.dy < 0 ? -local.dy : (local.dy > box.size.height ? local.dy - box.size.height : 0.0);
      if (d < distance) {
        distance = d;
        nearest = view;
      }
    }
    final rp = nearest?.renderParagraph;
    if (nearest == null || rp == null) return null;
    final (offset, inner) = nearest.toModel(rp.getPositionForOffset(rp.globalToLocal(global)).offset);
    return ((para: nearest.widget.index, offset: offset), inner);
  }

  void _down(PointerDownEvent event) {
    focus.requestFocus();
    final at = hit(event.position);
    if (at == null) return;
    HardwareKeyboard.instance.isShiftPressed ? select(selection.value.base, at.$1) : select(at.$1);
    inside.value = at.$2;
  }

  void _drag(PointerMoveEvent event) {
    final at = hit(event.position);
    if (at != null) select(selection.value.base, at.$1);
  }

  // IME.

  void _syncIme() {
    final c = caret;
    final p = model.paras[c.para];
    final s = selection.value;
    final within = s.base.para == c.para;
    _ime = TextEditingValue(
      text: p.text,
      selection: TextSelection(baseOffset: within ? s.base.offset : c.offset, extentOffset: c.offset),
    );
    _connection?.setEditingState(_ime);
    _placeIme();
  }

  /// Tells the IME where the caret and composing text are, so its candidate
  /// window opens next to them.
  void _placeIme() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final connection = _connection;
      final view = views[caret.para];
      final rp = view?.renderParagraph;
      if (connection == null || !connection.attached || view == null || rp == null || !rp.attached) return;
      connection.setEditableSizeAndTransform(rp.size, rp.getTransformTo(null));
      final c = composing;
      final offset = c != null ? c.at + c.text.length : view.toRendered(caret.offset, inside.value);
      final at = rp.getOffsetForCaret(TextPosition(offset: offset), Rect.zero);
      final height = rp.getFullHeightForCaret(TextPosition(offset: offset));
      connection.setCaretRect(Rect.fromLTWH(at.dx, at.dy, 2, height));
      if (c != null) {
        final boxes = rp.getBoxesForSelection(TextSelection(baseOffset: c.at, extentOffset: c.at + c.text.length));
        if (boxes.isNotEmpty) {
          connection.setComposingRect(boxes.map((b) => b.toRect()).reduce((a, b) => a.expandToInclude(b)));
        }
      }
    });
  }

  @override
  TextEditingValue? get currentTextEditingValue => _ime;

  @override
  AutofillScope? get currentAutofillScope => null;

  @override
  void updateEditingValue(TextEditingValue value) {}

  @override
  void updateEditingValueWithDeltas(List<TextEditingDelta> deltas) {
    for (final delta in deltas) {
      final before = _ime;
      final after = delta.apply(before);
      if (composing == null && after.composing.isCollapsed) {
        _plain(delta, after);
      } else {
        _composed(before, after);
      }
    }
  }

  void _plain(TextEditingDelta delta, TextEditingValue after) {
    final c = caret.para;
    switch (delta) {
      case TextEditingDeltaInsertion(:final textInserted, :final insertionOffset):
        if (collapsed && caret.offset != insertionOffset) select((para: c, offset: insertionOffset));
        typeText(textInserted);
      case TextEditingDeltaDeletion(:final deletedRange):
        select((para: c, offset: deletedRange.start), (para: c, offset: deletedRange.end));
        backspace();
      case TextEditingDeltaReplacement(:final replacedRange, :final replacementText):
        select((para: c, offset: replacedRange.start), (para: c, offset: replacedRange.end));
        typeText(replacementText);
      case TextEditingDeltaNonTextUpdate():
        select((para: c, offset: after.selection.baseOffset), (para: c, offset: after.selection.extentOffset));
    }
    _ime = after.copyWith(text: model.paras[caret.para].text);
  }

  void _composed(TextEditingValue before, TextEditingValue after) {
    final c = caret.para;
    if (!collapsed && composing == null) {
      final (a, b) = ordered;
      select(model.delete(a, b));
    }
    final base = model.paras[c].text;
    if (!after.composing.isCollapsed) {
      final at = after.composing.start;
      composing = (para: c, at: at, text: after.composing.textInside(after.text));
      _ime = after;
      repaint.value++;
      _placeIme();
      return;
    }
    final start = composing?.at ?? after.selection.baseOffset;
    final added = after.text.length - base.length;
    composing = null;
    if (added > 0) {
      select((para: c, offset: start));
      typeText(after.text.substring(start, start + added));
    } else {
      repaint.value++;
    }
    _syncIme();
  }

  @override
  void performAction(TextInputAction action) {
    if (action == TextInputAction.newline) enter();
  }

  @override
  void performSelector(String selectorName) {
    switch (selectorName) {
      case 'insertNewline:':
        enter();
      case 'deleteBackward:':
        backspace();
    }
  }

  @override
  void performPrivateCommand(String action, Map<String, dynamic> data) {}

  @override
  void updateFloatingCursor(RawFloatingCursorPoint point) {}

  @override
  void showAutocorrectionPromptRect(int start, int end) {}

  @override
  void connectionClosed() => _connection = null;

  @override
  Widget build(BuildContext context) => Focus(
    focusNode: focus,
    onKeyEvent: _onKey,
    child: Listener(
      onPointerDown: _down,
      onPointerMove: _drag,
      child: ListView.builder(
        controller: scroll,
        padding: const EdgeInsets.all(24),
        itemCount: model.paras.length,
        itemBuilder: (context, index) =>
            ParagraphView(key: ObjectKey(model.paras[index]), index: index, para: model.paras[index], editor: this),
      ),
    ),
  );
}

class ParagraphView extends StatefulWidget {
  const ParagraphView({super.key, required this.index, required this.para, required this.editor});

  final int index;
  final Para para;
  final SectionEditorState editor;

  @override
  State<ParagraphView> createState() => ParagraphViewState();
}

class ParagraphViewState extends State<ParagraphView> {
  final _text = GlobalKey();

  /// Characters shown that are not in the Paragraph: revealed Markdown and
  /// composing text, as (model offset, length, closes a run).
  List<(int, int, bool)> _shown = const [];

  RenderParagraph? get renderParagraph {
    final r = _text.currentContext?.findRenderObject();
    return r is RenderParagraph && r.hasSize ? r : null;
  }

  @override
  void initState() {
    super.initState();
    widget.editor.views[widget.index] = this;
  }

  @override
  void didUpdateWidget(ParagraphView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.editor.views[oldWidget.index] == this) widget.editor.views.remove(oldWidget.index);
    widget.editor.views[widget.index] = this;
  }

  @override
  void dispose() {
    if (widget.editor.views[widget.index] == this) widget.editor.views.remove(widget.index);
    super.dispose();
  }

  int toRendered(int offset, bool inside) {
    var r = offset;
    for (final (at, length, closes) in _shown) {
      if (at < offset || (at == offset && closes && !inside)) r += length;
    }
    return r;
  }

  (int, bool) toModel(int rendered) {
    var shift = 0;
    for (final (at, length, closes) in _shown) {
      final start = at + shift;
      if (rendered <= start) break;
      if (rendered < start + length) return (at, closes ? rendered - start < length / 2 : false);
      if (rendered == start + length && closes) return (at, false);
      shift += length;
    }
    return (rendered - shift, true);
  }

  TextSpan _span(BuildContext context) {
    final editor = widget.editor;
    final p = widget.para;
    final theme = Theme.of(context);
    final base = TextStyle(
      fontSize: switch (p.block.type) {
        'heading1' => 30,
        'heading2' => 24,
        'heading3' => 20,
        _ => 16,
      },
      height: 1.5,
      fontWeight: p.block.type.startsWith('heading') ? FontWeight.w600 : FontWeight.w400,
      color: theme.colorScheme.onSurface,
    );
    final faint = base.copyWith(color: theme.colorScheme.onSurface.withValues(alpha: 0.4), fontWeight: FontWeight.w400);
    final caret = editor.caret;
    final mine = caret.para == widget.index && editor.collapsed;
    final insert = <int, List<(String, TextStyle, bool)>>{};
    final shown = <(int, int, bool)>[];
    final composing = editor.composing;
    if (composing != null && composing.para == widget.index) {
      insert.putIfAbsent(composing.at, () => []).add((
        composing.text,
        base.copyWith(decoration: TextDecoration.underline),
        false,
      ));
      shown.add((composing.at, composing.text.length, false));
    } else if (mine) {
      for (final name in delimiters.keys) {
        for (final m in _runs(p, name)) {
          if (m.$1 <= caret.offset && caret.offset <= m.$2) {
            final d = delimiters[name]!;
            insert.putIfAbsent(m.$1, () => []).add((d, faint, false));
            insert.putIfAbsent(m.$2, () => []).insert(0, (d, faint, true));
            shown.add((m.$1, d.length, false));
            shown.add((m.$2, d.length, true));
          }
        }
      }
    }
    shown.sort((a, b) => a.$1 != b.$1 ? a.$1 - b.$1 : (a.$3 ? 0 : 1) - (b.$3 ? 0 : 1));
    _shown = shown;
    final children = <InlineSpan>[];
    void put(int at) {
      for (final (text, style, _) in insert[at] ?? const <(String, TextStyle, bool)>[]) {
        children.add(TextSpan(text: text, style: style));
      }
    }

    var o = 0;
    while (o < p.text.length) {
      put(o);
      final marks = p.marksAt(o);
      var end = o + 1;
      while (end < p.text.length && !insert.containsKey(end) && mapEqualsMarks(p.marksAt(end), marks)) {
        end++;
      }
      children.add(TextSpan(text: p.text.substring(o, end), style: _style(base, marks, theme)));
      o = end;
    }
    put(p.text.length);
    if (p.text.isEmpty && children.isEmpty) children.add(TextSpan(text: '​', style: base));
    return TextSpan(style: base, children: children);
  }

  static bool mapEqualsMarks(Map<String, Object> a, Map<String, Object> b) =>
      a.length == b.length && a.entries.every((e) => b[e.key] == e.value);

  static List<(int, int)> _runs(Para p, String name) {
    final out = <(int, int)>[];
    var start = -1;
    for (var o = 0; o <= p.text.length; o++) {
      final has = o < p.text.length && p.marksAt(o).containsKey(name);
      if (has && start < 0) start = o;
      if (!has && start >= 0) {
        out.add((start, o));
        start = -1;
      }
    }
    return out;
  }

  TextStyle _style(TextStyle base, Map<String, Object> marks, ThemeData theme) => base.copyWith(
    fontWeight: marks.containsKey('bold') ? FontWeight.w700 : null,
    fontStyle: marks.containsKey('italic') ? FontStyle.italic : null,
    decoration: TextDecoration.combine([
      if (marks.containsKey('strike')) TextDecoration.lineThrough,
      if (marks.containsKey('link')) TextDecoration.underline,
    ]),
    color: marks.containsKey('link') ? theme.colorScheme.primary : null,
    fontFamily: marks.containsKey('code') ? 'Menlo' : null,
    backgroundColor: marks.containsKey('code') ? theme.colorScheme.surfaceContainerHighest : null,
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      widget.para.version,
      widget.editor.selection,
      widget.editor.inside,
      widget.editor.repaint,
    ]),
    builder: (context, _) {
      final p = widget.para;
      final text = CustomPaint(
        painter: _SelectionPainter(this),
        foregroundPainter: _CaretPainter(this),
        child: RichText(key: _text, text: _span(context)),
      );
      final marker = switch (p.block.type) {
        'bullet' => const Text('•  '),
        'numbered' => const Text('1. '),
        'todo' => Icon(p.block.done ? Icons.check_box : Icons.check_box_outline_blank, size: 18),
        _ => null,
      };
      return Padding(
        padding: EdgeInsets.only(
          top: p.block.type.startsWith('heading') ? 12 : 2,
          bottom: 2,
          left: 24.0 * p.block.indent,
        ),
        child: marker == null
            ? text
            : Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  marker,
                  Expanded(child: text),
                ],
              ),
      );
    },
  );
}

class _SelectionPainter extends CustomPainter {
  _SelectionPainter(this.view);

  final ParagraphViewState view;

  @override
  void paint(Canvas canvas, Size size) {
    final editor = view.widget.editor;
    if (editor.collapsed) return;
    final rp = view.renderParagraph;
    if (rp == null) return;
    final (a, b) = editor.ordered;
    final i = view.widget.index;
    if (i < a.para || i > b.para) return;
    final from = view.toRendered(i == a.para ? a.offset : 0, true);
    final to = i == b.para ? view.toRendered(b.offset, true) : view.toRendered(view.widget.para.text.length, true) + 1;
    final paint = Paint()..color = const Color(0x553D9BFF);
    for (final box in rp.getBoxesForSelection(TextSelection(baseOffset: from, extentOffset: to))) {
      canvas.drawRect(box.toRect(), paint);
    }
  }

  @override
  bool shouldRepaint(_SelectionPainter old) => true;
}

class _CaretPainter extends CustomPainter {
  _CaretPainter(this.view);

  final ParagraphViewState view;

  @override
  void paint(Canvas canvas, Size size) {
    final editor = view.widget.editor;
    if (editor.caret.para != view.widget.index || !editor.focus.hasFocus) return;
    final rp = view.renderParagraph;
    if (rp == null) return;
    final composing = editor.composing;
    final offset = composing != null && composing.para == view.widget.index
        ? composing.at + composing.text.length
        : view.toRendered(editor.caret.offset, editor.inside.value);
    final position = TextPosition(offset: offset);
    final at = rp.getOffsetForCaret(position, Rect.zero);
    final height = rp.getFullHeightForCaret(position);
    canvas.drawRect(Rect.fromLTWH(at.dx, at.dy, 2, height), Paint()..color = const Color(0xFF3D9BFF));
  }

  @override
  bool shouldRepaint(_CaretPainter old) => true;
}
