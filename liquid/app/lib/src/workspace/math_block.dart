import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_math_fork/flutter_math.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'follow.dart';
import 'page_editor.dart';
import 'page_views.dart';
import 'workspace_state.dart';

const mathSize = 20.0;

class MathBlock extends StatefulWidget {
  const MathBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<MathBlock> createState() => _MathBlockState();
}

class _MathBlockState extends State<MathBlock> {
  final _text = TextEditingController();
  late final _focus = FocusNode(
    debugLabel: 'Math Block',
    onKeyEvent: (_, event) => _key(event),
  );
  String? _stored;
  bool _quiet = false;
  int _reads = 0;
  var _edits = EditChain();
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _settled;

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;
  bool _editing = false;

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    _focus.addListener(_left);
    widget.views.caret.addListener(_takeCaret);
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
    widget.views.caret.removeListener(_takeCaret);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _left() {
    if (!_focus.hasFocus && _editing && mounted) {
      setState(() => _editing = false);
    }
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  Future<void> _read() async {
    final read = ++_reads;
    final views = widget.views;
    final clean = views.pending(_block) == 0;
    final sent = views.sent(_block);
    final BridgeAnswer answer;
    try {
      answer = await _state.read(
        query(BridgeQueryKind.math, page: _page, block: _block),
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
    if (_text.text != text) {
      _quiet = true;
      writeText(
        _text,
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(
            offset: _text.selection.baseOffset.clamp(0, text.length),
          ),
        ),
      );
      _quiet = false;
    }
    setState(() {});
    if (first) _takeCaret();
  }

  void _typed() {
    if (_quiet || _stored == null) return;
    setState(() {});
    if (!_text.value.composing.isCollapsed && !_state.quitting) return;
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
    unawaited(
      widget.views
          .write(textEdit(_page, _block, stored, text), [_block], chain: _edits)
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  void _takeCaret() {
    final caret = widget.views.caret.value;
    if (caret == null ||
        caret.view != widget.view.id ||
        caret.kind != BridgeBlockKind.math ||
        _stored == null) {
      return;
    }
    widget.views.caret.value = null;
    _open();
  }

  void _open() {
    setState(() => _editing = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _focus.requestFocus();
      _text.selection = TextSelection.collapsed(offset: _text.text.length);
    });
  }

  KeyEventResult _key(KeyEvent event) {
    if (event is KeyUpEvent || _stored == null) return KeyEventResult.ignored;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.escape:
        _focus.unfocus();
      case LogicalKeyboardKey.backspace when _text.text.isEmpty:
        unawaited(
          widget.views
              .write(
                mutation(
                  BridgeMutationKind.turnIntoText,
                  page: _page,
                  block: _block,
                ),
                [_block],
              )
              .then<void>((_) {}, onError: (Object _) {}),
        );
        widget.views.moveCaret(Caret(widget.view.id, 0));
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final source = _text.text;
    final mono = TextStyle(
      fontFamily: WsMetrics.fontMono.first,
      fontSize: WsMetrics.fsMono,
      height: WsMetrics.lhMono / WsMetrics.fsMono,
      color: ws.text,
    );
    final formula = source.trim().isEmpty
        ? Text(
            'An empty formula',
            style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.textFaint),
          )
        : Math.tex(
            source,
            mathStyle: MathStyle.display,
            textStyle: TextStyle(fontSize: mathSize, color: ws.text),
            onErrorFallback: (error) => Column(
              key: const Key('math error'),
              children: [
                Text(source, style: mono.copyWith(color: ws.textFaint)),
                const SizedBox(height: WsMetrics.space1),
                Text(
                  error.messageWithType,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.danger,
                  ),
                ),
              ],
            ),
          );
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: _editing ? null : _open,
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space5,
          vertical: WsMetrics.space4,
        ),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          border: Border.all(
            color: _editing ? ws.accent : Colors.transparent,
            width: 1.5,
          ),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Center(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: formula,
              ),
            ),
            if (_editing)
              Padding(
                padding: const EdgeInsets.only(top: WsMetrics.space3),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space4,
                        vertical: WsMetrics.space3,
                      ),
                      decoration: BoxDecoration(
                        color: ws.code,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                        border: Border.all(color: ws.accent, width: 1.5),
                      ),
                      child: TextField(
                        key: const Key('math source'),
                        controller: _text,
                        focusNode: _focus,
                        maxLines: null,
                        keyboardType: TextInputType.multiline,
                        style: mono,
                        decoration: InputDecoration.collapsed(
                          hintText: r'LaTeX, as \frac{a}{b}',
                          hintStyle: mono.copyWith(color: ws.textFaint),
                        ),
                      ),
                    ),
                    const SizedBox(height: WsMetrics.space1),
                    Text(
                      'The formula follows as you type · esc or a click '
                      'outside closes',
                      textAlign: TextAlign.right,
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.textFaint,
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }
}
