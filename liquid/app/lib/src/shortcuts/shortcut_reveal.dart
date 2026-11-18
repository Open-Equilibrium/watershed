import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'keymap.dart';

class ShortcutReveal extends StatefulWidget {
  const ShortcutReveal({super.key, required this.liquid, required this.child});

  final bool liquid;
  final Widget child;

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
  void didUpdateWidget(ShortcutReveal old) {
    super.didUpdateWidget(old);
    if (old.liquid != widget.liquid) {
      _timer?.cancel();
      _shown.value = false;
    }
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_key);
    _timer?.cancel();
    _shown.dispose();
    super.dispose();
  }

  bool _key(KeyEvent event) {
    final key = event.logicalKey;
    final reveals = widget.liquid
        ? key == liquidKey
        : key == LogicalKeyboardKey.metaLeft ||
              key == LogicalKeyboardKey.metaRight;
    final handled = widget.liquid && key == liquidKey;
    if (event is KeyRepeatEvent) return handled;
    _timer?.cancel();
    if (event is KeyDownEvent &&
        reveals &&
        HardwareKeyboard.instance.logicalKeysPressed.length == 1) {
      _timer = Timer(_delay, () => _shown.value = true);
    } else {
      _shown.value = false;
    }
    return handled;
  }

  @override
  Widget build(BuildContext context) =>
      _Shown(notifier: _shown, child: widget.child);
}

class _Shown extends InheritedNotifier<ValueNotifier<bool>> {
  const _Shown({required super.notifier, required super.child});
}
