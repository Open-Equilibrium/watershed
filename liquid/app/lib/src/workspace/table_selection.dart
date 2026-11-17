import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../rust/api/media.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'cell_pickers.dart';
import 'file_cells.dart';
import 'media_block.dart';
import 'paste.dart';
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
    required this.report,
    required this.addOption,
    required this.attach,
    required this.save,
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

  /// Sets a row's cell of a field; a refused write completes with its error, which the
  /// Page shows.
  final Future<void> Function(BridgeRow row, BridgeField field, BridgeCell cell)
  write;

  /// Scrolls a cell, as row and field positions, into view.
  final void Function(int row, int field) reveal;

  /// esc on a selected cell leaves the Table.
  final VoidCallback leave;

  /// Shows a problem on the Page.
  final void Function(String problem) report;

  /// Adds an option named so, of that color, to a select field, and answers its identity.
  final Future<String> Function(BridgeField field, String name, int color)
  addOption;

  /// Adds files as attachments and answers those that went in.
  final Future<List<BridgeMedia>> Function(List<MediaSource> sources) attach;

  /// Saves a copy of a file where the person chooses.
  final Future<void> Function(BridgeMedia file) save;

  /// The selected cell, as row and field positions, and whether it is edited.
  (int, int)? selected;
  var editing = false;

  /// Whether a select or date cell's picker is open, and what is typed in it.
  var picking = false;
  final pickText = TextEditingController();

  /// The open picker's keys: ↑↓, ⏎, esc and ⇥.
  KeyEventResult Function(KeyEvent event)? pickKeys;

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

  /// Keeps the editor, its text input included, when its cell is built elsewhere, as when
  /// its row moves or the Table scrolls sideways.
  final _editorKey = GlobalKey(debugLabel: 'Cell editor');

  /// Ticks written and not yet read back, by row and field, so quick toggles each flip.
  final _ticks = <(String, String), bool>{};

  /// Why the edited value was refused, shown under the Table until it is fixed.
  String? error;

  /// The person's number format and region, as the Table last built them.
  NumberFormat numbers = NumberFormat.decimalPattern('en_US');
  String days = 'en_US';

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
    // A click on the picked cell leaves its picker open.
    if (picking && selected == (row, field)) return;
    closePick(stay: false);
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
    } else if (again && _picks(column.kind)) {
      _startPick('');
    }
  }

  static bool _picks(BridgeFieldKind kind) =>
      kind == BridgeFieldKind.select ||
      kind == BridgeFieldKind.date ||
      kind == BridgeFieldKind.file;

  /// The selected file cell's files.
  List<BridgeMedia> get pickedFiles => switch (_target) {
    (_, _, final cell?) when cell.kind == BridgeCellKind.files => cell.files,
    _ => const [],
  };

  /// Adds `sources` to the file cell at `row` and `field`, after its files, once they are
  /// in; files another write added meanwhile stay.
  Future<void> addFiles(int row, int field, List<MediaSource> sources) {
    // The row and field as they are now, by identity, as the add may wait its turn.
    final all = fields();
    final rowId = rowAt(row)?.id;
    if (rowId == null || field >= all.length) return Future.value();
    return _queue(rowId, all[field].id, sources);
  }

  Future<void> _queue(String row, String field, List<MediaSource> sources) {
    // One add at a time, so each starts from the files the one before wrote.
    final done = _adding.then((_) => _addFiles(row, field, sources));
    _adding = done.then<void>((_) {}, onError: (Object _) {});
    return done;
  }

  Future<void> _adding = Future.value();

  /// The files last written to each cell, by row and field, until a read shows them.
  final _written = <(String, String), List<BridgeMedia>>{};

  /// The row `id` as read now, wherever it is; none once it went.
  BridgeRow? _rowById(String id) {
    for (var at = 0; at < rowCount(); at++) {
      if (rowAt(at) case final found? when found.id == id) return found;
    }
    return null;
  }

  Future<void> _addFiles(
    String rowId,
    String fieldId,
    List<MediaSource> sources,
  ) async {
    final column = fields().where((field) => field.id == fieldId).firstOrNull;
    final read = _rowById(rowId);
    if (read == null ||
        column == null ||
        column.kind != BridgeFieldKind.file ||
        sources.isEmpty) {
      return;
    }
    final key = (read.id, column.id);
    List<BridgeMedia> filesOf(BridgeRow row) {
      final shown = [
        ...?row.cells
            .where((cell) => cell.field == column.id)
            .firstOrNull
            ?.value
            .files,
      ];
      // A write not yet read back is what the cell holds.
      final written = _written[key];
      if (written == null) return shown;
      if (_sameFiles(shown, written)) _written.remove(key);
      return written;
    }

    String full() => 'A cell holds at most $maxCellFiles files.';
    if (filesOf(read).length + sources.length > maxCellFiles) {
      report(full());
      return;
    }
    final added = await attach(sources);
    if (added.isEmpty || _disposed) return;
    // The row as read now, wherever it went meanwhile; gone, it takes nothing.
    final now = _rowById(read.id);
    if (now == null) {
      report(
        'Liquid did not add the files, as their row went; add them again.',
      );
      return;
    }
    final files = [...filesOf(now), ...added];
    if (files.length > maxCellFiles) {
      report(full());
      return;
    }
    _written[key] = files;
    try {
      await write(
        read,
        column,
        BridgeCell(
          kind: BridgeCellKind.files,
          number: 0,
          checked: false,
          files: files,
        ),
      );
    } on Object {
      // Refused: the Page shows why, and the cell holds what is stored.
      _written.remove(key);
    }
  }

  static bool _sameFiles(List<BridgeMedia> a, List<BridgeMedia> b) =>
      a.length == b.length &&
      [
        for (var at = 0; at < a.length; at++)
          a[at].attachment == b[at].attachment,
      ].every((same) => same);

  /// Opens the file dialog and adds the chosen files to the selected file cell.
  Future<void> addPickedFiles() async {
    // The cell as it is now, by identity, as the dialog may stay open a while.
    final target = _target;
    closePick();
    if (target == null) return;
    final (row, field, _) = target;
    final sources = await MediaPlatform.pickFiles();
    await _queue(row.id, field.id, sources);
  }

  /// Removes the selected file cell's file at `index`; the last one empties it.
  void removeFile(int index) {
    if (_target case (final row, final field, final cell?)
        when cell.kind == BridgeCellKind.files && index < cell.files.length) {
      // From the files an add wrote, if a read does not show them yet.
      final held = _written[(row.id, field.id)] ?? cell.files;
      if (index >= held.length) return;
      final files = [...held]..removeAt(index);
      _write(
        row,
        field,
        files.isEmpty
            ? _cell(BridgeCellKind.empty)
            : BridgeCell(
                kind: BridgeCellKind.files,
                number: 0,
                checked: false,
                files: files,
              ),
      );
    }
  }

  /// ⌘V on a selected file cell adds copied files, or a copied image.
  Future<void> _pasteFiles(String row, String field) async {
    final Pasted pasted;
    try {
      pasted = await readClipboard();
    } on Object catch (error) {
      report('Liquid could not read the clipboard: $error');
      return;
    }
    final sources = [
      ...await sourcesOfPaths(pasted.files, report),
      if (pasted.files.isEmpty && pasted.image != null)
        MediaSource.bytes('Pasted image.png', pasted.image!),
    ];
    await _queue(row, field, sources);
  }

  /// The selected select, date or file cell's field while its picker is open.
  BridgeField? get picked => picking ? _target?.$2 : null;

  /// Opens the selected cell's picker with what is typed; a date cell's starts with its
  /// day when nothing is.
  void _startPick(String typed) {
    final (row, field) = selected!;
    reveal(row, field);
    if (_target case (_, final column, final cell)
        when typed.isEmpty && column.kind == BridgeFieldKind.date) {
      typed = _shownDay(cell);
    }
    pickText.value = TextEditingValue(
      text: typed,
      selection: TextSelection.collapsed(offset: typed.length),
    );
    picking = true;
    _changed();
  }

  /// Picks `option` for the selected cell, unless it holds it already, and closes the picker.
  void pick(BridgeOption option) {
    if (_target case (final row, final field, final cell)
        when cell?.kind != BridgeCellKind.select || cell?.text != option.id) {
      _write(row, field, _cell(BridgeCellKind.select, text: option.id));
    }
    closePick();
  }

  /// Creates an option named `name` in the next color and picks it for the selected cell.
  void create(String name) {
    final target = _target;
    closePick();
    if (target == null) return;
    final (row, field, _) = target;
    addOption(field, name, nextColor(field))
        .then(
          (option) =>
              write(row, field, _cell(BridgeCellKind.select, text: option)),
        )
        .then<void>((_) {}, onError: (Object _) {});
  }

  /// Writes `day` into the selected date cell, or empties it, and closes the picker;
  /// with `stay`, the selection takes the keys again.
  void pickDay(DateTime? day, {bool stay = true}) {
    if (_target case (final row, final field, final cell)) {
      final stored = day == null ? null : storedDays.format(day);
      final kept = stored == null
          ? cell == null || cell.kind == BridgeCellKind.empty
          : cell?.kind == BridgeCellKind.date && cell?.text == stored;
      if (!kept) {
        _write(
          row,
          field,
          stored == null
              ? _cell(BridgeCellKind.empty)
              : _cell(BridgeCellKind.date, text: stored),
        );
      }
    }
    closePick(stay: stay);
  }

  /// A date cell's day as cells show it.
  String _shownDay(BridgeCell? cell) =>
      switch (DateTime.tryParse(cell?.text ?? '')) {
        final day? => shownDays(days).format(day),
        null => '',
      };

  /// ⏎ in a date picker writes the day typed, or empties the cell when nothing is; the
  /// cell's own day is kept as it is, and a text that is no day stays, saying so, and
  /// answers false.
  bool enterDay({bool stay = true}) {
    final typed = pickText.text;
    if (_target case (_, _, final cell)
        when typed.isNotEmpty && typed == _shownDay(cell)) {
      closePick(stay: stay);
      return true;
    }
    final day = parseDay(typed, days);
    if (typed.trim().isNotEmpty && day == null) {
      error = '“$typed” is no date.';
      _changed();
      return false;
    }
    pickDay(day, stay: stay);
    return true;
  }

  /// A click outside a picker closes it; a date picker writes the day typed first, as
  /// leaving an editor keeps the edit, and drops a text that is no day.
  void leavePick() {
    if (picked?.kind == BridgeFieldKind.date && enterDay(stay: false)) return;
    closePick(stay: false);
  }

  /// Closes the picker and keeps the cell as it is; with `stay`, the selection takes the
  /// keys again.
  void closePick({bool stay = true}) {
    if (!picking) return;
    picking = false;
    error = null;
    _changed();
    if (stay) cellsFocus.requestFocus();
  }

  /// ⇥ in the picker keeps the value and selects the next or, going back, previous cell;
  /// a date picker writes the day typed first, and one that is no day stays, saying so.
  void tabFromPick(bool back) {
    if (picked?.kind == BridgeFieldKind.date && !enterDay()) return;
    closePick();
    _tab(back: back);
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
    // Ending an edit notifies, which asks to relocate again.
    if (row < 0 || _relocating) return;
    _relocating = true;
    try {
      _relocate(row, rowIndex);
    } finally {
      _relocating = false;
    }
  }

  var _relocating = false;

  void _relocate(int row, int? Function(String row) rowIndex) {
    // A selection made before its row was read takes the row once it is.
    _rowId ??= rowAt(row)?.id;
    final (rowId, fieldId) = (_rowId, _fieldId);
    if (rowId == null || fieldId == null) return;
    final at = fields().indexWhere((shown) => shown.id == fieldId);
    final found = rowIndex(rowId);
    if (at >= 0 && found != null) {
      if ((found, at) == selected) return;
      selected = (found, at);
      // A cell moved under its picker closes it, as a scroll does.
      if (picking) {
        picking = false;
        error = null;
        cellsFocus.requestFocus();
      }
      _changed();
      return;
    }
    final unread = found == null && rowAt(row) == null && row < rowCount();
    if (at >= 0 && unread) return;
    // A number that is none cannot be written; the Page says it was not kept.
    if (editing && !commit(stay: false)) {
      report(
        '${error ?? 'The edit'} It was not kept, as its row or field is gone.',
      );
    }
    clear();
  }

  /// Forgets the selection, as when the Table is left.
  void clear() {
    selected = null;
    editing = false;
    picking = false;
    _edited = null;
    error = null;
    _changed();
  }

  /// Writes an edit still open as it stands, as when the Table goes or Liquid quits.
  void finish() {
    if (editing && !commit(stay: false)) {
      report('$error It was not kept, as the Table closed.');
    }
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
    // A selected cell scrolled out of view comes back to be edited.
    final (row, field) = selected!;
    reveal(row, field);
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
    if (typed != _shown(field.kind, before)) _write(row, field, cell);
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
  }) => BridgeCell(
    kind: kind,
    text: text,
    number: number,
    checked: checked,
    files: const [],
  );

  void _toggle(BridgeRow row, BridgeField field, BridgeCell? cell) {
    final at = (row.id, field.id);
    final checked = !(_ticks[at] ?? cell?.checked ?? false);
    _ticks[at] = checked;
    write(row, field, _cell(BridgeCellKind.checkbox, checked: checked)).then(
      (_) {},
      // A refused tick is forgotten, so the next click flips what is stored.
      onError: (Object _) {
        if (_ticks[at] == checked) _ticks.remove(at);
      },
    );
  }

  /// Writes a cell; any other write to a file cell supersedes the list an add wrote.
  void _write(BridgeRow row, BridgeField field, BridgeCell cell) {
    _written.remove((row.id, field.id));
    write(row, field, cell).then<void>((_) {}, onError: (Object _) {});
  }

  /// Forgets the ticks and file lists a read now shows, looking only at the rows read.
  void read(int? Function(String row) rowIndex) {
    BridgeCell? shown((String, String) at) {
      final index = rowIndex(at.$1);
      final row = index == null ? null : rowAt(index);
      return row?.cells.where((cell) => cell.field == at.$2).firstOrNull?.value;
    }

    _ticks.removeWhere((at, checked) {
      final index = rowIndex(at.$1);
      if (index == null || rowAt(index) == null) return false;
      return (shown(at)?.checked ?? false) == checked;
    });
    _written.removeWhere(
      (at, files) => _sameFiles(shown(at)?.files ?? const [], files),
    );
  }

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
    if (picking) {
      // Until the picker takes the keys, its own keys act as there, and typing filters it.
      if (pickKeys?.call(event) == KeyEventResult.handled) {
        return KeyEventResult.handled;
      }
      final typed = event.character;
      if (typed == null || !_printable(typed)) return KeyEventResult.ignored;
      pickText.value = TextEditingValue(
        text: pickText.text + typed,
        selection: TextSelection.collapsed(offset: pickText.text.length + 1),
      );
      return KeyEventResult.handled;
    }
    if (editing) {
      if (editFocus.hasFocus) return KeyEventResult.ignored;
      // Until the editor takes the keys, ⏎, ⇥ and esc act on the edit as there, and
      // typing goes into it.
      if (_editKey(editFocus, event) == KeyEventResult.handled) {
        return KeyEventResult.handled;
      }
      final typed = event.character;
      if (typed == null || !_printable(typed)) return KeyEventResult.ignored;
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
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter
          when _picks(field.kind):
        _startPick('');
      case LogicalKeyboardKey.escape:
        clear();
        leave();
      case LogicalKeyboardKey.space when field.kind == BridgeFieldKind.checkbox:
        // A held Space ticks once.
        if (event is! KeyRepeatEvent) _toggle(row, field, cell);
      case LogicalKeyboardKey.keyV
          when field.kind == BridgeFieldKind.file && isPaste(event):
        // A held ⌘V pastes once.
        if (event is! KeyRepeatEvent) unawaited(_pasteFiles(row.id, field.id));
      case LogicalKeyboardKey.backspace || LogicalKeyboardKey.delete:
        if (cell != null && cell.kind != BridgeCellKind.empty) {
          _write(row, field, _cell(BridgeCellKind.empty));
        }
      default:
        // Typing replaces a text or number cell, and goes into a select or date cell's
        // picker.
        final typed = event.character;
        if (typed == null ||
            !_printable(typed) ||
            keys.isControlPressed ||
            keys.isMetaPressed) {
          return KeyEventResult.ignored;
        }
        if (_edits(field.kind)) {
          _startEdit(typed);
        } else if (_picks(field.kind)) {
          _startPick(typed);
        } else {
          return KeyEventResult.ignored;
        }
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
    key: const Key('cell editor'),
    alignment: Alignment.centerLeft,
    child: TextField(
      key: _editorKey,
      controller: edit,
      focusNode: editFocus,
      // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
      selectAllOnFocus: false,
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
    pickText.dispose();
    editFocus.dispose();
    cellsFocus.dispose();
    super.dispose();
  }
}

/// The picked select or date cell, which its picker hangs from (see [AnchoredMenu]); a
/// click outside both leaves the picker. Once first laid out it hands itself to
/// [onPlaced], which brings it into view.
class PickedCell extends StatefulWidget {
  const PickedCell({
    super.key,
    required this.cells,
    required this.onPlaced,
    required this.child,
  });

  final CellSelection cells;
  final void Function(RenderObject cell) onPlaced;
  final Widget child;

  @override
  State<PickedCell> createState() => _PickedCellState();
}

class _PickedCellState extends State<PickedCell> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (context.findRenderObject() case final cell? when mounted) {
        widget.onPlaced(cell);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final cells = widget.cells;
    final field = cells.picked;
    if (field == null) return widget.child;
    final date = field.kind == BridgeFieldKind.date;
    if (field.kind == BridgeFieldKind.file) {
      final files = cells.pickedFiles;
      return AnchoredMenu(
        height: FilesPicker.heightFor(files.length),
        onTapOutside: (_) => cells.leavePick(),
        menu: (context) => FilesPicker(
          files: cells.pickedFiles,
          onSave: (file) => unawaited(cells.save(file)),
          onRemove: cells.removeFile,
          onAdd: () => unawaited(cells.addPickedFiles()),
          onClose: cells.closePick,
          onTab: cells.tabFromPick,
          onKeys: (keys) => cells.pickKeys = keys,
        ),
        child: widget.child,
      );
    }
    return AnchoredMenu(
      height: date
          ? DatePicker.height
          : SelectPicker.heightFor(field.options.length),
      onTapOutside: (_) => cells.leavePick(),
      menu: (context) => date
          ? DatePicker(
              text: cells.pickText,
              locale: cells.days,
              onDay: cells.pickDay,
              onEnter: cells.enterDay,
              onClear: () => cells.pickDay(null),
              onClose: cells.closePick,
              onTab: cells.tabFromPick,
              onKeys: (keys) => cells.pickKeys = keys,
            )
          : SelectPicker(
              field: field,
              text: cells.pickText,
              onPick: cells.pick,
              onCreate: cells.create,
              onClose: cells.closePick,
              onTab: cells.tabFromPick,
              onKeys: (keys) => cells.pickKeys = keys,
            ),
      // Nor does a click in the cell take the keys from the picker's filter.
      child: TextFieldTapRegion(child: widget.child),
    );
  }
}
