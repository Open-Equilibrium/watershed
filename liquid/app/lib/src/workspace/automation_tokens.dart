import 'package:flutter/material.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'chart_block.dart' show ChoiceList;

class Token extends StatefulWidget {
  const Token({
    super.key,
    required this.name,
    required this.label,
    required this.choices,
    this.reference = false,
    this.danger = false,
    this.add = false,
  });

  final String name;
  final String label;
  final List<(String, bool, VoidCallback)> choices;
  final bool reference;
  final bool danger;
  final bool add;

  @override
  State<Token> createState() => _TokenState();
}

class _TokenState extends State<Token> {
  final _menu = OverlayPortalController();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (fill, color, border) = widget.danger
        ? (ws.dangerSubtle, ws.danger, Colors.transparent)
        : widget.reference
        ? (ws.accentSubtle, ws.accent, Colors.transparent)
        : widget.add
        ? (Colors.transparent, ws.textFaint, ws.borderStrong)
        : (ws.subtle, ws.text, ws.border);
    return AnchoredMenu(
      controller: _menu,
      height: 32.0 * widget.choices.length + 8,
      onTapOutside: (_) => setState(_menu.hide),
      menu: (_) => ChoiceList(
        choices: [
          for (final (label, chosen, onTap) in widget.choices)
            (
              label,
              chosen,
              () {
                setState(_menu.hide);
                if (!chosen) onTap();
              },
            ),
        ],
      ),
      child: Semantics(
        button: true,
        child: InkWell(
          key: Key('token ${widget.name}'),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          onTap: widget.choices.isEmpty ? null : () => setState(_menu.toggle),
          child: Container(
            height: 30,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: fill,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              border: Border.all(color: border),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (widget.add) ...[
                  Icon(LucideIcons.plus, size: 14, color: color),
                  const SizedBox(width: WsMetrics.space1),
                ],
                Text(
                  widget.label,
                  style: TextStyle(
                    fontSize: WsMetrics.fsUi,
                    color: color,
                    fontWeight: widget.reference ? WsMetrics.fwMedium : null,
                    decoration: widget.danger
                        ? TextDecoration.lineThrough
                        : null,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class TokenField extends StatelessWidget {
  const TokenField({
    super.key,
    required this.controller,
    required this.hint,
    required this.onFocus,
    required this.onSubmitted,
    this.width = 140,
  });

  final TextEditingController controller;
  final String hint;
  final ValueChanged<bool> onFocus;
  final ValueChanged<String> onSubmitted;
  final double width;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      width: width,
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        border: Border.all(color: ws.border),
      ),
      child: Focus(
        onFocusChange: onFocus,
        child: TextField(
          controller: controller,
          onSubmitted: onSubmitted,
          style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
          decoration: InputDecoration.collapsed(
            hintText: hint,
            hintStyle: TextStyle(color: ws.textFaint),
          ),
        ),
      ),
    );
  }
}
