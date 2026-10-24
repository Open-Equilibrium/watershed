import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../agent/action_area.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'trajectory_model.dart';

/// One track of the overview, such as `Model` or a compared branch.
@immutable
class OverviewTrack {
  const OverviewTrack(this.label, this.segments);

  final String label;
  final List<Segment> segments;
}

/// The Trajectory's single top section: every step on one time axis. Dragging focuses a range,
/// scrolling zooms around the pointer.
class Overview extends StatefulWidget {
  const Overview({
    super.key,
    required this.summary,
    required this.tracks,
    required this.span,
    required this.equalWidth,
    required this.window,
    required this.range,
    required this.rangeLabel,
    required this.onRange,
    required this.onWindow,
    this.mark,
    this.legend = false,
  });

  final String summary;
  final List<OverviewTrack> tracks;

  /// The axis length: milliseconds, or steps with `equalWidth`.
  final double span;
  final bool equalWidth;

  /// The zoomed part of the axis; `null` shows all of it.
  final (double, double)? window;

  /// The focused range.
  final (double, double)? range;
  final String? rangeLabel;
  final ValueChanged<(double, double)?> onRange;
  final ValueChanged<(double, double)?> onWindow;

  /// A diamond on the axis: the selected step or the branch point.
  final double? mark;

  /// Whether to explain the colors below the tracks.
  final bool legend;

  @override
  State<Overview> createState() => _OverviewState();
}

class _OverviewState extends State<Overview> {
  /// Where the current drag started on the axis.
  double? _anchor;

  /// The trackpad pinch scale seen last during the current gesture.
  double _lastScale = 1;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final Overview(
      :summary,
      :tracks,
      :span,
      :equalWidth,
      :window,
      :range,
      :rangeLabel,
      :onRange,
      :onWindow,
      :mark,
      :legend,
    ) = widget;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final visible = window ?? (0, math.max(span, 1));
    return Container(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        10,
        WsMetrics.space5,
        WsMetrics.space4,
      ),
      decoration: BoxDecoration(
        color: ws.subtle,
        border: Border.symmetric(horizontal: BorderSide(color: ws.border)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Text(
                'Overview',
                style: small.copyWith(
                  color: ws.textMuted,
                  fontWeight: WsMetrics.fwMedium,
                ),
              ),
              const SizedBox(width: 14),
              Flexible(
                child: Text(
                  summary,
                  style: small,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              const SizedBox(width: 14),
              Flexible(
                child: Text(
                  'drag to focus · scroll to zoom',
                  style: small,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              const Spacer(),
              if (rangeLabel != null) ...[
                Text(
                  rangeLabel,
                  key: const Key('range'),
                  style: small.copyWith(color: ws.accent),
                ),
                const SizedBox(width: WsMetrics.space2),
                Hint('esc'),
              ],
            ],
          ),
          const SizedBox(height: WsMetrics.space3),
          LayoutBuilder(
            builder: (context, constraints) {
              final width = constraints.maxWidth - 56;
              double at(double x) =>
                  visible.$1 +
                  (x.clamp(0, width) / width) * (visible.$2 - visible.$1);
              // Zooms around `x`; positive `delta` zooms out.
              void zoom(double x, double delta, {double? scale}) {
                if (scale != null) _lastScale = scale;
                final pivot = at(x);
                final factor = math.exp(delta / 500);
                final length = ((visible.$2 - visible.$1) * factor).clamp(
                  span / 1000,
                  span,
                );
                final fraction =
                    (pivot - visible.$1) / (visible.$2 - visible.$1);
                final start = (pivot - fraction * length).clamp(
                  0.0,
                  span - length,
                );
                onWindow(length >= span ? null : (start, start + length));
              }

              return Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                    width: 56,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const SizedBox(height: 14),
                        for (final track in tracks)
                          Padding(
                            padding: const EdgeInsets.only(
                              top: WsMetrics.space2,
                            ),
                            child: SizedBox(
                              height: 12,
                              child: Text(
                                track.label,
                                style: small.copyWith(height: 1),
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                  Expanded(
                    child: Listener(
                      // A mouse wheel scrolls; a trackpad pans and pinches.
                      onPointerSignal: (signal) {
                        if (signal is PointerScrollEvent) {
                          zoom(signal.localPosition.dx, signal.scrollDelta.dy);
                        }
                      },
                      onPointerPanZoomUpdate: (update) => zoom(
                        update.localPosition.dx,
                        update.localPanDelta.dy -
                            500 * math.log(update.scale / _lastScale),
                        scale: update.scale,
                      ),
                      onPointerPanZoomEnd: (_) => _lastScale = 1,
                      child: GestureDetector(
                        key: const Key('overview-tracks'),
                        behavior: HitTestBehavior.opaque,
                        dragStartBehavior: DragStartBehavior.down,
                        // Trackpad gestures zoom; a pressed trackpad drags as a mouse.
                        supportedDevices: const {
                          PointerDeviceKind.mouse,
                          PointerDeviceKind.touch,
                          PointerDeviceKind.stylus,
                          PointerDeviceKind.invertedStylus,
                        },
                        onTap: () => onRange(null),
                        onHorizontalDragStart: (details) =>
                            _anchor = at(details.localPosition.dx),
                        onHorizontalDragEnd: (_) => _anchor = null,
                        onHorizontalDragUpdate: (details) {
                          final from = _anchor;
                          if (from == null) return;
                          final to = at(details.localPosition.dx);
                          onRange((math.min(from, to), math.max(from, to)));
                        },
                        child: Column(
                          children: [
                            SizedBox(
                              height: 14,
                              width: double.infinity,
                              child: CustomPaint(
                                painter: _AxisPainter(
                                  visible: visible,
                                  equalWidth: equalWidth,
                                  mark: mark,
                                  palette: ws,
                                ),
                              ),
                            ),
                            for (final track in tracks)
                              Padding(
                                padding: const EdgeInsets.only(
                                  top: WsMetrics.space2,
                                ),
                                child: SizedBox(
                                  height: 12,
                                  width: double.infinity,
                                  child: CustomPaint(
                                    painter: _TrackPainter(
                                      segments: track.segments,
                                      visible: visible,
                                      range: range,
                                      palette: ws,
                                    ),
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ],
              );
            },
          ),
          if (legend)
            Padding(
              padding: const EdgeInsets.only(left: 56, top: WsMetrics.space2),
              child: Row(
                children: [
                  for (final (label, color) in [
                    ('model', ws.accent),
                    ('tool', ws.data2),
                    ('waiting for you', ws.warning),
                  ]) ...[
                    Container(
                      width: 8,
                      height: 8,
                      decoration: BoxDecoration(
                        color: color,
                        borderRadius: BorderRadius.circular(2),
                      ),
                    ),
                    const SizedBox(width: WsMetrics.space2),
                    Text(label, style: small.copyWith(fontSize: 11)),
                    const SizedBox(width: WsMetrics.space5),
                  ],
                ],
              ),
            ),
        ],
      ),
    );
  }
}

class _AxisPainter extends CustomPainter {
  _AxisPainter({
    required this.visible,
    required this.equalWidth,
    required this.mark,
    required this.palette,
  });

  final (double, double) visible;
  final bool equalWidth;
  final double? mark;
  final WsPalette palette;

  @override
  void paint(Canvas canvas, Size size) {
    final length = visible.$2 - visible.$1;
    double x(double value) => (value - visible.$1) / length * size.width;
    canvas.drawLine(
      Offset(0, size.height - 0.5),
      Offset(size.width, size.height - 0.5),
      Paint()..color = palette.border,
    );
    final step = _tickStep(length / (equalWidth ? 1 : 1000), equalWidth);
    final unit = equalWidth ? 1.0 : 1000.0;
    var tick = (visible.$1 / unit / step).ceil() * step;
    while (tick * unit <= visible.$2) {
      final label = equalWidth ? '#${tick.round() + 1}' : '${_trim(tick)} s';
      final text = TextPainter(
        text: TextSpan(
          text: label,
          style: TextStyle(color: palette.textFaint, fontSize: 11, height: 1),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      final left = (x(tick * unit) - text.width / 2)
          .clamp(0.0, math.max(0.0, size.width - text.width))
          .toDouble();
      text.paint(canvas, Offset(left, 0));
      tick += step;
    }
    final mark = this.mark;
    if (mark != null && mark >= visible.$1 && mark <= visible.$2) {
      final center = Offset(x(mark), size.height + 0.5);
      final diamond = Path()
        ..moveTo(center.dx, center.dy - 4.5)
        ..lineTo(center.dx + 4.5, center.dy)
        ..lineTo(center.dx, center.dy + 4.5)
        ..lineTo(center.dx - 4.5, center.dy)
        ..close();
      canvas.drawPath(diamond, Paint()..color = palette.canvas);
      canvas.drawPath(
        diamond,
        Paint()
          ..color = palette.accent
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
    }
  }

  static double _tickStep(double length, bool equalWidth) {
    const steps = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800];
    for (final step in steps) {
      if (step < 1 && equalWidth) continue;
      if (length / step <= 6) return step.toDouble();
    }
    return (length / 6).ceilToDouble();
  }

  static String _trim(double value) =>
      value == value.roundToDouble() ? '${value.round()}' : '$value';

  @override
  bool shouldRepaint(_AxisPainter old) =>
      old.visible != visible ||
      old.equalWidth != equalWidth ||
      old.mark != mark ||
      old.palette != palette;
}

class _TrackPainter extends CustomPainter {
  _TrackPainter({
    required this.segments,
    required this.visible,
    required this.range,
    required this.palette,
  });

  final List<Segment> segments;
  final (double, double) visible;
  final (double, double)? range;
  final WsPalette palette;

  @override
  void paint(Canvas canvas, Size size) {
    final length = visible.$2 - visible.$1;
    double x(double value) => (value - visible.$1) / length * size.width;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    final bar = Rect.fromLTRB(0, 1, 0, size.height - 1);
    for (final segment in segments) {
      if (segment.end < visible.$1 || segment.start > visible.$2) continue;
      final left = x(segment.start);
      final right = math.max(x(segment.end), left + 2);
      final rect = Rect.fromLTRB(left, bar.top, right, bar.bottom);
      switch (segment.lane) {
        case OverviewLane.model:
          final firstToken = segment.firstToken;
          final split = firstToken == null
              ? left
              : x(firstToken).clamp(left, right);
          _bar(
            canvas,
            Rect.fromLTRB(left, rect.top, split, rect.bottom),
            palette.accent.withValues(alpha: 0.35),
          );
          _bar(
            canvas,
            Rect.fromLTRB(split, rect.top, right, rect.bottom),
            palette.accent,
          );
        case OverviewLane.tools:
          _bar(canvas, rect, palette.data2);
        case OverviewLane.you:
          canvas.save();
          canvas.clipRRect(
            RRect.fromRectAndRadius(rect, const Radius.circular(2)),
          );
          final stripe = Paint()
            ..color = palette.warning
            ..strokeWidth = 2.2;
          for (var at = rect.left - rect.height; at < rect.right; at += 6) {
            canvas.drawLine(
              Offset(at, rect.bottom),
              Offset(at + rect.height, rect.top),
              stripe,
            );
          }
          canvas.restore();
      }
    }
    final range = this.range;
    if (range != null) {
      final brush = Rect.fromLTRB(x(range.$1), 0, x(range.$2), size.height);
      canvas.drawRect(
        brush,
        Paint()..color = palette.accent.withValues(alpha: 0.1),
      );
      final edge = Paint()
        ..color = palette.accent
        ..strokeWidth = 2;
      canvas.drawLine(brush.topLeft, brush.bottomLeft, edge);
      canvas.drawLine(brush.topRight, brush.bottomRight, edge);
    }
    canvas.restore();
  }

  void _bar(Canvas canvas, Rect rect, Color color) {
    if (rect.width <= 0) return;
    canvas.drawRRect(
      RRect.fromRectAndRadius(rect, const Radius.circular(2)),
      Paint()..color = color,
    );
  }

  @override
  bool shouldRepaint(_TrackPainter old) =>
      old.segments != segments ||
      old.visible != visible ||
      old.range != range ||
      old.palette != palette;
}
