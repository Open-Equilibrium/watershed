import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'table_cells.dart';

/// A Table's selected cell and its edit, as in a spreadsheet and in Notion (approved Table
/// View mockups, sections 4 and 9): a click selects a cell, a second click or ⏎ edits a
/// text or number cell, typing replaces it, and a checkbox ticks on a click or Space.
class CellSelection extends ChangeNotifier {
  CellSelection({
    required this.rowAt,
    required this.rowCount,
    required this.fields,
    required this.write,
    required this.reveal,
    required this.leave,
  }) {
    // Leaving the cell editor keeps what was typed, as a spreadsheet does.
    editFocus.addListener(() {
      if (!editFocus.hasFocus && editing) commit(stay: false);
    });
  }

  /// The row at a position, or none while its window is read.
  final BridgeRow? Function(int index) rowAt;
  final int Function() rowCount;
  final List<BridgeField> Function() fields;

  /// Sets a row's cell of a field.
  final void Function(BridgeRow row, BridgeField field, BridgeCell cell) write;

  /// Scrolls a cell, as row and field positions, into view.
  final void Function(int row, int field) reveal;

  /// esc on a selected cell leaves the Table.
  final VoidCallback leave;

  /// The selected cell, as row and field positions, and whether it is edited.
  (int, int)? selected;
  var editing = false;

  /// The selected row's and field's identities, which the selection follows when rows or
  /// fields change elsewhere.
  String? _rowId;
  String? _fieldId;

  /// The cell an edit started on, which it is written to, and its value then.
  (BridgeRow, BridgeField, BridgeCell?)? _edited;
  final edit = TextEditingController();
  late final editFocus = FocusNode(
    debugLabel: 'Cell editor',
    onKeyEvent: _editKey,
  );
  late final cellsFocus = FocusNode(
    debugLabel: 'Table cells',
    onKeyEvent: _cellKey,
  );

  /// Why the edited value was refused, shown under the Table until it is fixed.
  String? error;

  /// The person's number format, as the Table last built it.
  NumberFormat numbers = NumberFormat.decimalPattern('en_US');

  var _disposed = false;

  /// The selected cell's row, field and value, or none while its row is read.
  (BridgeRow, BridgeField, BridgeCell?)? get _target {
    final (row, field) = selected ?? (-1, -1);
    final all = fields();
    final read = rowAt(row);
    if (read == null || field < 0 || field >= all.length) return null;
    final value = read.cells
        .where((cell) => cell.field == all[field].id)
        .firstOrNull
        ?.value;
    return (read, all[field], value);
  }

  /// How a cell shows that it is selected or edited.
  Decoration? mark(BuildContext context, int row, int field) {
    if (selected != (row, field)) return null;
    final accent = context.ws.accent;
    return editing
        ? BoxDecoration(border: Border.all(color: accent, width: 2))
        : BoxDecoration(
            color: accent.withValues(alpha: 0.06),
            border: Border.all(color: accent.withValues(alpha: 0.45), width: 2),
          );
  }

  /// A click selects a cell, a second one edits text and numbers, and a checkbox ticks.
  void tap(int row, int field) {
    // A click in the edited cell, beside its text, leaves the edit as it is.
    if (editing && selected == (row, field)) return;
    if (editing && !commit(stay: false)) return;
    final again = selected == (row, field);
    _place(row, field);
    cellsFocus.requestFocus();
    _changed();
    final target = _target;
    if (target == null) return;
    final (read, column, cell) = target;
    if (column.kind == BridgeFieldKind.checkbox) {
      _toggle(read, column, cell);
    } else if (again && _edits(column.kind)) {
      _startEdit(_shown(column.kind, cell));
    }
  }

  void _place(int row, int field) {
    selected = (row, field);
    _rowId = rowAt(row)?.id;
    final all = fields();
    _fieldId = field < all.length ? all[field].id : null;
  }

  /// Moves the selection to where its row and field are now, after rows or fields changed
  /// elsewhere. A row whose place is not read keeps its place. A row or field no longer
  /// where the reads have it ends the selection; an open edit is still written to its
  /// cell, which takes it if the row only moved and refuses it visibly if it went.
  void relocate(int? Function(String row) rowIndex) {
    final (row, _) = selected ?? (-1, -1);
    if (row < 0) return;
    // A selection made before its row was read takes the row once it is.
    _rowId ??= rowAt(row)?.id;
    final (rowId, fieldId) = (_rowId, _fieldId);
    if (rowId == null || fieldId == null) return;
    final at = fields().indexWhere((shown) => shown.id == fieldId);
    final found = rowIndex(rowId);
    if (at >= 0 && found != null) {
      if ((found, at) == selected) return;
      selected = (found, at);
      _changed();
      return;
    }
    final unread = found == null && rowAt(row) == null && row < rowCount();
    if (at >= 0 && unread) return;
    if (editing) commit(stay: false);
    clear();
  }

  /// Forgets the selection, as when the Table is left.
  void clear() {
    selected = null;
    editing = false;
    _edited = null;
    error = null;
    _changed();
  }

  /// Writes an edit still open as it stands, as when the Table goes or Liquid quits.
  void finish() {
    if (editing) commit(stay: false);
  }

  static bool _edits(BridgeFieldKind kind) =>
      kind == BridgeFieldKind.text || kind == BridgeFieldKind.number;

  /// A cell's value as its editor starts with it.
  String _shown(BridgeFieldKind kind, BridgeCell? cell) => switch (cell?.kind) {
    BridgeCellKind.text => cell!.text ?? '',
    BridgeCellKind.number => numberText(numbers, cell!.number),
    _ => '',
  };

  void _startEdit(String text) {
    _edited = _target;
    edit.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
    editing = true;
    error = null;
    _changed();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_disposed && editing) editFocus.requestFocus();
    });
  }

  /// Writes what was typed and ends the edit; a number that is none stays to be fixed.
  /// With `stay`, the selection takes the keys again.
  bool commit({required bool stay}) {
    final target = _edited;
    if (target == null) {
      _endEdit(stay: stay);
      return true;
    }
    final (row, field, before) = target;
    final typed = edit.text;
    final BridgeCell cell;
    if (typed.trim().isEmpty) {
      cell = _cell(BridgeCellKind.empty);
    } else if (field.kind == BridgeFieldKind.number) {
      final number = _parse(typed.trim());
      if (number == null) {
        error = '“$typed” is no number.';
        _changed();
        if (stay) editFocus.requestFocus();
        return false;
      }
      cell = _cell(BridgeCellKind.number, number: number);
    } else {
      cell = _cell(BridgeCellKind.text, text: typed);
    }
    if (typed != _shown(field.kind, before)) write(row, field, cell);
    _endEdit(stay: stay);
    return true;
  }

  double? _parse(String typed) {
    try {
      final number = numbers.parse(typed).toDouble();
      return number.isFinite ? number : null;
    } on FormatException {
      return null;
    }
  }

  void _endEdit({required bool stay}) {
    editing = false;
    _edited = null;
    error = null;
    _changed();
    if (stay) cellsFocus.requestFocus();
  }

  static BridgeCell _cell(
    BridgeCellKind kind, {
    String? text,
    double number = 0,
    bool checked = false,
  }) => BridgeCell(kind: kind, text: text, number: number, checked: checked);

  void _toggle(BridgeRow row, BridgeField field, BridgeCell? cell) => write(
    row,
    field,
    _cell(BridgeCellKind.checkbox, checked: !(cell?.checked ?? false)),
  );

  /// ⏎ keeps the edit and selects the cell, ⇥ keeps it and selects the next one, esc
  /// undoes it; the rest is typing.
  KeyEventResult _editKey(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !edit.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        commit(stay: true);
      case LogicalKeyboardKey.tab:
        if (commit(stay: true)) {
          _tab(back: HardwareKeyboard.instance.isShiftPressed);
        }
      case LogicalKeyboardKey.escape:
        _endEdit(stay: true);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  /// The keys on a selected cell (approved Table View mockups, section 9).
  KeyEventResult _cellKey(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    if (editing) {
      // Typing that comes before the editor takes the keys goes into it.
      final typed = event.character;
      if (editFocus.hasFocus || typed == null || !_printable(typed)) {
        return KeyEventResult.ignored;
      }
      edit.value = TextEditingValue(
        text: edit.text + typed,
        selection: TextSelection.collapsed(offset: edit.text.length + 1),
      );
      return KeyEventResult.handled;
    }
    final target = _target;
    if (target == null) return KeyEventResult.ignored;
    final (row, field, cell) = target;
    final keys = HardwareKeyboard.instance;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowUp:
        _move(-1, 0);
      case LogicalKeyboardKey.arrowDown:
        _move(1, 0);
      case LogicalKeyboardKey.arrowLeft:
        _move(0, -1);
      case LogicalKeyboardKey.arrowRight:
        _move(0, 1);
      case LogicalKeyboardKey.tab:
        _tab(back: keys.isShiftPressed);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter
          when _edits(field.kind):
        _startEdit(_shown(field.kind, cell));
      case LogicalKeyboardKey.escape:
        clear();
        leave();
      case LogicalKeyboardKey.space when field.kind == BridgeFieldKind.checkbox:
        _toggle(row, field, cell);
      case LogicalKeyboardKey.backspace || LogicalKeyboardKey.delete:
        if (cell != null && cell.kind != BridgeCellKind.empty) {
          write(row, field, _cell(BridgeCellKind.empty));
        }
      default:
        // Typing replaces a text or number cell.
        final typed = event.character;
        if (typed == null ||
            !_printable(typed) ||
            keys.isControlPressed ||
            keys.isMetaPressed ||
            !_edits(field.kind)) {
          return KeyEventResult.ignored;
        }
        _startEdit(typed);
    }
    return KeyEventResult.handled;
  }

  static bool _printable(String typed) =>
      typed.isNotEmpty &&
      typed.codeUnitAt(0) >= 0x20 &&
      typed.codeUnitAt(0) != 0x7f;

  void _move(int rows, int columns) {
    final (row, field) = selected!;
    _select(
      (row + rows).clamp(0, math.max(0, rowCount() - 1)),
      (field + columns).clamp(0, math.max(0, fields().length - 1)),
    );
  }

  /// ⇥ selects the next cell, after a row's last field the next row's first; ⇧⇥ goes back.
  void _tab({required bool back}) {
    final (row, field) = selected!;
    final width = fields().length;
    var (next, at) = (row, field + (back ? -1 : 1));
    if (at >= width) (next, at) = (row + 1, 0);
    if (at < 0) (next, at) = (row - 1, width - 1);
    if (next < 0 || next >= rowCount()) return;
    _select(next, at);
  }

  void _select(int row, int field) {
    _place(row, field);
    _changed();
    reveal(row, field);
  }

  /// The edited cell's text field.
  Widget editor(BuildContext context, BridgeField field) => Align(
    alignment: Alignment.centerLeft,
    child: TextField(
      key: const Key('cell editor'),
      controller: edit,
      focusNode: editFocus,
      textAlign: field.kind == BridgeFieldKind.number
          ? TextAlign.right
          : TextAlign.start,
      style: TextStyle(fontSize: WsMetrics.fsUi, color: context.ws.text),
      decoration: const InputDecoration.collapsed(hintText: null),
    ),
  );

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    edit.dispose();
    editFocus.dispose();
    cellsFocus.dispose();
    super.dispose();
  }
}
