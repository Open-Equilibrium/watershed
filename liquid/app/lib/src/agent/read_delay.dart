import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

mixin ReadDelay<T extends StatefulWidget> on State<T> {
  bool _armed = false;
  Timer? _arming;

  late final AppLifecycleListener _front;

  final Set<PhysicalKeyboardKey> _held = {
    ...HardwareKeyboard.instance.physicalKeysPressed,
  };

  void decided(VoidCallback decision) {
    if (_armed) decision();
  }

  KeyEventResult skipHeld(FocusNode node, KeyEvent event) {
    if (!_held.contains(event.physicalKey)) return KeyEventResult.ignored;
    if (event is KeyUpEvent) _held.remove(event.physicalKey);
    return KeyEventResult.handled;
  }

  @override
  void initState() {
    super.initState();
    _front = AppLifecycleListener(onStateChange: (_) => _arm());
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _arm();
    });
  }

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
