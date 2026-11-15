import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// A row of the session card: 32 px high with 13 px medium text and a hover fill, as the rows of
/// T3 Code's thread details card.
class CardRow extends StatelessWidget {
  const CardRow({super.key, required this.children, this.onTap});

  final List<Widget> children;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Material(
      type: MaterialType.transparency,
      child: InkWell(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        hoverColor: ws.hover,
        highlightColor: Colors.transparent,
        splashColor: Colors.transparent,
        onTap: onTap,
        child: Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: DefaultTextStyle.merge(
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontWeight: WsMetrics.fwMedium,
              color: ws.text,
            ),
            child: Row(spacing: 10, children: children),
          ),
        ),
      ),
    );
  }
}
