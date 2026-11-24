part of 'text_block.dart';

extension _TextBlockKeys on _TextBlockState {
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
}
