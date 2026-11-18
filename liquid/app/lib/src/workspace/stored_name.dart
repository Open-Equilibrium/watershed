import 'dart:async';

import 'package:flutter/widgets.dart';

import '../controls.dart';

class StoredName {
  StoredName(
    String stored, {
    required this.rename,
    required this.restore,
    required this.quitting,
    required this.replaced,
  }) : _named = stored {
    _controller = TextEditingController(text: stored)..addListener(typed);
  }

  final Future<void> Function(String name) rename;

  final Future<String?> Function() restore;

  final bool Function() quitting;

  final VoidCallback replaced;

  TextEditingController get controller => _controller;
  late TextEditingController _controller;

  String _named;
  int _renames = 0;
  bool _replaced = false;

  void typed() {
    if (!_controller.value.composing.isCollapsed && !quitting()) return;
    write();
  }

  void write() {
    final name = _controller.text;
    if (name == _named) return;
    _named = name;
    final asked = ++_renames;
    unawaited(
      rename(name).then<void>(
        (_) {},
        onError: (Object _) async {
          if (asked != _renames) return;
          final stored = await restore();
          if (stored != null && asked == _renames) show(stored);
        },
      ),
    );
  }

  void show(String name) {
    _named = name;
    if (!writeText(_controller, TextEditingValue(text: name))) return;
    _replaced = true;
    replaced();
  }

  void handOver(String text, {required bool written}) {
    if (written) _named = text;
    _controller.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }

  TextEditingController take() {
    if (_replaced) {
      _replaced = false;
      _controller = adoptController(_controller, listener: typed);
    }
    return _controller;
  }

  void dispose() {
    write();
    _controller.dispose();
  }
}
