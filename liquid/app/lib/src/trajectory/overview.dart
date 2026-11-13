import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';

import '../agent/input_lane.dart';
import '../rust/api/history.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'step_parts.dart';
import 'trajectory_model.dart';

/// One track of the overview, such as `Model` or a compared branch.
@immutable
class OverviewTrack {
  const OverviewTrack(this.label, this.segments);

  final String label;
  final List<Segment> segments;
}

/// The record a range handle sits on: its index, how many records there are and its time.
@immutable
class HandleStep {
  const HandleStep(this.index, this.count, this.time);

  /// Zero-based.
  final int index;
  final int count;
  final String time;

  /// The tip while the handle has the focus, such as `Step 31 of 48 · 51.2 s`.
  String get tip => 'Step ${index + 1} of $count · $time';

  /// What assistive technology reads as the handle's value, such as `step 31 of 48, 51.2 s`.
  String get spoken => 'step ${index + 1} of $count, $time';

  @override
  bool operator ==(Object other) =>
      other is HandleStep &&
      other.index == index &&
      other.count == count &&
      other.time == time;

  @override
  int get hashCode => Object.hash(index, count, time);
}

/// The part of the axis the list shows, or the range it is limited to, and how to read it.
@immutable
class Brush {
  const Brush(
    this.start,
    this.end,
    this.label, {
    this.range = false,
    required this.startStep,
    required this.endStep,
  });

  final double start;
  final double end;

  /// Which steps it covers, for assistive technology.
  final String label;

  /// Whether it is a range that limits the list rather than what the list shows.
  final bool range;

  /// The records the start and the end handle sit on.
  final HandleStep startStep;
  final HandleStep endStep;

  @override
  bool operator ==(Object other) =>
      other is Brush &&
      other.start == start &&
      other.end == end &&
      other.label == label &&
      other.range == range &&
      other.startStep == startStep &&
      other.endStep == endStep;

  @override
  int get hashCode => Object.hash(start, end, label, range, startStep, endStep);
}

/// The Trajectory's single top section: every step on one time axis. A brush marks what the list
/// shows; its handles limit the list to a range, dragging it scrolls the list, a click jumps to the
/// step there and scrolling zooms around the pointer.
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
    this.onHandleStep,
    this.handleStep,
    this.stepExtent,
    this.onLeave,
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

  /// A range handle moved to the start (`true`) or the end of record `step`.
  final void Function(bool start, int step)? onHandleStep;

  /// What a range handle says at record `index`, for the values assistive technology previews;
  /// null past the records shown.
  final HandleStep? Function(int index)? handleStep;

  /// Where record `index` lies on the axis, which decides whether a handle's key moves it past
  /// that record; null past the records shown.
  final (double, double)? Function(int index)? stepExtent;

  /// Esc on a focused range handle, which hands the keyboard back to the list.
  final VoidCallback? onLeave;

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
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final visible = widget.window ?? (0, math.max(widget.span, 1));
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
                  constraints.maxWidth -
                  _labelWidth -
                  WsMetrics.space2 -
                  _infoSize;
              double at(double x) =>
                  visible.$1 +
                  (x.clamp(0, width) / width) * (visible.$2 - visible.$1);
              return Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  _labels(small),
                  Expanded(child: _plot(visible, width, at)),
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
          if (widget.legend) _legend(ws, small),
        ],
      ),
    );
  }

  /// The track labels, beside the tracks and under the empty axis.
  Widget _labels(TextStyle small) => SizedBox(
    width: _labelWidth,
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SizedBox(height: _axisHeight),
        for (final track in widget.tracks)
          Padding(
            padding: const EdgeInsets.only(top: WsMetrics.space2),
            child: SizedBox(
              height: _trackHeight,
              child: Text(track.label, style: small.copyWith(height: 1)),
            ),
          ),
      ],
    ),
  );

  /// The axis and tracks, with the brush over them; a wheel or trackpad zooms around the pointer.
  Widget _plot(
    (double, double) visible,
    double width,
    double Function(double) at,
  ) {
    final span = widget.span;
    // Zooms around `x`; positive `delta` zooms out.
    void zoom(double x, double delta, {double? scale}) {
      if (scale != null) _lastScale = scale;
      final pivot = at(x);
      final factor = math.exp(delta / 500);
      final length = ((visible.$2 - visible.$1) * factor).clamp(
        span / 1000,
        span,
      );
      final fraction = (pivot - visible.$1) / (visible.$2 - visible.$1);
      final start = (pivot - fraction * length).clamp(0.0, span - length);
      widget.onWindow(length >= span ? null : (start, start + length));
    }

    return Listener(
      // A mouse wheel scrolls; a trackpad pans and pinches.
      onPointerSignal: (signal) {
        if (signal is PointerScrollEvent) {
          zoom(signal.localPosition.dx, signal.scrollDelta.dy);
        }
      },
      onPointerPanZoomUpdate: (update) => zoom(
        update.localPosition.dx,
        update.localPanDelta.dy - 500 * math.log(update.scale / _lastScale),
        scale: update.scale,
      ),
      onPointerPanZoomEnd: (_) => _lastScale = 1,
      // A focused handle's tip hangs over the axis, past the plot's edges at the ends.
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          _paths(visible, at),
          // The brush moves with every scroll of the list; its own layer
          // keeps the tracks from being painted again each time.
          if (widget.brush case final brush?)
            Positioned.fill(
              child: RepaintBoundary(
                child: ValueListenableBuilder(
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
                          onJump: widget.onJump,
                          onHandleStep: widget.onHandleStep,
                          handleStep: widget.handleStep,
                          stepExtent: widget.stepExtent,
                          onLeave: widget.onLeave,
                        ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// The axis and each track painted over the time axis; a click on them jumps to that point.
  Widget _paths((double, double) visible, double Function(double) at) {
    final ws = context.ws;
    final Overview(:tracks, :equalWidth, :onJump, :matches, :mark) = widget;
    return GestureDetector(
      key: const Key('overview-tracks'),
      behavior: HitTestBehavior.opaque,
      onTapUp: onJump == null
          ? null
          : (details) => onJump(at(details.localPosition.dx)),
      child: Column(
        children: [
          SizedBox(
            height: _axisHeight,
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
              padding: const EdgeInsets.only(top: WsMetrics.space2),
              child: SizedBox(
                height: _trackHeight,
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
    );
  }

  /// Explains the two colors of the tracks below them.
  Widget _legend(WsPalette ws, TextStyle small) => Padding(
    padding: const EdgeInsets.only(left: _labelWidth, top: WsMetrics.space2),
    child: Row(
      children: [
        for (final (label, kind) in [
          ('model', BridgeRecordKind.message),
          ('tool', BridgeRecordKind.tool),
        ]) ...[
          Container(
            width: WsMetrics.space3,
            height: WsMetrics.space3,
            decoration: BoxDecoration(
              color: kindColor(ws, kind),
              borderRadius: BorderRadius.circular(2),
            ),
          ),
          const SizedBox(width: WsMetrics.space2),
          Text(label, style: small.copyWith(fontSize: WsMetrics.fsCaption)),
          const SizedBox(width: WsMetrics.space5),
        ],
      ],
    ),
  );
}

/// The track labels' column, as the approved mockup draws it; no token is this wide.
const _labelWidth = 56.0;

/// The time axis, one caption line, and each track.
const _axisHeight = WsMetrics.lhCaption;
const _trackHeight = WsMetrics.space4;

/// The size of the hint's icon at the end of the axis.
const _infoSize = _axisHeight;

/// The width a range handle takes clicks in.
const _handleWidth = WsMetrics.space3;

/// The brush over the tracks with a handle at each end.
class _BrushView extends StatefulWidget {
  const _BrushView({
    required this.brush,
    required this.width,
    required this.visible,
    required this.at,
    required this.onRange,
    required this.onPan,
    required this.onJump,
    required this.onHandleStep,
    required this.handleStep,
    required this.stepExtent,
    required this.onLeave,
  });

  final Brush brush;
  final double width;
  final (double, double) visible;

  /// The axis point at a horizontal position.
  final double Function(double x) at;
  final ValueChanged<(double, double)?>? onRange;
  final ValueChanged<double>? onPan;

  /// A click on the brush jumps as one on the tracks beneath it does.
  final ValueChanged<double>? onJump;
  final void Function(bool start, int step)? onHandleStep;
  final HandleStep? Function(int index)? handleStep;
  final (double, double)? Function(int index)? stepExtent;
  final VoidCallback? onLeave;

  @override
  State<_BrushView> createState() => _BrushViewState();
}

class _BrushViewState extends State<_BrushView> {
  /// Where a drag of the brush started: its start on the axis and the pointer's position, so
  /// small moves add up instead of snapping back to the same step.
  (double, double)? _drag;

  /// The keyboard focus of each handle, in the order Tab takes them.
  final _startFocus = FocusNode(debugLabel: 'range start');
  final _endFocus = FocusNode(debugLabel: 'range end');

  @override
  void initState() {
    super.initState();
    _startFocus.addListener(_focusChanged);
    _endFocus.addListener(_focusChanged);
  }

  @override
  void dispose() {
    _startFocus.removeListener(_focusChanged);
    _endFocus.removeListener(_focusChanged);
    _startFocus.dispose();
    _endFocus.dispose();
    super.dispose();
  }

  void _focusChanged() => setState(() {});

  double _pointer(Offset global) =>
      (context.findRenderObject()! as RenderBox).globalToLocal(global).dx;

  /// The record a handle reaches by `delta` records, never past the other handle or the first and
  /// last record. A record that would not move the handle on from its edge in the key's direction,
  /// or would reverse the range, is skipped for the next one that does; a handle at its limit, or
  /// with no such record, stays where it is.
  int _reach(bool start, int delta) {
    final brush = widget.brush;
    final own = (start ? brush.startStep : brush.endStep).index;
    final other = (start ? brush.endStep : brush.startStep).index;
    final low = start ? 0 : other;
    final high = start ? other : brush.startStep.count - 1;
    final target = math.max(low, math.min(high, own + delta));
    if (target == own) return own;
    final step = delta.sign;
    for (var index = target; index >= low && index <= high; index += step) {
      if (_moves(start, index, step)) return index;
    }
    for (var index = target - step; index != own; index -= step) {
      if (_moves(start, index, step)) return index;
    }
    return own;
  }

  /// Whether the handle moves to record `index` going `step` ways: its edge lies beyond the
  /// range's edge that way, and the range stays in order.
  bool _moves(bool start, int index, int step) {
    final brush = widget.brush;
    final extent = widget.stepExtent?.call(index);
    if (extent == null) return false;
    final (from, to) = extent;
    if (start) {
      return from <= brush.end &&
          (step > 0 ? from > brush.start : from < brush.start);
    }
    return to >= brush.start && (step > 0 ? to > brush.end : to < brush.end);
  }

  /// Moves a handle `delta` records, as [_reach] allows.
  void _shift(bool start, int delta) {
    final own = (start ? widget.brush.startStep : widget.brush.endStep).index;
    final step = _reach(start, delta);
    if (step == own) return;
    _announce = true;
    widget.onHandleStep?.call(start, step);
  }

  /// Whether the next change of the range came from a key or assistive adjust, which announce
  /// it; a pointer drag or a live run would announce every step they cross.
  bool _announce = false;

  /// What a handle says after `delta` records, which assistive technology previews; what it says
  /// now where that record is not known, as at a limit.
  String _spoken(bool start, int delta, HandleStep now) =>
      widget.handleStep?.call(_reach(start, delta))?.spoken ?? now.spoken;

  /// A handle moved by a key or assistive adjust announces the range, as "Steps 12–31 of 48".
  @override
  void didUpdateWidget(_BrushView old) {
    super.didUpdateWidget(old);
    final brush = widget.brush;
    final (first, last) = (brush.startStep, brush.endStep);
    final moved =
        first.index != old.brush.startStep.index ||
        last.index != old.brush.endStep.index;
    if (!moved) return;
    final announce = _announce;
    _announce = false;
    if (!brush.range || !announce) return;
    SemanticsService.sendAnnouncement(
      View.of(context),
      'Steps ${first.index + 1}–${last.index + 1} of ${first.count}',
      Directionality.of(context),
    );
  }

  /// The keys of one handle: those of [Fixed.rangeHandle], and esc, which leaves the handle.
  Map<ShortcutActivator, VoidCallback> _keys(bool start) {
    final count = widget.brush.startStep.count;
    return {
      ...Fixed.rangeHandle.bind([
        () => _shift(start, -1),
        () => _shift(start, 1),
        () => _shift(start, -10),
        () => _shift(start, 10),
        () => _shift(start, -count),
        () => _shift(start, count),
        () => widget.onRange?.call(null),
      ]),
      escapeKey: () => widget.onLeave?.call(),
    };
  }

  /// The tip above a focused handle, centered on it.
  Widget _tip(double x, HandleStep step) {
    final ws = context.ws;
    return Positioned(
      top: 0,
      left: x,
      child: FractionalTranslation(
        translation: const Offset(-0.5, 0),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: ws.text,
            borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
            child: Text(
              step.tip,
              maxLines: 1,
              style: TextStyle(color: ws.canvas, fontSize: WsMetrics.fsSmall),
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final _BrushView(:brush, :width, :visible, :at, :onRange, :onPan, :onJump) =
        widget;
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
    Widget handle(
      bool start,
      double center,
      FocusNode focus,
      HandleStep step,
    ) => Positioned(
      left: (center - _handleWidth / 2).clamp(
        0.0,
        math.max(0.0, width - _handleWidth),
      ),
      top: _axisHeight + WsMetrics.space2,
      bottom: 0,
      width: _handleWidth,
      child: ViewShortcuts(
        bindings: _keys(start),
        child: Focus(
          focusNode: focus,
          child: Semantics(
            key: Key(start ? 'range-start' : 'range-end'),
            slider: true,
            label: start ? 'Range start' : 'Range end',
            value: step.spoken,
            onIncrease: () => _shift(start, 1),
            increasedValue: _spoken(start, 1, step),
            onDecrease: () => _shift(start, -1),
            decreasedValue: _spoken(start, -1, step),
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              dragStartBehavior: DragStartBehavior.down,
              onDoubleTap: onRange == null ? null : () => onRange(null),
              onHorizontalDragUpdate: onRange == null
                  ? null
                  : (details) {
                      // The brush's own box, not the moving handle's, places the pointer.
                      final value = at(_pointer(details.globalPosition));
                      onRange(
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
                    // The focus ring, as the design tokens draw it.
                    boxShadow: focus.hasFocus
                        ? [BoxShadow(color: ws.accent, spreadRadius: 2)]
                        : null,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    return Stack(
      clipBehavior: Clip.none,
      children: [
        Positioned(
          left: left,
          width: right - left,
          top: _axisHeight + WsMetrics.space2 - 3,
          bottom: 0,
          child: Semantics(
            key: const Key('range-brush'),
            container: true,
            label: brush.label,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTapUp: onJump == null
                  ? null
                  : (details) => onJump(at(_pointer(details.globalPosition))),
              onHorizontalDragStart: (details) =>
                  _drag = (brush.start, _pointer(details.globalPosition)),
              onHorizontalDragEnd: (_) => _drag = null,
              onHorizontalDragUpdate: onPan == null
                  ? null
                  : (details) {
                      final (start, from) = _drag ?? (brush.start, 0.0);
                      onPan(
                        start +
                            (_pointer(details.globalPosition) - from) /
                                width *
                                (visible.$2 - visible.$1),
                      );
                    },
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: ws.accent.withValues(alpha: 0.08),
                  border: Border.all(color: ws.accent.withValues(alpha: 0.55)),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                ),
              ),
            ),
          ),
        ),
        handle(true, left, _startFocus, brush.startStep),
        handle(false, right, _endFocus, brush.endStep),
        if (_startFocus.hasFocus) _tip(left, brush.startStep),
        if (_endFocus.hasFocus) _tip(right, brush.endStep),
      ],
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
          final color = kindColor(palette, BridgeRecordKind.message);
          final firstToken = segment.firstToken;
          final split = firstToken == null
              ? left
              : x(firstToken).clamp(left, right);
          _bar(
            canvas,
            Rect.fromLTRB(left, rect.top, split, rect.bottom),
            color.withValues(alpha: 0.35),
          );
          _bar(
            canvas,
            Rect.fromLTRB(split, rect.top, right, rect.bottom),
            color,
          );
        case OverviewLane.tools:
          _bar(canvas, rect, kindColor(palette, BridgeRecordKind.tool));
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
