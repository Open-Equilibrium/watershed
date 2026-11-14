import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// One turn's column: its parts from the bottom up, and what its tooltip says.
class ChartColumn {
  const ChartColumn({required this.parts, required this.tip});

  final List<int> parts;
  final ChartTip tip;
}

/// A tooltip of the chart: a title, one row per part with its colour, and notes.
class ChartTip {
  const ChartTip({
    required this.title,
    this.rows = const [],
    this.notes = const [],
  });

  final String title;
  final List<ChartTipRow> rows;

  /// Lines below the rows, in a fainter ink.
  final List<String> notes;
}

class ChartTipRow {
  const ChartTipRow(this.color, this.label, this.value);

  final Color color;
  final String label;
  final String value;
}

/// The smallest axis top from the steps 1, 2, 2.5, 5 and 10 times a power of ten that holds
/// `max`; zero for no data.
int niceTop(int max) {
  if (max <= 0) return 0;
  final magnitude = math.pow(10, (math.log(max) / math.ln10).floor()).toInt();
  for (final step in [1, 2, 2.5, 5, 10]) {
    final top = (step * magnitude).round();
    if (top >= max) return top;
  }
  return max;
}

/// The chart's size, as the statistics panel draws it: 156 px high, the axis to the left.
const _height = 156.0;
const _axisWidth = 54.0;
const _top = 8.0;
const _labelsTop = 150.0;

/// A stacked column chart with one column per turn. The column under the pointer lights up and
/// shows its tooltip.
class TurnChart extends StatefulWidget {
  const TurnChart({
    super.key,
    required this.columns,
    required this.colors,
    required this.axis,
  });

  final List<ChartColumn> columns;

  /// The colour of each part, bottom up.
  final List<Color> colors;

  /// The axis label of a value, such as `$0.25`.
  final String Function(int value) axis;

  @override
  State<TurnChart> createState() => _TurnChartState();
}

class _TurnChartState extends State<TurnChart> {
  int? _hovered;

  int? _columnAt(double x, double width) {
    final plot = width - _axisWidth;
    if (widget.columns.isEmpty || x < _axisWidth || plot <= 0) return null;
    final index = ((x - _axisWidth) / (plot / widget.columns.length)).floor();
    return index.clamp(0, widget.columns.length - 1);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return LayoutBuilder(
      builder: (context, box) {
        final width = box.maxWidth;
        final hovered = _hovered;
        final column = hovered == null ? null : widget.columns[hovered];
        final slot = (width - _axisWidth) / math.max(1, widget.columns.length);
        return MouseRegion(
          onHover: (event) => setState(
            () => _hovered = _columnAt(event.localPosition.dx, width),
          ),
          onExit: (_) => setState(() => _hovered = null),
          child: SizedBox(
            height: _height,
            width: width,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned.fill(
                  child: CustomPaint(
                    painter: _ChartPainter(
                      columns: widget.columns,
                      colors: widget.colors,
                      axis: widget.axis,
                      hovered: hovered,
                      ink: ws.textFaint,
                      grid: ws.border,
                      hover: ws.hover,
                    ),
                  ),
                ),
                if (column != null && hovered != null)
                  Positioned(
                    top: 6,
                    left: math.max(
                      0,
                      math.min(
                        _axisWidth + slot * (hovered + 0.5) + 12,
                        width - _tipWidth,
                      ),
                    ),
                    child: _Tip(tip: column.tip, ws: ws),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

const _tipWidth = 196.0;

class _Tip extends StatelessWidget {
  const _Tip({required this.tip, required this.ws});

  final ChartTip tip;
  final WsPalette ws;

  @override
  Widget build(BuildContext context) {
    final style = TextStyle(
      fontSize: WsMetrics.fsSmall,
      color: ws.canvas,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    return Container(
      width: _tipWidth,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: ws.text,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: DefaultTextStyle.merge(
        style: style,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: 2,
          children: [
            Text(
              tip.title,
              style: style.copyWith(fontWeight: WsMetrics.fwSemibold),
            ),
            for (final row in tip.rows)
              Row(
                spacing: 6,
                children: [
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                      color: row.color,
                      borderRadius: BorderRadius.circular(2),
                    ),
                  ),
                  Expanded(
                    child: Text(
                      row.label,
                      overflow: TextOverflow.ellipsis,
                      maxLines: 1,
                    ),
                  ),
                  Text(row.value),
                ],
              ),
            for (final note in tip.notes)
              Text(
                note,
                style: style.copyWith(color: ws.canvas.withValues(alpha: 0.72)),
              ),
          ],
        ),
      ),
    );
  }
}

class _ChartPainter extends CustomPainter {
  _ChartPainter({
    required this.columns,
    required this.colors,
    required this.axis,
    required this.hovered,
    required this.ink,
    required this.grid,
    required this.hover,
  });

  final List<ChartColumn> columns;
  final List<Color> colors;
  final String Function(int value) axis;
  final int? hovered;
  final Color ink;
  final Color grid;
  final Color hover;

  @override
  void paint(Canvas canvas, Size size) {
    final totals = [
      for (final column in columns)
        column.parts.fold(0, (sum, part) => sum + part),
    ];
    final top = niceTop(totals.fold(0, math.max));
    final baseline = _height - 22;
    final plotHeight = baseline - _top;
    final plotLeft = _axisWidth;
    final plotWidth = size.width - plotLeft;

    // Three gridlines: zero, half and the top of the axis.
    for (final value in [0, top ~/ 2, top]) {
      final y = baseline - (top == 0 ? 0 : plotHeight * value / top);
      canvas.drawLine(
        Offset(plotLeft, y),
        Offset(size.width, y),
        Paint()
          ..color = grid
          ..strokeWidth = 1,
      );
      _text(
        canvas,
        value == 0 ? axis(0) : axis(value),
        Offset(plotLeft - 8, y),
        alignRight: true,
      );
    }
    if (columns.isEmpty) return;

    final slot = plotWidth / columns.length;
    final columnWidth = math.min(22.0, slot * 0.5);
    final labelEvery = (columns.length / 12).ceil();
    for (var index = 0; index < columns.length; index++) {
      final centre = plotLeft + slot * (index + 0.5);
      if (hovered == index) {
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(
              plotLeft + slot * index + 3,
              _top - 2,
              slot - 6,
              baseline - _top + 2,
            ),
            const Radius.circular(6),
          ),
          Paint()..color = hover,
        );
      }
      var bottom = baseline;
      final parts = columns[index].parts;
      for (var part = 0; part < parts.length; part++) {
        if (parts[part] <= 0) continue;
        final height = top == 0 ? 0.0 : plotHeight * parts[part] / top;
        final rect = Rect.fromLTWH(
          centre - columnWidth / 2,
          bottom - height,
          columnWidth,
          height,
        );
        canvas.drawRRect(
          RRect.fromRectAndRadius(rect, const Radius.circular(3)),
          Paint()..color = colors[part],
        );
        bottom -= height;
      }
      if (index % labelEvery == 0 || index == columns.length - 1) {
        _text(canvas, '${index + 1}', Offset(centre, _labelsTop), centre: true);
      }
    }
  }

  void _text(
    Canvas canvas,
    String text,
    Offset at, {
    bool alignRight = false,
    bool centre = false,
  }) {
    final painter = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(
          fontSize: 11,
          color: ink,
          fontFeatures: const [FontFeature.tabularFigures()],
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    final dx = alignRight
        ? at.dx - painter.width
        : centre
        ? at.dx - painter.width / 2
        : at.dx;
    painter.paint(canvas, Offset(dx, at.dy - painter.height / 2));
  }

  @override
  bool shouldRepaint(_ChartPainter old) => true;
}
