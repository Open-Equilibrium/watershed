import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'action_area.dart';

/// Height of the extension widgets' strip above the input area.
const widgetStripHeight = 36.0;

/// The room the floating input area takes at the bottom of its column; content keeps it free
/// below its last row. Room the input area takes beyond it, such as for the post-push offer,
/// [InputLane] keeps free by ending its content higher.
const inputClearance =
    widgetStripHeight + actionAreaHeight + 2 * WsMetrics.space4;

/// A tab's main column with the input area floating at its bottom, as T3 Code's composer does:
/// centered, at most the reading width, over a fade into the canvas. Without
/// an `input` it shows only `child`.
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
        LayoutId(
          id: _Part.input,
          child: InputScope(child: input),
        ),
      ],
    );
  }
}

enum _Part { content, fade, input }

/// Lays out the input area first, so the content ends above whatever it takes beyond
/// [inputClearance] in the same frame.
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

/// Marks the input area, so a tab's own keys can leave typing in it alone.
class InputScope extends StatelessWidget {
  const InputScope({super.key, required this.child});

  final Widget child;

  /// Whether the keyboard is in the input area.
  static bool hasFocus() =>
      FocusManager.instance.primaryFocus?.context
          ?.findAncestorWidgetOfExactType<InputScope>() !=
      null;

  @override
  Widget build(BuildContext context) => child;
}

/// A tab's own keys, as [CallbackShortcuts] binds them, except while the keyboard is in the
/// floating input area: there its keys type, and app-wide keys above still work.
class ViewShortcuts extends StatelessWidget {
  const ViewShortcuts({super.key, required this.bindings, required this.child});

  final Map<ShortcutActivator, VoidCallback> bindings;
  final Widget child;

  @override
  Widget build(BuildContext context) => Focus(
    canRequestFocus: false,
    skipTraversal: true,
    onKeyEvent: (node, event) {
      if (InputScope.hasFocus()) return KeyEventResult.ignored;
      for (final MapEntry(key: activator, value: callback)
          in bindings.entries) {
        if (activator.accepts(event, HardwareKeyboard.instance)) {
          callback();
          return KeyEventResult.handled;
        }
      }
      return KeyEventResult.ignored;
    },
    child: child,
  );
}
