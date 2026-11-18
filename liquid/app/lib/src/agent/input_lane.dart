import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../controls.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'action_area.dart';

const inputClearance = actionAreaHeight + 2 * WsMetrics.space4;

class InputLane extends StatelessWidget {
  const InputLane({super.key, required this.input, required this.child});

  final Widget? input;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final input = this.input;
    if (input == null) return child;
    final canvas = context.ws.canvas;
    return CustomMultiChildLayout(
      delegate: _LaneLayout(),
      children: [
        LayoutId(id: _Part.content, child: child),
        LayoutId(
          id: _Part.fade,
          child: IgnorePointer(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  colors: [canvas.withValues(alpha: 0), canvas],
                  stops: const [0, 0.25],
                ),
              ),
            ),
          ),
        ),
        LayoutId(id: _Part.input, child: input),
      ],
    );
  }
}

enum _Part { content, fade, input }

class _LaneLayout extends MultiChildLayoutDelegate {
  @override
  void performLayout(Size size) {
    final width = math.max(
      0.0,
      math.min(readingWidth, size.width - 2 * WsMetrics.space4),
    );
    final input = layoutChild(
      _Part.input,
      BoxConstraints(minWidth: width, maxWidth: width, maxHeight: size.height),
    );
    positionChild(
      _Part.input,
      Offset(
        (size.width - width) / 2,
        size.height - WsMetrics.space4 - input.height,
      ),
    );
    final room = math.max(inputClearance, input.height + 2 * WsMetrics.space4);
    final content = math.max(0.0, size.height - room + inputClearance);
    layoutChild(_Part.content, BoxConstraints.tight(Size(size.width, content)));
    positionChild(_Part.content, Offset.zero);
    layoutChild(
      _Part.fade,
      BoxConstraints.tight(Size(size.width, math.min(room, size.height))),
    );
    positionChild(
      _Part.fade,
      Offset(0, size.height - math.min(room, size.height)),
    );
  }

  @override
  bool shouldRelayout(_LaneLayout oldDelegate) => false;
}
