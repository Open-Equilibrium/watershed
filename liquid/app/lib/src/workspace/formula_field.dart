import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'formula_text.dart';

const formulaHint = 'A value, or = and a formula';

class FormulaField extends StatefulWidget {
  const FormulaField({
    super.key,
    required this.formula,
    required this.names,
    required this.known,
    required this.onCommit,
    required this.onDone,
    this.always = false,
    this.autofocus = true,
  });

  final String formula;
  final List<String?> names;
  final List<Suggestion> Function() known;
  final Future<void> Function(String formula) onCommit;
  final VoidCallback onDone;
  final bool always;
  final bool autofocus;

  @override
  State<FormulaField> createState() => _FormulaFieldState();
}

class _FormulaFieldState extends State<FormulaField> {
  final _text = FormulaText();
  late final _focus = FocusNode(
    debugLabel: 'Formula',
    onKeyEvent: (_, event) => _key(event),
  );
  final _menu = OverlayPortalController();
  List<Suggestion> _suggestions = const [];
  int _chosen = 0;
  bool _done = false;

  @override
  void initState() {
    super.initState();
    final (text, chips) = editable(
      widget.formula,
      widget.names,
      always: widget.always,
    );
    _text
      ..always = widget.always
      ..load(text, chips);
    _text.addListener(_typed);
    _focus.addListener(_left);
    if (!widget.autofocus) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _left() {
    if (_focus.hasFocus || _done) return;
    unawaited(_commit());
  }

  void _typed() {
    final word = _text.word;
    final next = word == null
        ? const <Suggestion>[]
        : suggestionsFor(word, widget.known());
    final same =
        next.length == _suggestions.length &&
        next.indexed.every(
          (item) => _suggestions[item.$1].name == item.$2.name,
        );
    if (same) return;
    setState(() {
      _suggestions = next;
      _chosen = 0;
    });
    next.isEmpty ? _menu.hide() : _menu.show();
  }

  Future<void> _commit() async {
    final formula = storedFormula(
      _text.text,
      _text.chips,
      always: widget.always,
    );
    if (formula == widget.formula.trim()) return _finish();
    try {
      await widget.onCommit(formula);
      _finish();
    } on Object catch (_) {
      if (mounted && !_focus.hasFocus) _finish();
    }
  }

  void _finish() {
    if (_done) return;
    _done = true;
    widget.onDone();
  }

  void _take(Suggestion suggestion) {
    _text.take(suggestion);
    setState(() => _suggestions = const []);
    _menu.hide();
  }

  KeyEventResult _key(KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final key = event.logicalKey;
    final count = _suggestions.length;
    if (count > 0 && key == LogicalKeyboardKey.arrowDown) {
      setState(() => _chosen = (_chosen + 1) % count);
    } else if (count > 0 && key == LogicalKeyboardKey.arrowUp) {
      setState(() => _chosen = (_chosen - 1 + count) % count);
    } else if (count > 0 &&
        (key == LogicalKeyboardKey.tab || key == LogicalKeyboardKey.enter)) {
      _take(_suggestions[_chosen]);
    } else if (key == LogicalKeyboardKey.enter) {
      unawaited(_commit());
    } else if (key == LogicalKeyboardKey.escape && count > 0) {
      setState(() => _suggestions = const []);
      _menu.hide();
    } else if (key == LogicalKeyboardKey.escape) {
      _finish();
    } else {
      return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return AnchoredMenu(
      controller: _menu,
      height: 28.0 * _suggestions.length + 8,
      menu: (_) => _list(ws),
      child: Container(
        height: 26,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        alignment: Alignment.centerLeft,
        decoration: BoxDecoration(
          color: ws.canvas,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          border: Border.all(color: ws.accent, width: 1.5),
        ),
        child: TextField(
          controller: _text,
          focusNode: _focus,
          style: TextStyle(
            fontFamily: WsMetrics.fontMono.first,
            fontSize: WsMetrics.fsMono,
            color: ws.text,
          ),
          decoration: InputDecoration.collapsed(
            hintText: widget.always ? '= a formula' : formulaHint,
            hintStyle: TextStyle(color: ws.textFaint),
          ),
        ),
      ),
    );
  }

  Widget _list(WsPalette ws) => FloatingPanel(
    width: 280,
    padding: const EdgeInsets.all(WsMetrics.space1),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (at, suggestion) in _suggestions.indexed)
          GestureDetector(
            onTap: () => _take(suggestion),
            child: Container(
              height: 28,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              decoration: BoxDecoration(
                color: at == _chosen ? ws.selected : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Row(
                children: [
                  Icon(
                    suggestion.reference == null
                        ? LucideIcons.functionSquare
                        : LucideIcons.hash,
                    size: 14,
                    color: ws.textFaint,
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  Expanded(
                    child: Text(
                      suggestion.name,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.text,
                      ),
                    ),
                  ),
                  Text(
                    suggestion.detail,
                    style: TextStyle(
                      fontSize: WsMetrics.fsSmall,
                      color: ws.textFaint,
                    ),
                  ),
                ],
              ),
            ),
          ),
      ],
    ),
  );
}
