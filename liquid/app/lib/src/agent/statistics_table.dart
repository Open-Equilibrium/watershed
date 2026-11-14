import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/row_menu.dart';

/// A breakdown table: a `#` column, a name that takes the rest, then right-aligned figures.
class StatisticsTable extends StatelessWidget {
  const StatisticsTable({
    super.key,
    required this.headers,
    required this.rows,
    this.onSelect,
  });

  final List<String> headers;
  final List<List<Widget>> rows;

  /// Called with the index of a row when it is chosen; rows choose nothing without it.
  final ValueChanged<int>? onSelect;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget cell(int column, Widget child) => column == 0
        ? SizedBox(width: 22, child: child)
        : column == 1
        ? Expanded(
            child: Align(alignment: Alignment.centerLeft, child: child),
          )
        : SizedBox(
            width: 72,
            child: Align(alignment: Alignment.centerRight, child: child),
          );
    Widget row(
      List<Widget> cells, {
      required bool header,
      VoidCallback? onTap,
    }) {
      final content = Container(
        padding: const EdgeInsets.symmetric(vertical: 7),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (column, child) in cells.indexed) cell(column, child),
          ],
        ),
      );
      return header ? content : _BreakdownRow(onTap: onTap, child: content);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row([
          for (final label in headers)
            Text(
              label,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
        ], header: true),
        for (final (index, cells) in rows.indexed)
          row(
            cells,
            header: false,
            onTap: onSelect == null ? null : () => onSelect!(index),
          ),
      ],
    );
  }
}

/// The legend of a chart or a share bar: a dot, its label and, when given, its value.
class StatisticsLegend extends StatelessWidget {
  const StatisticsLegend(this.items, {super.key});

  /// Each item's label, colour and value; an empty value shows the label alone.
  final List<(String, Color, String)> items;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Wrap(
      spacing: 14,
      runSpacing: 4,
      children: [
        for (final (label, colour, value) in items)
          Row(
            mainAxisSize: MainAxisSize.min,
            spacing: 5,
            children: [
              ColourDot(colour, size: 8, radius: 2),
              Text(
                label,
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
              if (value.isNotEmpty)
                Text(
                  value,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.text,
                    fontFeatures: const [FontFeature.tabularFigures()],
                  ),
                ),
            ],
          ),
      ],
    );
  }
}

/// A small square of a series' colour, as the legends and the card's rows show it.
class ColourDot extends StatelessWidget {
  const ColourDot(this.colour, {super.key, this.size = 8, this.radius = 4});

  final Color colour;
  final double size;
  final double radius;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: colour,
      borderRadius: BorderRadius.circular(radius),
    ),
  );
}

/// A breakdown row: it takes the hover colour while the pointer is over it, and runs `onTap`
/// when it is selectable.
class _BreakdownRow extends StatefulWidget {
  const _BreakdownRow({required this.onTap, required this.child});

  final VoidCallback? onTap;
  final Widget child;

  @override
  State<_BreakdownRow> createState() => _BreakdownRowState();
}

class _BreakdownRowState extends State<_BreakdownRow> {
  bool _hovered = false;
  bool _focused = false;

  @override
  Widget build(BuildContext context) => HoverRegion(
    onHover: (hovered, _) {
      if (mounted) setState(() => _hovered = hovered);
    },
    child: InkWell(
      // A row that selects takes the keyboard, and Enter and Space select it, as a button does.
      canRequestFocus: widget.onTap != null,
      onTap: widget.onTap,
      onFocusChange: (focused) {
        if (mounted) setState(() => _focused = focused);
      },
      mouseCursor: widget.onTap == null
          ? MouseCursor.defer
          : SystemMouseCursors.click,
      hoverColor: Colors.transparent,
      focusColor: Colors.transparent,
      highlightColor: Colors.transparent,
      splashColor: Colors.transparent,
      child: AnimatedContainer(
        duration: WsMetrics.motionFast,
        color: _hovered || _focused ? context.ws.hover : null,
        child: widget.child,
      ),
    ),
  );
}
