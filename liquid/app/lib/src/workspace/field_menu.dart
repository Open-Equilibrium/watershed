import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'field_panel.dart';

/// The menu a click on a field's header opens (approved Table View mockups, section 5): its
/// name to rename in place, its kind, which cannot change yet, and Delete field. ⏎ keeps
/// the name and closes it, esc drops the name; ↑↓ choose Delete field, which ⏎ then takes.
class FieldMenu extends StatefulWidget {
  const FieldMenu({
    super.key,
    required this.field,
    required this.name,
    required this.onClose,
    required this.onDelete,
  });

  final BridgeField field;

  /// The name as typed, kept by the Table so closing the menu any way keeps it.
  final TextEditingController name;

  /// Closes the menu, keeping the name typed unless `drop`.
  final void Function({bool drop}) onClose;
  final VoidCallback onDelete;

  @override
  State<FieldMenu> createState() => _FieldMenuState();
}

class _FieldMenuState extends State<FieldMenu> {
  late final _focus = FocusNode(debugLabel: 'Field menu', onKeyEvent: _key);

  /// Whether ↓ chose Delete field.
  var _deleting = false;

  @override
  void initState() {
    super.initState();
    // The name takes the keys, also while a cell had them.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !widget.name.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        setState(() => _deleting = !_deleting);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is KeyRepeatEvent) break;
        _deleting ? widget.onDelete() : widget.onClose();
      case LogicalKeyboardKey.escape:
        widget.onClose(drop: true);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final kind = widget.field.kind;
    return Container(
      key: const Key('field menu'),
      width: 230,
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
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.canvas,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                border: Border.all(color: ws.accent, width: 1.5),
              ),
              child: TextField(
                controller: widget.name,
                focusNode: _focus,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: const InputDecoration.collapsed(hintText: null),
              ),
            ),
            // A field's kind cannot change yet (section 3).
            _item(
              context,
              icon: fieldKindIcon(kind),
              label: fieldKindName(kind),
              trailing: 'kind',
            ),
            Divider(height: 9, thickness: 1, color: ws.border),
            MouseRegion(
              onEnter: (_) => setState(() => _deleting = true),
              onExit: (_) => setState(() => _deleting = false),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: widget.onDelete,
                child: _item(
                  context,
                  icon: LucideIcons.trash,
                  label: 'Delete field',
                  color: ws.danger,
                  on: _deleting,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  static Widget _item(
    BuildContext context, {
    required IconData icon,
    required String label,
    String? trailing,
    Color? color,
    bool on = false,
  }) {
    final ws = context.ws;
    return Container(
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        color: on ? ws.hover : null,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        children: [
          Icon(icon, size: 14, color: color ?? ws.textFaint),
          const SizedBox(width: 10),
          Expanded(
            child: Text(label, style: TextStyle(color: color)),
          ),
          if (trailing != null)
            Text(
              trailing,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
        ],
      ),
    );
  }
}

/// Asks before a field and its values in all rows go (approved Table View mockups,
/// section 5); answers whether to delete it.
Future<bool> confirmDeleteField(
  BuildContext context,
  BridgeField field,
  int rows,
) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) => DangerDialog(
        title: 'Delete the field “${field.name}”?',
        action: 'Delete field',
        children: [
          Text(switch (rows) {
            0 => 'It holds no values yet.',
            1 => 'Its value in 1 row goes with it.',
            _ => 'Its values in all $rows rows go with it.',
          }, style: DangerDialog.text(context)),
        ],
      ),
    ) ??
    false;
