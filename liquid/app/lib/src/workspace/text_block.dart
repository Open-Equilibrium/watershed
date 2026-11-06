import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'block_menu.dart';
import 'follow.dart';
import 'page_editor.dart';
import 'page_views.dart';
import 'workspace_state.dart';

/// A caret offset that means a Text Block's end.
const _end = 1 << 30;

/// One Text Block on its Page, with the divider above it when it starts a Section. It
/// follows only its own Block (client architecture rule 6); typing shows at once and
/// reaches the core afterwards (rule 7). ⏎ and ⌫ at its start go through the Page's input
/// queue (ADR-0240).
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

class _TextBlockState extends State<TextBlock> implements EditorSource {
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

  /// While the text changes from the core or a key, not from typing.
  bool _quiet = false;

  /// Reads asked for, so only the latest shows.
  int _reads = 0;

  /// Whether the field was last built editable, which it is once its text is known.
  bool _editable = false;

  /// A caret place past the text shown, which a write on its way makes room for.
  int? _wanted;
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _settled;

  /// The Block menu `/` opens, its choices for what follows `/` and the one ⏎ takes.
  final _menu = OverlayPortalController();
  final _menuAnchor = LayerLink();
  var _choices = const <BlockChoice>[];
  var _chosen = 0;

  /// While a Table chosen in the Block menu is added in this line's place.
  var _adding = false;

  WorkspaceState get _state => widget.views.state;
  PageEditor get _editor => widget.views.editor;
  String get _page => widget.views.page;

  @override
  String get block => widget.view.block;

  @override
  TextEditingValue get typed => _text.value;

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    _state.finishing.add(_typed);
    widget.views.caret.addListener(_takeCaret);
    widget.views.ready.addListener(_readied);
    _focus.addListener(_focusChanged);
    _followed = follow(
      _state.workspace,
      BridgeResource(kind: BridgeResourceKind.block, page: _page, block: block),
    ).listen((_) => unawaited(_read()), onError: _report);
    // Built after the caret was handed to it.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _takeCaret();
    });
    // Once every write to this Block arrived, its text is read again.
    _settled = widget.views.settled
        .where((settled) => settled == block)
        .listen((_) => unawaited(_read()));
  }

  @override
  void dispose() {
    // A composition still open, or anything else not yet sent, is written as it stands.
    if (!_editor.owns(this) &&
        !_adding &&
        _stored != null &&
        _text.text != _stored) {
      _send(_text.text);
    }
    _editor.left(this);
    _state.finishing.remove(_typed);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    widget.views.caret.removeListener(_takeCaret);
    widget.views.ready.removeListener(_readied);
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// The Page's earlier queue finished: what it wrote shows, and the field takes keys.
  void _readied() {
    if (!mounted) return;
    setState(() {});
    unawaited(_read());
  }

  @override
  bool get focused => _focus.hasFocus;

  void _focusChanged() {
    if (!_focus.hasFocus) _closeMenu();
  }

  void _closeMenu() {
    if (_menu.isShowing) _menu.hide();
  }

  /// Opens the Block menu once `/` is typed in an empty Text Block, filters it by what
  /// follows, and closes it when nothing matches or the `/` is gone.
  void _offer(String text, String stored) {
    final opening = text == '/' && stored.isEmpty;
    if (!opening && !_menu.isShowing) return;
    final choices = text.startsWith('/')
        ? blockChoicesFor(text.substring(1))
        : const <BlockChoice>[];
    if (choices.isEmpty) return _closeMenu();
    setState(() {
      _choices = choices;
      _chosen = 0;
    });
    _menu.show();
  }

  /// Applies `choice` in place of what was typed for it: a style, or a Table in this line's
  /// place, starting the Section the line started. A line that ends the Page stays, empty,
  /// below the Table, so the Page goes on.
  Future<void> _choose(BlockChoice choice) async {
    _closeMenu();
    _set('', 0);
    _send('');
    if (choice.style case final style?) {
      if (style != _style) _restyle(style);
      return;
    }
    final views = widget.views;
    final view = widget.view;
    final last = widget.index == (views.count ?? 0) - 1;
    // What is typed meanwhile stays in the field and becomes the Table's name.
    _adding = true;
    final BridgeApplied added;
    try {
      // The last line keeps its place below the Table, which goes after the View above,
      // read first if its window was not; only the Page's first line has none.
      String? after = view.id;
      if (last) {
        final above = widget.index > 0
            ? await views.viewAt(widget.index - 1)
            : null;
        if (widget.index > 0 && above == null) {
          throw 'The Page changed; choose Table again.';
        }
        after = above?.id;
      }
      added = await views.write(
        mutation(
          BridgeMutationKind.addDatabaseBlock,
          page: _page,
          after: after,
          text: '',
        ),
        const [],
      );
    } on Object catch (error) {
      // The problem shows on the Page; what was typed stays in the line.
      _report(error);
      if (mounted) {
        _adding = false;
        _typed();
      }
      return;
    }
    // Once the Table exists it takes the name, and the caret unless the person moved to
    // another field meanwhile, also if a later write is refused, whose problem shows on
    // the Page.
    var typed = '';
    var stayed = true;
    try {
      Future<void> boundary(String view, bool starts) => views.write(
        mutation(
          BridgeMutationKind.setSectionBoundary,
          page: _page,
          view: view,
          boundary: starts,
        ),
        const [],
      );
      if (view.boundary) {
        await boundary(added.view!, true);
        if (last) await boundary(view.id, false);
      }
      if (!last) {
        // As the line may be gone once its View is.
        typed = _text.text;
        stayed = _focus.hasFocus;
        await views.write(
          mutation(BridgeMutationKind.removeView, page: _page, view: view.id),
          [block],
        );
      }
    } on Object {
      // Shown on the Page.
    } finally {
      if (mounted) {
        typed = _text.text;
        stayed = _focus.hasFocus;
        _set('', 0);
        _adding = false;
      }
      if (stayed) {
        views.moveCaret(Caret(added.view!, typed.length, text: typed));
      } else if (typed.isNotEmpty) {
        unawaited(
          views
              .write(
                mutation(
                  BridgeMutationKind.renameDatabase,
                  page: _page,
                  block: added.block,
                  text: typed,
                ),
                [added.block!],
              )
              .then<void>((_) {}, onError: (Object _) {}),
        );
      }
    }
  }

  /// ↑↓ choose in the open Block menu, ⏎ takes the choice and esc closes it.
  KeyEventResult _menuKey(LogicalKeyboardKey key) {
    switch (key) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = key == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(() => _chosen = (_chosen + by) % _choices.length);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        unawaited(_choose(_choices[_chosen]));
      case LogicalKeyboardKey.escape:
        _closeMenu();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  void _report(Object error) {
    if (mounted) _state.problem.value = '$error';
  }

  /// Shows the stored text and style, unless a write to this Block or a change of the
  /// Page's structure from it is on its way.
  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    // A read asked while a write is on its way may miss it, whenever it answers.
    final clean = views.pending(block) == 0;
    final sent = views.sent(block);
    final BridgeAnswer answer;
    try {
      answer = await _state.workspace.query(
        query: query(BridgeQueryKind.text, page: _page, block: block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    // While a Table takes this line's place, what is typed there is its name.
    if (!mounted || read != _reads || _editor.isSource(this) || _adding) {
      return;
    }
    final text = answer.text;
    if (text == null ||
        !clean ||
        views.pending(block) > 0 ||
        sent != views.sent(block)) {
      return;
    }
    final style = answer.style ?? BridgeTextStyle.paragraph;
    views.setStyle(block, style);
    if (style == BridgeTextStyle.numbered) {
      unawaited(views.readRun(widget.index).catchError(_report));
    }
    if (style != _style) setState(() => _style = style);
    // Editable once the text is known, so nothing typed before is lost.
    if (_stored == null) setState(() {});
    _stored = text;
    if (_text.text != text && !_composing) {
      final offset = (_wanted ?? _text.selection.baseOffset).clamp(
        0,
        text.length,
      );
      if ((_wanted ?? 0) <= text.length) _wanted = null;
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

  @override
  void collapse() {
    final selection = _text.selection;
    if (!selection.isValid || selection.isCollapsed) return;
    _quiet = true;
    _text.selection = TextSelection.collapsed(offset: selection.end);
    _quiet = false;
  }

  @override
  void settle(String? text) {
    if (!mounted || text == null) return;
    _stored = text;
    if (_text.text != text) {
      _set(text, _text.selection.baseOffset.clamp(0, text.length));
    }
  }

  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null || caret.view != widget.view.id) return;
    // What the input queue wrote last is what the core holds, so it needs no read first.
    if (caret.text case final text?) {
      _stored = text;
      if (_text.text != text) _set(text, text.length);
    }
    if (caret.style case final style? when style != _style) {
      setState(() => _style = style);
    }
    if (_stored == null) return;
    // Takes the caret once the field is built editable, so it opens its text input.
    if (!_editable) {
      setState(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _takeCaret();
      });
      return;
    }
    widget.views.caret.value = null;
    _focus.requestFocus();
    final length = _text.text.length;
    if (caret.offset > length && caret.offset < _end) _wanted = caret.offset;
    _text.selection = TextSelection.collapsed(
      offset: caret.offset.clamp(0, length),
    );
  }

  /// Sends what typing changed, or applies a shortcut typed at the start. While the Page's
  /// structure changes from here, the input queue takes it instead.
  void _typed() {
    final stored = _stored;
    if (_quiet || stored == null || _adding) return;
    if (_editor.isSource(this)) {
      _editor.typed(this);
      return;
    }
    // On quitting, a composition still open is written as it stands.
    if (_composing && !_state.quitting) return;
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
      _write(
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
    _offer(text, stored);
  }

  /// Sends the change from the stored text to `text` as one edit.
  void _send(String text) {
    final stored = _stored!;
    if (text == stored) return;
    _stored = text;
    _write(textEdit(_page, block, stored, text));
  }

  /// Writes `change` after earlier ones; a refusal shows the stored text again (rule 7).
  void _write(BridgeMutation change) => unawaited(
    widget.views
        .write(change, [block])
        .then<void>((_) {}, onError: (Object _) {}),
  );

  void _restyle(BridgeTextStyle style) {
    setState(() => _style = style);
    widget.views.setStyle(block, style);
    _write(
      mutation(
        BridgeMutationKind.setTextStyle,
        page: _page,
        block: block,
        style: style,
      ),
    );
  }

  KeyEventResult _key(KeyEvent event) {
    if (event is KeyUpEvent || _composing || _stored == null) {
      return KeyEventResult.ignored;
    }
    // Until the Page's earlier queue is done, the field takes no keys at all.
    if (!widget.views.ready.value) return KeyEventResult.handled;
    final keys = HardwareKeyboard.instance;
    final plain =
        !keys.isShiftPressed &&
        !keys.isMetaPressed &&
        !keys.isControlPressed &&
        !keys.isAltPressed;
    if (!plain) return KeyEventResult.ignored;
    if (_editor.isSource(this)) return _editor.queue(this, event);
    if (_menu.isShowing) return _menuKey(event.logicalKey);
    // While a Table takes this line's place, only typing reaches the field.
    if (_adding) {
      return switch (event.logicalKey) {
        LogicalKeyboardKey.enter ||
        LogicalKeyboardKey.numpadEnter ||
        LogicalKeyboardKey.arrowUp ||
        LogicalKeyboardKey.arrowDown ||
        LogicalKeyboardKey.arrowLeft ||
        LogicalKeyboardKey.arrowRight => KeyEventResult.handled,
        LogicalKeyboardKey.backspace
            when _text.selection.isCollapsed &&
                _text.selection.baseOffset == 0 =>
          KeyEventResult.handled,
        _ => KeyEventResult.ignored,
      };
    }
    // While another Text Block's change of structure runs, this field types and deletes as
    // usual, and its own ⏎ or ⌫ at the start joins the queue behind that change.
    final selection = _text.selection;
    if (!selection.isValid) return KeyEventResult.ignored;
    final at = selection.baseOffset;
    final collapsed = selection.isCollapsed;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (!collapsed) {
          final text = _text.text.replaceRange(
            selection.start,
            selection.end,
            '',
          );
          _set(text, selection.start);
          _send(text);
        }
        _structure(LogicalKeyboardKey.enter);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.backspace when collapsed && at == 0:
        _structure(LogicalKeyboardKey.backspace);
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

  /// Hands ⏎ or ⌫ at the start to the Page's input queue, which changes the structure.
  void _structure(LogicalKeyboardKey key) => _editor.start(
    this,
    EditorTarget(
      view: widget.view.id,
      block: block,
      index: widget.index,
      text: _stored!,
      caret: _text.selection.start,
      style: _style,
      boundary: widget.view.boundary,
    ),
    key,
  );

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

  /// Puts the caret into the neighbouring Text Block, past Blocks of other kinds: at the end
  /// above, at the start below.
  bool _move(int by) {
    // The caret is the input queue's to place while the structure changes.
    if (_editor.busy) return false;
    final neighbour = widget.views.textFrom(widget.index + by, by);
    if (neighbour == null) return false;
    widget.views.caret.value = Caret(neighbour.id, by < 0 ? _end : 0);
    return true;
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
      readOnly: !(_editable = _stored != null && widget.views.ready.value),
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
    // The Block menu opens under the line `/` was typed in, and clicking it keeps the
    // field's focus. It sits above the style's frame, which a restyle rebuilds.
    return CompositedTransformTarget(
      link: _menuAnchor,
      child: OverlayPortal(
        controller: _menu,
        overlayChildBuilder: (context) => Align(
          alignment: Alignment.topLeft,
          child: CompositedTransformFollower(
            link: _menuAnchor,
            targetAnchor: Alignment.bottomLeft,
            child: TextFieldTapRegion(
              child: BlockMenu(
                choices: _choices,
                chosen: _chosen,
                onChoose: (choice) => unawaited(_choose(choice)),
                onHover: (index) => setState(() => _chosen = index),
              ),
            ),
          ),
        ),
        child: Padding(
          padding: EdgeInsets.only(top: top, bottom: WsMetrics.space1),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (widget.view.boundary && widget.index > 0)
                const SectionDivider(),
              text,
            ],
          ),
        ),
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

/// The divider `---` leaves above a View that starts a Section; tile edges show it in
/// Arrange mode instead.
class SectionDivider extends StatelessWidget {
  const SectionDivider({super.key = const Key('section-divider')});

  @override
  Widget build(BuildContext context) => Container(
    height: 1,
    margin: const EdgeInsets.symmetric(vertical: WsMetrics.space4),
    color: context.ws.border,
  );
}
