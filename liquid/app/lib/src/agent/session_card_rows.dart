import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/icons.dart';
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

/// A section of the card: its blocks in a padded column, below a hairline when `divided`, which
/// T3 Code draws when an earlier section has content.
class CardSection extends StatelessWidget {
  const CardSection({super.key, this.divided = false, required this.children});

  final bool divided;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      border: divided
          ? Border(top: BorderSide(color: context.ws.border))
          : null,
    ),
    child: Padding(
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      ),
    ),
  );
}

/// A section's heading, small and muted, with what sits at its end, such as a menu button.
class CardHeading extends StatelessWidget {
  const CardHeading({super.key, required this.title, this.trailing});

  final String title;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(10, 0, 4, 4),
    child: Row(
      children: [
        Expanded(
          child: Text(
            title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: WsMetrics.fsCaption,
              fontWeight: WsMetrics.fwMedium,
              color: context.ws.textMuted,
            ),
          ),
        ),
        ?trailing,
      ],
    ),
  );
}

/// A row's small icon button, named by its tooltip; it is off when `onPressed` is null.
class CardIconButton extends StatelessWidget {
  const CardIconButton({
    super.key,
    required this.icon,
    required this.tooltip,
    this.onPressed,
    this.color,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback? onPressed;
  final Color? color;

  @override
  Widget build(BuildContext context) => IconButton(
    icon: Icon(icon, size: 16, color: color ?? context.ws.textMuted),
    tooltip: tooltip,
    onPressed: onPressed,
    visualDensity: VisualDensity.compact,
    padding: EdgeInsets.zero,
    constraints: const BoxConstraints.tightFor(width: 24, height: 24),
  );
}

/// Rows that show `initial` at first. "Show N more" reveals `page` more at a time, or all the rest
/// when `page` is null; a `collapsible` list also offers "Show less" once it shows more.
class PagedRows extends StatefulWidget {
  const PagedRows({
    super.key,
    required this.rows,
    required this.initial,
    this.page,
    this.collapsible = false,
  });

  final List<Widget> rows;
  final int initial;
  final int? page;
  final bool collapsible;

  @override
  State<PagedRows> createState() => _PagedRowsState();
}

class _PagedRowsState extends State<PagedRows> {
  late int _visible = widget.initial;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final hidden = widget.rows.length - _visible;
    final more = widget.page == null ? hidden : math.min(hidden, widget.page!);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ...widget.rows.take(_visible),
        if (hidden > 0)
          CardRow(
            onTap: () => setState(() => _visible += more),
            children: [
              Icon(LucideIcons.plus, size: 16, color: ws.textFaint),
              Text('Show $more more'),
            ],
          ),
        if (hidden <= 0 && widget.collapsible && _visible > widget.initial)
          CardRow(
            onTap: () => setState(() => _visible = widget.initial),
            children: [
              Icon(LucideIcons.chevronUp, size: 16, color: ws.textFaint),
              const Text('Show less'),
            ],
          ),
      ],
    );
  }
}
