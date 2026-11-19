import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'follow.dart';
import 'media_block.dart';
import 'page_editor.dart';
import 'page_views.dart';
import 'paste.dart';
import 'slash_commands.dart';
import 'workspace_state.dart';

const _end = caretEnd;

class TextBlock extends StatefulWidget {
  const TextBlock({
    super.key,
    required this.views,
    required this.view,
    required this.index,
  });

  final PageViews views;
  final BridgeView view;

  final int index;

  @override
  State<TextBlock> createState() => _TextBlockState();
}

class _TextBlockState extends State<TextBlock> implements EditorSource {
  var _text = TextEditingController();

  final _field = GlobalKey(debugLabel: 'Text Block field');
  late final _focus = FocusNode(
    debugLabel: 'Text Block',
    onKeyEvent: (_, event) => _key(event),
  );

  String? _stored;
  BridgeTextStyle _style = BridgeTextStyle.paragraph;

  bool _quiet = false;

  bool _replaced = false;

  int _reads = 0;

  var _edits = EditChain();

  bool _editable = false;

  int? _wanted;
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _settled;

  final _menu = OverlayPortalController();
  var _choices = const <SlashCommand>[];
  var _chosen = 0;

  var _adding = false;

  BridgeApplied? _added;

  String? _gone;

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
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _takeCaret();
    });
    _settled = widget.views.settled
        .where((settled) => settled == block)
        .listen((_) => unawaited(_read()));
  }

  @override
  void dispose() {
    if (_adding) {
      final typed = _gone = _text.text;
      if (_added case final added?) {
        _name(widget.views, added, typed);
        widget.views.naming[added.view!] = () => (typed, true);
      }
    }
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

  void _readied() {
    if (!mounted) return;
    setState(() => _stored = null);
    unawaited(_read());
  }

  @override
  bool get focused => _focus.hasFocus;

  void _focusChanged() {
    if (_focus.hasFocus) return;
    _closeMenu();
    if (_adding && _added != null) _finish(_text.text);
  }

  void _closeMenu() {
    if (_menu.isShowing) _menu.hide();
  }

  void _offer(String text, String stored, {bool handed = false}) {
    final opening =
        stored.isEmpty && (text == '/' || handed && text.startsWith('/'));
    if (!opening && !_menu.isShowing) return;
    final choices = text.startsWith('/')
        ? slashCommandsFor(text.substring(1))
        : const <SlashCommand>[];
    if (choices.isEmpty) return _closeMenu();
    setState(() {
      _choices = choices;
      _chosen = 0;
    });
    _menu.show();
  }

  void _take(SlashCommand choice) => _state.track(_choose(choice));

  Future<void> _choose(SlashCommand choice) async {
    _closeMenu();
    _set('', 0);
    _send('');
    if (choice.style case final style?) {
      if (style != _style) _restyle(style);
      return;
    }
    if (choice.block == BridgeBlockKind.code) return _turnIntoCode();
    if (choice.block == BridgeBlockKind.media) return _addMedia(_pickFiles);
    if (choice.block == BridgeBlockKind.whiteboard) return _addWhiteboard();
    final views = widget.views;
    final view = widget.view;
    final last = widget.index == (views.count ?? 0) - 1;
    _adding = true;
    final BridgeApplied added;
    try {
      String? after = view.id;
      if (last) {
        final above = widget.index > 0
            ? await views.viewAt(widget.index - 1)
            : null;
        if (widget.index > 0 && above == null) {
          _state.report(
            'Liquid did not add the Table, as the Page changed; choose Table again.',
          );
          throw StateError('The Page changed.');
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
    } on Object {
      if (mounted) {
        _adding = false;
        _typed();
      } else if (_gone case final gone? when gone.isNotEmpty) {
        unawaited(
          views
              .write(textEdit(_page, block, '', gone), [block])
              .then<void>((_) {}, onError: (Object _) {}),
        );
      }
      return;
    }
    _added = added;
    if (!mounted) {
      _name(views, added, _gone ?? '');
    } else if (!_focus.hasFocus || _state.quitting) {
      _finish(_text.text);
    } else {
      views.naming[added.view!] = _handOver;
      views.moveCaret(Caret(added.view!, 0));
    }
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
        await views.write(
          mutation(BridgeMutationKind.removeView, page: _page, view: view.id),
          [block],
        );
      }
    } on Object catch (_) {}
  }

  (String, bool) _handOver() {
    final typed = _text.text;
    _release();
    return (typed, false);
  }

  void _finish(String typed) {
    final added = _added!;
    final views = widget.views;
    if (views.caret.value?.view == added.view) views.caret.value = null;
    _name(views, added, typed);
    _release();
  }

  void _name(PageViews views, BridgeApplied added, String typed) {
    views.naming.remove(added.view);
    if (typed.isEmpty) return;
    unawaited(
      views
          .write(
            mutation(
              BridgeMutationKind.renameView,
              page: _page,
              view: added.view,
              text: typed,
            ),
            [added.view!],
          )
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  void _release() {
    widget.views.naming.remove(_added?.view);
    _adding = false;
    _added = null;
    if (mounted) _set('', 0);
  }

  Widget _menuView(BuildContext context) => SlashCommands(
    choices: _choices,
    chosen: _chosen,
    onChoose: _take,
    onHover: (index) => setState(() => _chosen = index),
  );

  KeyEventResult _menuKey(LogicalKeyboardKey key) {
    switch (key) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = key == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(() => _chosen = (_chosen + by) % _choices.length);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        _take(_choices[_chosen]);
      case LogicalKeyboardKey.escape:
        _closeMenu();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    final clean = views.pending(block) == 0;
    final sent = views.sent(block);
    final BridgeAnswer answer;
    try {
      answer = await _state.read(
        query(BridgeQueryKind.text, page: _page, block: block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
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
    if (_stored == null) setState(() {});
    _stored = text;
    _edits = EditChain();
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
    final changed = writeText(
      _text,
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: offset),
      ),
    );
    _quiet = false;
    if (changed && mounted) setState(() => _replaced = true);
  }

  void _adopt() {
    _replaced = false;
    _text = adoptController(_text, listener: _typed);
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
    if (caret == null ||
        caret.view != widget.view.id ||
        caret.kind != BridgeBlockKind.text) {
      return;
    }
    if (caret.text case final text?) {
      _stored = text;
      if (_text.text != text) _set(text, text.length);
    }
    if (caret.style case final style? when style != _style) {
      setState(() => _style = style);
    }
    if (_stored == null) return;
    if (!_editable) {
      setState(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _takeCaret();
      });
      return;
    }
    widget.views.caret.value = null;
    _focus.requestFocus();
    if (caret.text case final text? when caret.slash) {
      _offer(text, '', handed: true);
    }
    final length = _text.text.length;
    if (caret.offset > length && caret.offset < _end) _wanted = caret.offset;
    _text.selection = TextSelection.collapsed(
      offset: caret.offset.clamp(0, length),
    );
  }

  void _typed() {
    final stored = _stored;
    if (_adding) {
      if (_state.quitting && _added != null) _finish(_text.text);
      return;
    }
    if (_quiet || stored == null) return;
    if (_editor.isSource(this)) {
      _editor.typed(this);
      return;
    }
    if (_composing && !_state.quitting) return;
    final text = _text.text;
    if (text == stored) return;
    if (codeTyped(text, stored, _text.selection.baseOffset)) {
      final rest = text.substring(codeShortcut.length);
      _set(rest, 0);
      _send(rest);
      _turnIntoCode();
      return;
    }
    if (shortcutTyped(text, stored, _text.selection.baseOffset, _style) case (
      final typed,
      final style,
    )) {
      final rest = text.substring(typed.length);
      _set(rest, 0);
      _send(rest);
      _restyle(style);
      return;
    }
    if (startsSection(
      text,
      stored,
      index: widget.index,
      boundary: widget.view.boundary,
    )) {
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

  void _send(String text) {
    final stored = _stored!;
    if (_edits.refused != null) {
      unawaited(_read());
      return;
    }
    if (text == stored) return;
    _stored = text;
    _write(textEdit(_page, block, stored, text), chain: _edits);
  }

  void _write(BridgeMutation change, {EditChain? chain}) => unawaited(
    widget.views
        .write(change, [block], chain: chain)
        .then<void>((_) {}, onError: (Object _) {}),
  );

  Future<List<MediaSource>> _pickFiles() async {
    try {
      return await MediaPlatform.pickFiles();
    } on Object catch (error) {
      _state.report('Liquid could not open the file dialog: $error');
      return const [];
    }
  }

  Future<void> _addMedia(Future<List<MediaSource>> Function() sources) async {
    final views = widget.views;
    final view = widget.view;
    final index = widget.index;
    final empty = _text.text.isEmpty;
    final last = index == (views.count ?? 0) - 1;
    final added = await sources();
    if (added.isEmpty) return;
    String? after = view.id;
    if (empty && last) {
      final above = index > 0 ? await views.viewAt(index - 1) : null;
      if (index > 0 && above == null) {
        _state.report(
          'Liquid did not add the files, as the Page changed; choose Media again.',
        );
        return;
      }
      after = above?.id;
    }
    final first = await addMedia(
      views,
      added,
      after: after,
      replacing: empty && !last ? view : null,
    );
    if (empty && first != null) await _handOverSection(first, last: last);
  }

  Future<void> _handOverSection(String added, {required bool last}) async {
    final view = widget.view;
    if (!view.boundary) return;
    Future<void> boundary(String at, bool starts) => widget.views.write(
      mutation(
        BridgeMutationKind.setSectionBoundary,
        page: _page,
        view: at,
        boundary: starts,
      ),
      const [],
    );
    try {
      await boundary(added, true);
      if (last) await boundary(view.id, false);
    } on Object {
      // Shown on the Page.
    }
  }

  Future<void> _addWhiteboard() async {
    final views = widget.views;
    final view = widget.view;
    final index = widget.index;
    final empty = _text.text.isEmpty;
    final last = index == (views.count ?? 0) - 1;
    String? after = view.id;
    if (empty && last) {
      final above = index > 0 ? await views.viewAt(index - 1) : null;
      if (index > 0 && above == null) {
        _state.report(
          'Liquid did not add the Whiteboard, as the Page changed; choose Whiteboard again.',
        );
        return;
      }
      after = above?.id;
    }
    try {
      final added = await views.write(
        mutation(
          BridgeMutationKind.addWhiteboardBlock,
          page: _page,
          after: after,
        ),
        const [],
      );
      views.focusBoard.value = added.view;
      if (empty) await _handOverSection(added.view!, last: last);
      if (empty && !last) {
        await views.write(
          mutation(BridgeMutationKind.removeView, page: _page, view: view.id),
          [view.block],
        );
      }
    } on Object {
      // The problem shows on the Page.
    }
  }

  Future<void> _paste() async {
    final Pasted pasted;
    try {
      pasted = await readClipboard();
    } on Object catch (error) {
      _state.report('Liquid could not read the clipboard: $error');
      return;
    }
    if (!mounted) return;
    if (pasted.files.isNotEmpty) {
      return _addMedia(() => sourcesAt(_state, pasted.files));
    }
    if (pasted.image case final image?) {
      return _addMedia(
        () async => [MediaSource.bytes('Pasted image.png', image)],
      );
    }
    if (pasted.code case final code?) return _pasteCode(code, pasted.language);
    final text = pasted.text;
    final selection = _text.selection;
    if (text == null ||
        text.isEmpty ||
        !selection.isValid ||
        selection.end > _text.text.length) {
      return;
    }
    final value = _text.value.replaced(selection, text);
    _text.value = value.copyWith(
      selection: TextSelection.collapsed(offset: selection.start + text.length),
    );
  }

  Future<void> _pasteCode(String code, String language) async {
    final views = widget.views;
    final view = widget.view;
    var target = (view: view.id, block: block);
    if (_text.text.isNotEmpty) {
      final added = await views.write(
        mutation(
          BridgeMutationKind.addTextBlock,
          page: _page,
          after: view.id,
          text: '',
        ),
        const [],
      );
      target = (view: added.view!, block: added.block!);
    }
    final writes = [
      mutation(
        BridgeMutationKind.turnIntoCode,
        page: _page,
        block: target.block,
      ),
      mutation(
        BridgeMutationKind.editText,
        page: _page,
        block: target.block,
        text: code,
      ),
      if (language.isNotEmpty)
        mutation(
          BridgeMutationKind.setCodeLanguage,
          page: _page,
          block: target.block,
          text: language,
        ),
    ];
    try {
      for (final write in writes) {
        await views.write(write, [target.block]);
      }
    } on Object {
      return;
    }
    views.moveCaret(Caret(target.view, caretEnd, kind: BridgeBlockKind.code));
  }

  void _turnIntoCode() {
    _write(
      mutation(BridgeMutationKind.turnIntoCode, page: _page, block: block),
    );
    widget.views.moveCaret(
      Caret(widget.view.id, 0, kind: BridgeBlockKind.code),
    );
  }

  void _toggleToDo() {
    final done = _style == BridgeTextStyle.done;
    if (done || _style == BridgeTextStyle.todo) {
      _restyle(done ? BridgeTextStyle.todo : BridgeTextStyle.done);
    }
  }

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
    if (!widget.views.ready.value) return KeyEventResult.handled;
    final keys = HardwareKeyboard.instance;
    final plain =
        !keys.isShiftPressed &&
        !keys.isMetaPressed &&
        !keys.isControlPressed &&
        !keys.isAltPressed &&
        !keys.logicalKeysPressed.contains(liquidKey);
    if (isPaste(event) &&
        !_editor.isSource(this) &&
        !_adding &&
        !_menu.isShowing) {
      _state.track(_paste());
      return KeyEventResult.handled;
    }
    if (!plain) return KeyEventResult.ignored;
    if (_editor.isSource(this)) return _editor.queue(this, event);
    if (_menu.isShowing) return _menuKey(event.logicalKey);
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

  bool _move(int by) {
    if (_editor.busy) return false;
    final neighbour = widget.views.caretFrom(widget.index + by, by);
    if (neighbour == null) return false;
    widget.views.caret.value = Caret(
      neighbour.id,
      by < 0 ? _end : 0,
      kind: neighbour.kind,
    );
    return true;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (_replaced) _adopt();
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
                onTap: _toggleToDo,
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
    text = CallbackShortcuts(
      bindings: Keymap.of(context).bind({Command.toggleToDo: _toggleToDo}),
      child: text,
    );
    return AnchoredMenu(
      controller: _menu,
      height: SlashCommands.heightFor(_choices),
      gap: 0,
      menu: _menuView,
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
    );
  }

  int _number(Map<String, BridgeTextStyle> styles) {
    if (widget.view.boundary) return 1;
    var number = 1;
    for (var index = widget.index - 1; index >= 0; index--) {
      final above = widget.views.peek(index);
      if (above == null ||
          above.kind != BridgeBlockKind.text ||
          styles[above.block] != BridgeTextStyle.numbered) {
        break;
      }
      number++;
      if (above.boundary) break;
    }
    return number;
  }
}

class SectionDivider extends StatelessWidget {
  const SectionDivider({super.key = const Key('section-divider')});

  @override
  Widget build(BuildContext context) => Container(
    height: 1,
    margin: const EdgeInsets.symmetric(vertical: WsMetrics.space4),
    color: context.ws.border,
  );
}
