import 'history_dialog.dart' show LiveFocusNode;

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../highlight.dart';
import '../rust/api/workspace.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'follow.dart';
import 'page_editor.dart';
import 'page_views.dart';
import 'workspace_state.dart';

const codeLanguages = [
  ('', 'Plain text'),
  ('bash', 'Bash'),
  ('c', 'C'),
  ('cpp', 'C++'),
  ('csharp', 'C#'),
  ('css', 'CSS'),
  ('dart', 'Dart'),
  ('go', 'Go'),
  ('html', 'HTML'),
  ('java', 'Java'),
  ('javascript', 'JavaScript'),
  ('json', 'JSON'),
  ('kotlin', 'Kotlin'),
  ('markdown', 'Markdown'),
  ('python', 'Python'),
  ('ruby', 'Ruby'),
  ('rust', 'Rust'),
  ('sql', 'SQL'),
  ('swift', 'Swift'),
  ('typescript', 'TypeScript'),
  ('yaml', 'YAML'),
];

String languageLabel(String language) =>
    codeLanguages.where((known) => known.$1 == language).firstOrNull?.$2 ??
    language;

const codeLines = 24;

const _indent = '  ';

int _lineStart(String text, int at) =>
    at == 0 ? 0 : text.lastIndexOf('\n', at - 1) + 1;

TextEditingValue newLine(TextEditingValue value) {
  final TextEditingValue(:text, :selection) = value;
  final start = _lineStart(text, selection.start);
  final indent = RegExp(r'^[ \t]*')
      .stringMatch(text.substring(start, selection.start))!;
  final insert = '\n$indent';
  return TextEditingValue(
    text: text.replaceRange(selection.start, selection.end, insert),
    selection: TextSelection.collapsed(offset: selection.start + insert.length),
  );
}

TextEditingValue indentLines(TextEditingValue value, {required bool out}) {
  var TextEditingValue(:text, :selection) = value;
  var (start, end) = (selection.start, selection.end);
  final last = end > start && text.codeUnitAt(end - 1) == 0x0A ? end - 1 : end;
  final starts = [_lineStart(text, start)];
  for (
    var next = text.indexOf('\n', starts.last);
    next >= 0 && next < last;
    next = text.indexOf('\n', starts.last)
  ) {
    starts.add(next + 1);
  }
  int moved(int at, int line, int by) => by > 0
      ? (at >= line ? at + by : at)
      : (at > line ? at - math.min(-by, at - line) : at);
  for (final line in starts.reversed) {
    var by = _indent.length;
    if (out) {
      by = 0;
      while (by < _indent.length &&
          line + by < text.length &&
          text.codeUnitAt(line + by) == 0x20) {
        by++;
      }
      text = text.replaceRange(line, line + by, '');
      by = -by;
    } else {
      text = text.replaceRange(line, line, _indent);
    }
    (start, end) = (moved(start, line, by), moved(end, line, by));
  }
  return TextEditingValue(
    text: text,
    selection: TextSelection(baseOffset: start, extentOffset: end),
  );
}

class _CodeText extends TextEditingController {
  _CodeText.fromValue(super.value) : super.fromValue();

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    if (withComposing && !value.composing.isCollapsed) {
      return super.buildTextSpan(
        context: context,
        style: style,
        withComposing: withComposing,
      );
    }
    return highlight(context, text);
  }
}

class CodeBlock extends StatefulWidget {
  const CodeBlock({
    super.key,
    required this.views,
    required this.view,
    required this.index,
  });

  final PageViews views;
  final BridgeView view;

  final int index;

  @override
  State<CodeBlock> createState() => _CodeBlockState();
}

class _CodeBlockState extends State<CodeBlock> {
  TextEditingController _text = _CodeText.fromValue(TextEditingValue.empty);
  late final _focus = LiveFocusNode(
    debugLabel: 'Code Block',
    onKeyEvent: (_, event) => _key(event),
  );

  String? _stored;
  String _language = '';

  bool _quiet = false;

  bool _replaced = false;
  int _reads = 0;
  var _edits = EditChain();
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _settled;

  bool _hovered = false;
  bool _copied = false;
  Timer? _copiedFor;

  final _picker = OverlayPortalController();
  final _search = TextEditingController();
  late final _searchFocus = FocusNode(
    debugLabel: 'Language search',
    onKeyEvent: (_, event) => _pickerKey(event),
  );
  int _chosen = 0;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    _state.finishing.add(_typed);
    _focus.addListener(_redraw);
    widget.views.caret.addListener(_takeCaret);
    widget.views.ready.addListener(_redraw);
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
    _settled = widget.views.settled
        .where((settled) => settled == _block)
        .listen((_) => unawaited(_read()));
  }

  @override
  void dispose() {
    if (_stored != null && _text.text != _stored) _send(_text.text);
    _state.finishing.remove(_typed);
    widget.views.caret.removeListener(_takeCaret);
    widget.views.ready.removeListener(_redraw);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    _copiedFor?.cancel();
    _text.dispose();
    _focus.dispose();
    _search.dispose();
    _searchFocus.dispose();
    super.dispose();
  }

  void _redraw() {
    if (mounted) setState(() {});
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  bool get _composing => !_text.value.composing.isCollapsed;

  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    final clean = views.pending(_block) == 0;
    final sent = views.sent(_block);
    final BridgeAnswer answer;
    try {
      answer = await _state.read(
        query(BridgeQueryKind.code, page: _page, block: _block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    final text = answer.text;
    if (!mounted || read != _reads || text == null) return;
    if (!clean || views.pending(_block) > 0 || sent != views.sent(_block)) {
      return;
    }
    final first = _stored == null;
    _stored = text;
    _edits = EditChain();
    setState(() => _language = answer.language ?? '');
    if (_text.text != text && !_composing) {
      _set(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(
            offset: _text.selection.baseOffset.clamp(0, text.length),
          ),
        ),
      );
    }
    if (first) _takeCaret();
  }

  void _set(TextEditingValue value) {
    _quiet = true;
    final changed = writeText(_text, value);
    _quiet = false;
    if (changed && mounted) setState(() => _replaced = true);
  }

  void _edit(TextEditingValue value) {
    _quiet = true;
    _text.value = value;
    _quiet = false;
    _send(value.text);
  }

  void _typed() {
    if (_quiet || _stored == null) return;
    if (_composing && !_state.quitting) return;
    _send(_text.text);
  }

  void _send(String text) {
    final stored = _stored!;
    if (_edits.refused != null) {
      unawaited(_read());
      return;
    }
    if (text == stored) return;
    _stored = text;
    _write(textEdit(_page, _block, stored, text), chain: _edits);
  }

  void _write(BridgeMutation change, {EditChain? chain}) => unawaited(
    widget.views
        .write(change, [_block], chain: chain)
        .then<void>((_) {}, onError: (Object _) {}),
  );

  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null ||
        caret.view != widget.view.id ||
        caret.kind != BridgeBlockKind.code ||
        _stored == null) {
      return;
    }
    if (!_editable) {
      setState(() {});
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _takeCaret();
      });
      return;
    }
    widget.views.caret.value = null;
    _focus.requestFocus();
    _text.selection = TextSelection.collapsed(
      offset: caret.offset.clamp(0, _text.text.length),
    );
  }

  bool get _editable => _stored != null && widget.views.ready.value;

  KeyEventResult _key(KeyEvent event) {
    if (event is KeyUpEvent || _composing || _stored == null) {
      return KeyEventResult.ignored;
    }
    if (!widget.views.ready.value) return KeyEventResult.handled;
    final keys = HardwareKeyboard.instance;
    if (keys.isMetaPressed ||
        keys.isControlPressed ||
        keys.isAltPressed ||
        keys.logicalKeysPressed.contains(liquidKey)) {
      return KeyEventResult.ignored;
    }
    final value = _text.value;
    final selection = value.selection;
    if (!selection.isValid) return KeyEventResult.ignored;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.tab) {
      _edit(indentLines(value, out: keys.isShiftPressed));
      return KeyEventResult.handled;
    }
    if (keys.isShiftPressed) return KeyEventResult.ignored;
    final text = value.text;
    final at = selection.baseOffset;
    final collapsed = selection.isCollapsed;
    switch (key) {
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        _edit(newLine(value));
      case LogicalKeyboardKey.escape:
        _leave();
      case LogicalKeyboardKey.backspace when text.isEmpty:
        _write(
          mutation(BridgeMutationKind.turnIntoText, page: _page, block: _block),
        );
        widget.views.moveCaret(Caret(widget.view.id, 0));
      case LogicalKeyboardKey.arrowUp
          when collapsed && !text.substring(0, at).contains('\n'):
      case LogicalKeyboardKey.arrowLeft when collapsed && at == 0:
        return _move(-1) ? KeyEventResult.handled : KeyEventResult.ignored;
      case LogicalKeyboardKey.arrowDown
          when collapsed && !text.substring(at).contains('\n'):
      case LogicalKeyboardKey.arrowRight when collapsed && at == text.length:
        return _move(1) ? KeyEventResult.handled : KeyEventResult.ignored;
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  bool _move(int by) {
    if (widget.views.editor.busy) return false;
    final neighbour = widget.views.caretFrom(widget.index + by, by);
    if (neighbour == null) return false;
    widget.views.caret.value = Caret(
      neighbour.id,
      by < 0 ? caretEnd : 0,
      kind: neighbour.kind,
    );
    return true;
  }

  void _leave() {
    final below = widget.views.textFrom(widget.index + 1, 1);
    if (below == null) {
      _focus.unfocus();
    } else {
      widget.views.moveCaret(Caret(below.id, 0));
    }
  }

  void _copy() {
    unawaited(Clipboard.setData(ClipboardData(text: _text.text)));
    _copiedFor?.cancel();
    setState(() => _copied = true);
    _copiedFor = Timer(const Duration(milliseconds: 1500), () {
      if (mounted) setState(() => _copied = false);
    });
  }

  List<(String, String)> get _choices {
    final typed = _search.text.toLowerCase();
    return [
      for (final language in codeLanguages)
        if (language.$2.toLowerCase().contains(typed)) language,
    ];
  }

  void _openPicker() {
    _search.clear();
    setState(
      () => _chosen = math.max(
        0,
        codeLanguages.indexWhere((known) => known.$1 == _language),
      ),
    );
    _picker.show();
  }

  void _closePicker() {
    if (!_picker.isShowing) return;
    _picker.hide();
    _focus.requestFocus();
    setState(() {});
  }

  void _choose((String, String) language) {
    _closePicker();
    if (language.$1 == _language) return;
    setState(() => _language = language.$1);
    _write(
      mutation(
        BridgeMutationKind.setCodeLanguage,
        page: _page,
        block: _block,
        text: language.$1,
      ),
    );
  }

  KeyEventResult _pickerKey(KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final choices = _choices;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp
          when choices.isNotEmpty:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(() => _chosen = (_chosen + by) % choices.length);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (choices.isNotEmpty) _choose(choices[_chosen % choices.length]);
      case LogicalKeyboardKey.escape:
        _closePicker();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  Widget _pickerView(BuildContext context) {
    final ws = context.ws;
    final choices = _choices;
    return FloatingPanel(
      key: const Key('language picker'),
      width: 240,
      padding: const EdgeInsets.all(WsMetrics.space2),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            key: const Key('language search'),
            controller: _search,
            focusNode: _searchFocus,
            autofocus: true,
            onChanged: (_) => setState(() => _chosen = 0),
            style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
            decoration: InputDecoration(
              isDense: true,
              hintText: 'Search',
              hintStyle: TextStyle(color: ws.textFaint),
              contentPadding: const EdgeInsets.symmetric(
                horizontal: WsMetrics.space3,
                vertical: WsMetrics.space2,
              ),
              border: OutlineInputBorder(
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
            ),
          ),
          const SizedBox(height: WsMetrics.space1),
          Flexible(
            child: SingleChildScrollView(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (index, language) in choices.indexed)
                    MouseRegion(
                      onEnter: (_) => setState(() => _chosen = index),
                      child: GestureDetector(
                        onTap: () => _choose(language),
                        child: Container(
                          height: 30,
                          padding: const EdgeInsets.symmetric(
                            horizontal: WsMetrics.space3,
                          ),
                          decoration: BoxDecoration(
                            color: index == _chosen ? ws.hover : null,
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                          ),
                          child: Row(
                            children: [
                              Expanded(
                                child: Text(
                                  language.$2,
                                  style: TextStyle(
                                    fontSize: WsMetrics.fsUi,
                                    color: ws.text,
                                  ),
                                ),
                              ),
                              if (language.$1 == _language)
                                Icon(
                                  LucideIcons.check,
                                  size: 14,
                                  color: ws.textMuted,
                                ),
                            ],
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
          Divider(height: WsMetrics.space3, color: ws.border),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            child: Text(
              '↑↓ · ⏎ · esc',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _button(Widget child, {required VoidCallback onTap, Key? key}) =>
      MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          key: key,
          onTap: onTap,
          child: Container(
            height: 24,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
            alignment: Alignment.center,
            child: child,
          ),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (_replaced) {
      _replaced = false;
      _text = adoptController(
        _text,
        listener: _typed,
        create: _CodeText.fromValue,
      );
    }
    final small = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted);
    final shown = _hovered || _focus.hasFocus || _picker.isShowing;
    final header = Opacity(
      opacity: shown ? 1 : 0,
      child: SizedBox(
        height: 32,
        child: Padding(
          padding: const EdgeInsets.only(
            left: WsMetrics.space2,
            right: WsMetrics.space2,
          ),
          child: Row(
            children: [
              AnchoredMenu(
                controller: _picker,
                height: 360,
                onTapOutside: (_) => _closePicker(),
                menu: _pickerView,
                child: _button(
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(languageLabel(_language), style: small),
                      const SizedBox(width: WsMetrics.space1),
                      Icon(
                        LucideIcons.chevronDown,
                        size: 12,
                        color: ws.textMuted,
                      ),
                    ],
                  ),
                  onTap: _openPicker,
                ),
              ),
              const Spacer(),
              _button(
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(
                      _copied ? LucideIcons.check : LucideIcons.copy,
                      size: 14,
                      color: ws.textMuted,
                    ),
                    const SizedBox(width: WsMetrics.space2),
                    Text(_copied ? 'Copied' : 'Copy', style: small),
                  ],
                ),
                onTap: _copy,
              ),
            ],
          ),
        ),
      ),
    );
    final mono = monoStyle(context);
    final code = ListenableBuilder(
      listenable: _text,
      builder: (context, _) => LayoutBuilder(
        builder: (context, constraints) {
          final width = measureText(
            _text.text,
            mono,
            MediaQuery.textScalerOf(context),
          ).width;
          return ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: codeLines * WsMetrics.lhMono,
            ),
            child: SingleChildScrollView(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: SizedBox(
                  width: math.max(
                    constraints.maxWidth,
                    width + WsMetrics.space2,
                  ),
                  child: TextField(
                    key: const Key('code'),
                    controller: _text,
                    focusNode: _focus,
                    readOnly: !_editable,
                    maxLines: null,
                    keyboardType: TextInputType.multiline,
                    style: mono,
                    decoration: InputDecoration.collapsed(
                      hintText: 'Code',
                      hintStyle: mono.copyWith(color: ws.textFaint),
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
        decoration: BoxDecoration(
          color: ws.code,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          border: Border.all(
            color: _focus.hasFocus ? ws.accent : Colors.transparent,
            width: 1.5,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            header,
            Padding(
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space4,
                0,
                WsMetrics.space4,
                WsMetrics.space3,
              ),
              child: code,
            ),
          ],
        ),
      ),
    );
  }
}
