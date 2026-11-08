import 'dart:async';

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

  /// The name as typed, kept by [FieldMenus] so closing the menu any way keeps it.
  final TextEditingController name;

  /// Closes the menu, keeping the name typed unless `drop`.
  final void Function({bool drop}) onClose;
  final VoidCallback onDelete;

  @override
  State<FieldMenu> createState() => _FieldMenuState();
}

class _FieldMenuState extends State<FieldMenu> {
  late final _focus = FocusNode(debugLabel: 'Field menu', onKeyEvent: _key);

  /// Whether ↓ chose Delete field, which ⏎ then takes, and whether the pointer is on it,
  /// which only shows.
  var _deleting = false;
  var _hovered = false;

  @override
  void initState() {
    super.initState();
    widget.name.addListener(_typed);
    // The name takes the keys, also while a cell had them.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  /// Typing goes back to the name, which ⏎ then keeps.
  void _typed() {
    if (_deleting) setState(() => _deleting = false);
  }

  @override
  void dispose() {
    widget.name.removeListener(_typed);
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
                // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
                selectAllOnFocus: false,
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
              onEnter: (_) => setState(() => _hovered = true),
              onExit: (_) => setState(() => _hovered = false),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: widget.onDelete,
                child: _item(
                  context,
                  icon: LucideIcons.trash,
                  label: 'Delete field',
                  color: ws.danger,
                  on: _deleting || _hovered,
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
/// section 5), counting the `rows` once read; answers whether to delete it.
Future<bool> confirmDeleteField(
  BuildContext context,
  BridgeField field,
  int? rows,
) async =>
    await showDialog<bool>(
      context: context,
      builder: (context) => DangerDialog(
        title: 'Delete the field “${field.name}”?',
        action: 'Delete field',
        children: [
          Text(switch (rows) {
            null => 'Its values in all rows go with it.',
            0 => 'It holds no values yet.',
            1 => 'Its value in 1 row goes with it.',
            _ => 'Its values in all $rows rows go with it.',
          }, style: DangerDialog.text(context)),
        ],
      ),
    ) ??
    false;

/// A Table's field menu and what it writes (approved Table View mockups, section 5): it
/// opens under a field's header, keeps a typed name however it closes but for esc, also
/// when the Table goes or Liquid quits, and asks before Delete field.
class FieldMenus extends ChangeNotifier {
  FieldMenus({
    required this.fieldOf,
    required this.rename,
    required this.remove,
    required this.confirm,
  });

  /// The Table's field of an identity, as last read.
  final BridgeField? Function(String? id) fieldOf;

  /// Renames a field; a refused write shows on the Page.
  final void Function(String field, String name) rename;

  /// Deletes a field; a refused write shows on the Page.
  final Future<void> Function(String field) remove;

  /// Asks before a field and its values go.
  final Future<bool> Function(BridgeField field) confirm;

  final overlay = OverlayPortalController();
  final anchor = LayerLink();

  /// The field whose menu is open.
  String? open;

  /// The name typed in the menu, which closing it keeps unless esc drops it, and the name
  /// it opened with or last wrote.
  final name = TextEditingController();
  String _opened = '';

  /// A field and the name typed in its menu before Delete field, kept while the question
  /// is open, so quitting or the Table going meanwhile still writes it.
  (String, String)? _asking;

  var _disposed = false;

  /// A click on a field's header opens its menu, or closes it when open.
  void toggle(BridgeField field) {
    final wasOpen = open == field.id;
    close();
    if (wasOpen) return;
    name.value = TextEditingValue(
      text: field.name,
      selection: TextSelection.collapsed(offset: field.name.length),
    );
    _opened = field.name;
    open = field.id;
    overlay.show();
    notifyListeners();
  }

  /// Closes the menu and renames the field as typed, unless `drop`.
  void close({bool drop = false}) {
    if (open == null) return;
    if (!drop) renameTyped();
    open = null;
    overlay.hide();
    notifyListeners();
  }

  /// Renames the menu's field as typed, unless the name is empty or as the menu opened
  /// with it, so a rename elsewhere meanwhile stays; also as the Table goes or Liquid
  /// quits.
  void renameTyped() {
    if (_asking case (final field, final typed)) {
      _asking = null;
      rename(field, typed);
    }
    final field = open;
    final typed = name.text.trim();
    if (field == null || typed.isEmpty || typed == _opened) return;
    _opened = typed;
    rename(field, typed);
  }

  /// Delete field asks first, then deletes the field and its values; a name typed is kept
  /// when the question is cancelled.
  Future<void> delete() async {
    final field = fieldOf(open);
    final typed = name.text.trim();
    if (field != null && typed.isNotEmpty && typed != _opened) {
      _asking = (field.id, typed);
    }
    close(drop: true);
    if (field == null) return;
    final confirmed = await confirm(field);
    final asked = _asking;
    _asking = null;
    if (!confirmed) {
      if (asked case (final id, final kept) when !_disposed) rename(id, kept);
      return;
    }
    if (!_disposed) await remove(field.id);
  }

  /// A field gone elsewhere takes its menu along.
  void read() {
    if (open != null && fieldOf(open) == null) close(drop: true);
  }

  /// The open menu, under its field's header.
  Widget view(BuildContext context) {
    final field = fieldOf(open);
    if (field == null) return const SizedBox.shrink();
    return Align(
      alignment: Alignment.topLeft,
      child: CompositedTransformFollower(
        link: anchor,
        showWhenUnlinked: false,
        targetAnchor: Alignment.bottomLeft,
        offset: const Offset(0, WsMetrics.space1),
        child: TapRegion(
          groupId: overlay,
          onTapOutside: (_) => close(),
          // A click in the menu beside its name keeps the name's keys.
          child: TextFieldTapRegion(
            child: FieldMenu(
              field: field,
              name: name,
              onClose: close,
              onDelete: () => unawaited(delete()),
            ),
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _disposed = true;
    name.dispose();
    super.dispose();
  }
}
