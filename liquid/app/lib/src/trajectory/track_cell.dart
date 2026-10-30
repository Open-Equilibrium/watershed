import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../format.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'context_track.dart';

/// A context window's size, such as `200k` or `1M`.
String formatWindow(int tokens) => tokens >= 1000000 && tokens % 1000000 == 0
    ? '${tokens ~/ 1000000}M'
    : tokens % 1000 == 0
    ? '${tokens ~/ 1000}k'
    : formatCompact(tokens);

/// One row's cell of the context track (approved look mockup): a bar to the step's context,
/// light when estimated, a line through the bars' ends, pi's compaction point as a dashed line
/// and ··· for a collapsed turn.
class TrackCell extends StatelessWidget {
  const TrackCell({
    super.key,
    required this.level,
    required this.scale,
    this.previous,
    this.next,
    this.compaction,
    this.window,
    this.bar = true,
    this.collapsed = false,
  });

  /// The context at this row.
  final ContextLevel level;

  /// Tokens at the cell's full width.
  final double scale;

  /// The rows around it, which the line joins; none where the line breaks.
  final int? previous;
  final int? next;

  /// Where pi compacts, drawn only on the window's scale.
  final int? compaction;

  /// The model's context window, when known.
  final int? window;

  /// Whether the row draws a bar; turn lines and approvals only let the line pass.
  final bool bar;
  final bool collapsed;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final x = level.tokens / scale;
    final cell = CustomPaint(
      painter: _TrackPainter(
        level: collapsed ? null : level,
        previous: previous,
        next: next,
        scale: scale,
        compaction: compaction,
        bar: bar,
        color: ws.accent,
        mark: ws.warning,
      ),
      child: collapsed
          ? LayoutBuilder(
              builder: (context, constraints) => Padding(
                padding: EdgeInsets.only(
                  left: math.min(x, 1) * constraints.maxWidth * 0.9,
                ),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    '···',
                    style: TextStyle(
                      color: ws.textFaint,
                      fontSize: WsMetrics.fsSmall,
                    ),
                  ),
                ),
              ),
            )
          : const SizedBox.expand(),
    );
    if (!bar || collapsed) return cell;
    final window = this.window;
    return Semantics(
      container: true,
      label:
          'Context ${level.exact ? '' : '≈'}${formatCompact(level.tokens)}'
          '${window == null ? '' : ' of ${formatWindow(window)}'}',
      child: cell,
    );
  }
}

class _TrackPainter extends CustomPainter {
  _TrackPainter({
    required this.level,
    required this.previous,
    required this.next,
    required this.scale,
    required this.compaction,
    required this.bar,
    required this.color,
    required this.mark,
  });

  final ContextLevel? level;
  final int? previous;
  final int? next;
  final double scale;
  final int? compaction;
  final bool bar;
  final Color color;
  final Color mark;

  @override
  void paint(Canvas canvas, Size size) {
    double x(num tokens) => (tokens / scale * size.width).clamp(0, size.width);
    final mid = size.height / 2;
    if (compaction case final compaction?) {
      final paint = Paint()
        ..color = mark
        ..strokeWidth = 1;
      for (var y = 0.0; y < size.height; y += 5) {
        canvas.drawLine(
          Offset(x(compaction), y),
          Offset(x(compaction), math.min(y + 3, size.height)),
          paint,
        );
      }
    }
    final level = this.level;
    if (level == null) return;
    final end = x(level.tokens);
    if (bar) {
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromLTWH(0, mid - 4, math.max(end, 1.5), 8),
          const Radius.circular(2),
        ),
        Paint()..color = color.withValues(alpha: level.exact ? 1 : 0.38),
      );
    }
    final line = Paint()
      ..color = color
      ..strokeWidth = 1.5;
    if (previous case final previous?) {
      canvas.drawLine(
        Offset(x((previous + level.tokens) / 2), 0),
        Offset(end, mid),
        line,
      );
    }
    if (next case final next?) {
      canvas.drawLine(
        Offset(end, mid),
        Offset(x((level.tokens + next) / 2), size.height),
        line,
      );
    }
  }

  @override
  bool shouldRepaint(_TrackPainter old) =>
      old.level?.tokens != level?.tokens ||
      old.level?.exact != level?.exact ||
      old.previous != previous ||
      old.next != next ||
      old.scale != scale ||
      old.compaction != compaction ||
      old.bar != bar ||
      old.color != color ||
      old.mark != mark;
}
