import 'dart:async';

import 'package:flutter/widgets.dart';

import '../controls.dart';

/// A name field over a stored name, as a Page's title and a Table's name are: each change,
/// once any composition ends, is one rename (architecture rule 7), and a refused one puts the
/// stored name back. A name Liquid shows gets a fresh controller as the field builds, so ⌘Z
/// cannot bring back the name it replaced.
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

  /// Writes a rename to `name`.
  final Future<void> Function(String name) rename;

  /// The stored name to show after the latest rename was refused; null keeps the field.
  final Future<String?> Function() restore;

  /// Whether Liquid quits, which writes a composition still open as it stands.
  final bool Function() quitting;

  /// Called once a name Liquid showed waits for the field to build.
  final VoidCallback replaced;

  /// The field's controller for the build now; see [take].
  TextEditingController get controller => _controller;
  late TextEditingController _controller;

  /// The name last sent or stored, which a change of the field is compared with.
  String _named;
  int _renames = 0;
  bool _replaced = false;

  /// Renames once a composition ends.
  void typed() {
    if (!_controller.value.composing.isCollapsed && !quitting()) return;
    write();
  }

  /// Renames to what the field holds, unless that is already sent or stored.
  void write() {
    final name = _controller.text;
    if (name == _named) return;
    _named = name;
    final asked = ++_renames;
    unawaited(
      rename(name).then<void>(
        (_) {},
        onError: (Object _) async {
          // Only the latest rename's refusal puts the stored name back.
          if (asked != _renames) return;
          final stored = await restore();
          if (stored != null && asked == _renames) show(stored);
        },
      ),
    );
  }

  /// Shows `name` as the stored one, which is no rename.
  void show(String name) {
    _named = name;
    if (!writeText(_controller, TextEditingValue(text: name))) return;
    _replaced = true;
    replaced();
  }

  /// Puts `text`, typed elsewhere for this name, into the field with the caret at its end;
  /// `written` if it is stored already.
  void handOver(String text, {required bool written}) {
    if (written) _named = text;
    _controller.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }

  /// The controller for the field as it builds: a fresh one after a name Liquid showed.
  TextEditingController take() {
    if (_replaced) {
      _replaced = false;
      _controller = adoptController(_controller, listener: typed);
    }
    return _controller;
  }

  /// Writes a composition still open as it stands, then releases the controller.
  void dispose() {
    write();
    _controller.dispose();
  }
}
