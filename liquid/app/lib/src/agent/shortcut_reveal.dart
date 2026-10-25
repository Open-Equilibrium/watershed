import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// Shows contextual shortcut hints while ⌘ alone is held for a moment, so they stay out of the
/// GUI otherwise (ADR-0204). Pressing another key meanwhile, as for a ⌘ shortcut, shows nothing.
class ShortcutReveal extends StatefulWidget {
  const ShortcutReveal({super.key, required this.child});

  final Widget child;

  /// Whether hints below `context` show now; the caller rebuilds when this changes.
  static bool shown(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_Shown>()?.notifier?.value ??
      false;

  @override
  State<ShortcutReveal> createState() => _ShortcutRevealState();
}

class _ShortcutRevealState extends State<ShortcutReveal> {
  static const _delay = Duration(milliseconds: 600);
  final _shown = ValueNotifier(false);
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_key);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_key);
    _timer?.cancel();
    _shown.dispose();
    super.dispose();
  }

  bool _key(KeyEvent event) {
    if (event is KeyRepeatEvent) return false;
    final meta =
        event.logicalKey == LogicalKeyboardKey.metaLeft ||
        event.logicalKey == LogicalKeyboardKey.metaRight;
    _timer?.cancel();
    if (event is KeyDownEvent &&
        meta &&
        HardwareKeyboard.instance.logicalKeysPressed.length == 1) {
      _timer = Timer(_delay, () => _shown.value = true);
    } else {
      // Another key, or letting go of ⌘, ends it.
      _shown.value = false;
    }
    return false;
  }

  @override
  Widget build(BuildContext context) =>
      _Shown(notifier: _shown, child: widget.child);
}

class _Shown extends InheritedNotifier<ValueNotifier<bool>> {
  const _Shown({required super.notifier, required super.child});
}
