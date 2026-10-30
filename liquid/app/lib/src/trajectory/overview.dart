import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../theme/icons.dart';
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

/// The part of the axis the list shows, or the range it is limited to, and how to read it.
@immutable
class Brush {
  const Brush(this.start, this.end, this.label, {this.range = false});

  final double start;
  final double end;

  /// Which steps it covers, for assistive technology.
  final String label;

  /// Whether it is a range that limits the list rather than what the list shows.
  final bool range;
}

/// The Trajectory's single top section: every step on one time axis (approved navigation
/// mockup). A brush marks what the list shows; its handles limit the list to a range, dragging
/// it scrolls the list, a click jumps to the step there and scrolling zooms around the pointer.
class Overview extends StatefulWidget {
  const Overview({
    super.key,
    required this.tracks,
    required this.span,
    required this.equalWidth,
    required this.window,
    required this.onWindow,
    this.brush,
    this.onRange,
    this.onPan,
    this.onJump,
    this.matches,
    this.mark,
    this.legend = false,
  });

  final List<OverviewTrack> tracks;

  /// The axis length: milliseconds, or steps with `equalWidth`.
  final double span;
  final bool equalWidth;

  /// The zoomed part of the axis; `null` shows all of it.
  final (double, double)? window;
  final ValueChanged<(double, double)?> onWindow;

  /// What the list shows or the range limiting it; none while comparing.
  final ValueListenable<Brush?>? brush;

  /// A handle set the range, or cleared it with `null`.
  final ValueChanged<(double, double)?>? onRange;

  /// Dragging the brush asks the list to start at this point of the axis.
  final ValueChanged<double>? onPan;

  /// A click at this point of the axis.
  final ValueChanged<double>? onJump;

  /// Which records match the filters; the others draw faint.
  final List<bool>? matches;

  /// A diamond on the axis: the selected step or the branch point.
  final double? mark;

  /// Whether to explain the colors below the tracks.
  final bool legend;

  @override
  State<Overview> createState() => _OverviewState();
}

class _OverviewState extends State<Overview> {
  /// The trackpad pinch scale seen last during the current gesture.
  double _lastScale = 1;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final Overview(
      :tracks,
      :span,
      :equalWidth,
      :window,
      :onWindow,
      :brush,
      :onJump,
      :matches,
      :mark,
      :legend,
    ) = widget;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final visible = window ?? (0, math.max(span, 1));
    return Container(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        WsMetrics.space3,
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
          LayoutBuilder(
            builder: (context, constraints) {
              final width =
                  constraints.maxWidth - 56 - WsMetrics.space2 - _infoSize;
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
                      child: Stack(
                        children: [
                          GestureDetector(
                            key: const Key('overview-tracks'),
                            behavior: HitTestBehavior.opaque,
                            onTapUp: onJump == null
                                ? null
                                : (details) =>
                                      onJump(at(details.localPosition.dx)),
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
                                          matches: matches,
                                          palette: ws,
                                        ),
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                          ),
                          if (brush != null)
                            ValueListenableBuilder(
                              valueListenable: brush,
                              builder: (context, brush, _) => brush == null
                                  ? const SizedBox.shrink()
                                  : _BrushView(
                                      brush: brush,
                                      width: width,
                                      visible: visible,
                                      at: at,
                                      onRange: widget.onRange,
                                      onPan: widget.onPan,
                                    ),
                            ),
                        ],
                      ),
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  Tooltip(
                    message:
                        'drag a handle to limit the list · click to jump · '
                        'scroll to zoom',
                    child: Icon(
                      LucideIcons.info,
                      size: _infoSize,
                      color: ws.textFaint,
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
                    Text(
                      label,
                      style: small.copyWith(fontSize: WsMetrics.fsCaption),
                    ),
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

/// The size of the hint's icon at the end of the axis.
const _infoSize = 14.0;

/// The brush over the tracks with a handle at each end (approved navigation mockup).
class _BrushView extends StatelessWidget {
  const _BrushView({
    required this.brush,
    required this.width,
    required this.visible,
    required this.at,
    required this.onRange,
    required this.onPan,
  });

  final Brush brush;
  final double width;
  final (double, double) visible;

  /// The axis point at a horizontal position.
  final double Function(double x) at;
  final ValueChanged<(double, double)?>? onRange;
  final ValueChanged<double>? onPan;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    double x(double value) =>
        ((value - visible.$1) / (visible.$2 - visible.$1) * width).clamp(
          0.0,
          width,
        );
    final left = x(brush.start);
    final right = math.max(x(brush.end), left + 2);
    // At least a hundredth of the shown axis, so the handles never cross.
    final gap = (visible.$2 - visible.$1) / 100;
    Widget handle(Key key, double center, bool start) => Positioned(
      left: (center - 4).clamp(0.0, math.max(0.0, width - 8)),
      top: 14 + WsMetrics.space2,
      bottom: 0,
      width: 8,
      child: GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        dragStartBehavior: DragStartBehavior.down,
        onDoubleTap: onRange == null ? null : () => onRange!(null),
        onHorizontalDragUpdate: onRange == null
            ? null
            : (details) {
                // The brush's own box, not the moving handle's, places the pointer.
                final box = context.findRenderObject()! as RenderBox;
                final value = at(box.globalToLocal(details.globalPosition).dx);
                onRange!(
                  start
                      ? (math.min(value, brush.end - gap), brush.end)
                      : (brush.start, math.max(value, brush.start + gap)),
                );
              },
        child: Center(
          child: Container(
            width: 7,
            height: 18,
            decoration: BoxDecoration(
              color: ws.surface,
              border: Border.all(color: ws.accent),
              borderRadius: BorderRadius.circular(3),
            ),
          ),
        ),
      ),
    );
    return Positioned.fill(
      child: Stack(
        children: [
          Positioned(
            left: left,
            width: right - left,
            top: 14 + WsMetrics.space2 - 3,
            bottom: 0,
            child: Semantics(
              key: const Key('range-brush'),
              container: true,
              label: brush.label,
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onHorizontalDragUpdate: onPan == null
                    ? null
                    : (details) => onPan!(
                        brush.start +
                            details.delta.dx /
                                width *
                                (visible.$2 - visible.$1),
                      ),
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: ws.accent.withValues(alpha: 0.08),
                    border: Border.all(
                      color: ws.accent.withValues(alpha: 0.55),
                    ),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ),
            ),
          ),
          handle(const Key('range-start'), left, true),
          handle(const Key('range-end'), right, false),
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
          style: TextStyle(
            color: palette.textFaint,
            fontSize: WsMetrics.fsCaption,
            height: 1,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      final left = (x(tick * unit) - text.width / 2)
          .clamp(0.0, math.max(0.0, size.width - text.width))
          .toDouble();
      text
        ..paint(canvas, Offset(left, 0))
        ..dispose();
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
    required this.matches,
    required this.palette,
  });

  final List<Segment> segments;
  final (double, double) visible;

  /// Which records match the filters; the others draw faint.
  final List<bool>? matches;
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
      final faint = matches != null && !matches![segment.index];
      if (faint) {
        canvas.saveLayer(
          rect.inflate(1),
          Paint()..color = const Color(0x52000000),
        );
      }
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
      if (faint) canvas.restore();
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
      old.matches != matches ||
      old.palette != palette;
}
