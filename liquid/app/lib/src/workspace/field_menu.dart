import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'field_panel.dart';
import 'formula_field.dart';
import 'formula_text.dart';

class FieldMenu extends StatefulWidget {
  const FieldMenu({
    super.key,
    required this.field,
    required this.name,
    required this.onClose,
    required this.onDelete,
    this.formula,
  });

  final BridgeField field;

  final FieldFormula? formula;

  final TextEditingController name;

  final void Function({bool drop}) onClose;
  final VoidCallback onDelete;

  @override
  State<FieldMenu> createState() => _FieldMenuState();
}

class _FieldMenuState extends State<FieldMenu> {
  late final _focus = FocusNode(debugLabel: 'Field menu', onKeyEvent: _key);

  var _deleting = false;
  var _hovered = false;

  @override
  void initState() {
    super.initState();
    widget.name.addListener(_typed);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

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
                selectAllOnFocus: false,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: const InputDecoration.collapsed(hintText: null),
              ),
            ),
            _item(
              context,
              icon: fieldKindIcon(kind),
              label: fieldKindName(kind),
              trailing: 'kind',
            ),
            if (widget.formula case final formula?
                when kind == BridgeFieldKind.formula)
              Padding(
                padding: const EdgeInsets.all(2),
                child: FormulaField(
                  key: ValueKey(widget.field.formula),
                  formula: widget.field.formula,
                  names: formula.names,
                  known: formula.known,
                  onCommit: formula.write,
                  onDone: () {},
                  always: true,
                  autofocus: false,
                ),
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

class FieldFormula {
  const FieldFormula({
    required this.names,
    required this.known,
    required this.write,
  });

  final List<String?> names;
  final List<Suggestion> Function() known;
  final Future<void> Function(String formula) write;
}

class FieldMenus extends ChangeNotifier {
  FieldMenus({
    required this.fieldOf,
    required this.fields,
    required this.table,
    required this.formula,
    required this.rename,
    required this.remove,
    required this.confirm,
  });

  final BridgeField? Function(String? id) fieldOf;

  final List<BridgeField> Function() fields;

  final (String, String) Function() table;

  final Future<void> Function(String field, String formula) formula;

  final void Function(String field, String name) rename;

  final Future<void> Function(String field) remove;

  final Future<bool> Function(BridgeField field) confirm;

  String? open;

  final name = TextEditingController();
  String _opened = '';

  (String, String)? _asking;

  var _disposed = false;

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
    notifyListeners();
  }

  void close({bool drop = false}) {
    if (open == null) return;
    if (!drop) renameTyped();
    open = null;
    notifyListeners();
  }

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

  void read() {
    if (open != null && fieldOf(open) == null) close(drop: true);
  }

  Widget view(BuildContext context) {
    final field = fieldOf(open);
    if (field == null) return const SizedBox.shrink();
    return FieldMenu(
      field: field,
      name: name,
      onClose: close,
      onDelete: () => unawaited(delete()),
      formula: FieldFormula(
        names: tokenNames(field.formula, (ids) {
          final (id, name) = table();
          return switch (ids) {
            [final field] => fieldOf(field)?.name,
            [final other, final field] when other == id => switch (fieldOf(
              field,
            )) {
              final known? => '$name.${known.name}',
              null => null,
            },
            _ => 'Reference',
          };
        }),
        known: () => [
          for (final other in fields())
            if (other.id != field.id)
              Suggestion(
                other.name,
                'Field',
                reference: Reference('{${other.id}}', other.name),
              ),
          for (final name in formulaFunctions) Suggestion(name, 'Function'),
        ],
        write: (formula) => this.formula(field.id, formula),
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
