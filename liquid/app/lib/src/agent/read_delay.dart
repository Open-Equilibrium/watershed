import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// A card that decides something for the agent takes keys and clicks only once it could be read:
/// 0.5 s after it appears or Liquid comes back in front (ADR-0207, ADR-0209).
mixin ReadDelay<T extends StatefulWidget> on State<T> {
  bool _armed = false;
  Timer? _arming;

  /// Starts the wait again whenever Liquid comes back in front, since the card may have
  /// appeared while the user worked elsewhere.
  late final AppLifecycleListener _front;

  /// Keys held when the card appeared; until released, their repeats reach nothing in it.
  final Set<PhysicalKeyboardKey> _held = {
    ...HardwareKeyboard.instance.physicalKeysPressed,
  };

  /// Whether the card was shown long enough to be read.
  bool get armed => _armed;

  /// Runs `decision` only once the card could be read.
  void decided(VoidCallback decision) {
    if (_armed) decision();
  }

  /// Keeps the repeats and the release of a key held from before the card appeared from it.
  KeyEventResult skipHeld(FocusNode node, KeyEvent event) {
    if (!_held.contains(event.physicalKey)) return KeyEventResult.ignored;
    if (event is KeyUpEvent) _held.remove(event.physicalKey);
    return KeyEventResult.handled;
  }

  @override
  void initState() {
    super.initState();
    _front = AppLifecycleListener(onStateChange: (_) => _arm());
    _arm();
  }

  /// Takes keys and clicks 0.5 s after Liquid is in front with the card.
  void _arm() {
    _armed = false;
    _arming?.cancel();
    if (WidgetsBinding.instance.lifecycleState
        case null || AppLifecycleState.resumed) {
      _arming = Timer(const Duration(milliseconds: 500), () => _armed = true);
    }
  }

  @override
  void dispose() {
    _arming?.cancel();
    _front.dispose();
    super.dispose();
  }
}
