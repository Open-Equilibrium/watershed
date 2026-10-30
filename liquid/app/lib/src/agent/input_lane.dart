import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'action_area.dart';

/// Height of the extension widgets' strip above the input area.
const widgetStripHeight = 36.0;

/// The room the floating input area takes at the bottom of its column; content keeps it free
/// below its last row.
const inputClearance =
    widgetStripHeight + actionAreaHeight + 2 * WsMetrics.space4;

/// A tab's main column with the input area floating at its bottom, as T3 Code's composer does
/// (approved mockup): centered, at most the reading width, over a fade into the canvas. Without
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
    return Stack(
      children: [
        Positioned.fill(child: child),
        Positioned(
          left: 0,
          right: 0,
          bottom: 0,
          height: inputClearance,
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
        Positioned(
          left: WsMetrics.space4,
          right: WsMetrics.space4,
          bottom: WsMetrics.space4,
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: readingWidth),
              child: SizedBox(
                width: double.infinity,
                child: InputScope(child: input),
              ),
            ),
          ),
        ),
      ],
    );
  }
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
