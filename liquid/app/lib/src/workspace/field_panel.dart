import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// A field kind's name, as the field panel lists it and a field without a name takes it.
String fieldKindName(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 'Text',
  BridgeFieldKind.number => 'Number',
  BridgeFieldKind.select => 'Select',
  BridgeFieldKind.date => 'Date',
  BridgeFieldKind.checkbox => 'Checkbox',
};

/// A field kind's icon, in the header and the field panel.
IconData fieldKindIcon(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => LucideIcons.text,
  BridgeFieldKind.number => LucideIcons.hash,
  BridgeFieldKind.select => LucideIcons.circleChevronDown,
  BridgeFieldKind.date => LucideIcons.calendar,
  BridgeFieldKind.checkbox => LucideIcons.squareCheck,
};

/// The panel + in a Table's header opens (approved Table View mockups, section 3): the
/// field's name, then its kind. ↑↓ choose a kind, ⏎ or a click on a kind adds the field,
/// esc closes the panel. A field without a name takes its kind's, as in Notion.
class FieldPanel extends StatefulWidget {
  const FieldPanel({super.key, required this.onAdd, required this.onClose});

  /// Adds a field named so, of that kind; a refused one completes with an error.
  final Future<void> Function(String name, BridgeFieldKind kind) onAdd;
  final VoidCallback onClose;

  @override
  State<FieldPanel> createState() => _FieldPanelState();
}

class _FieldPanelState extends State<FieldPanel> {
  final _name = TextEditingController();
  late final _focus = FocusNode(debugLabel: 'Field name', onKeyEvent: _key);
  var _kind = BridgeFieldKind.text;

  /// While a field is added, so a second ⏎ or click adds no second one.
  var _adding = false;

  @override
  void initState() {
    super.initState();
    // The name takes the keys, also while another field had them.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _name.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _add(BridgeFieldKind kind) async {
    if (_adding) return;
    _adding = true;
    final name = _name.text.trim();
    try {
      await widget.onAdd(name.isEmpty ? fieldKindName(kind) : name, kind);
    } on Object {
      // The problem shows on the Page; the panel stays to try again.
    } finally {
      _adding = false;
    }
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !_name.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    // A held ⏎ adds one field.
    if (event is KeyRepeatEvent &&
        (event.logicalKey == LogicalKeyboardKey.enter ||
            event.logicalKey == LogicalKeyboardKey.numpadEnter)) {
      return KeyEventResult.handled;
    }
    const kinds = BridgeFieldKind.values;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(
          () => _kind = kinds[(kinds.indexOf(_kind) + by) % kinds.length],
        );
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        unawaited(_add(_kind));
      case LogicalKeyboardKey.escape:
        widget.onClose();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('field panel'),
      width: 250,
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: DefaultTextStyle.merge(
        style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 30,
              margin: const EdgeInsets.fromLTRB(2, 2, 2, WsMetrics.space1),
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.canvas,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                border: Border.all(color: ws.accent, width: 1.5),
              ),
              child: TextField(
                controller: _name,
                focusNode: _focus,
                // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
                selectAllOnFocus: false,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: InputDecoration.collapsed(
                  hintText: 'Field name',
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space3,
                WsMetrics.space2,
                WsMetrics.space3,
                2,
              ),
              child: Text(
                'Kind',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              ),
            ),
            for (final kind in BridgeFieldKind.values)
              MouseRegion(
                onEnter: (_) => setState(() => _kind = kind),
                child: Semantics(
                  button: true,
                  selected: kind == _kind,
                  child: GestureDetector(
                    onTap: () => unawaited(_add(kind)),
                    child: Container(
                      height: 30,
                      padding: const EdgeInsets.symmetric(
                        horizontal: WsMetrics.space3,
                      ),
                      decoration: BoxDecoration(
                        color: kind == _kind ? ws.hover : null,
                        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      ),
                      child: Row(
                        children: [
                          Icon(
                            fieldKindIcon(kind),
                            size: 14,
                            color: ws.textFaint,
                          ),
                          const SizedBox(width: 10),
                          Text(fieldKindName(kind)),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
