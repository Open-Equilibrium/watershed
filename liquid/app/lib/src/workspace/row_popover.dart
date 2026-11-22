import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'cell_pickers.dart';
import 'explorer.dart' show untitled;
import 'field_panel.dart' show fieldKindIcon;
import 'table_cells.dart';
import 'table_view_parts.dart';
import 'workspace_state.dart';

class TableData {
  const TableData({
    required this.page,
    required this.block,
    required this.database,
    required this.read,
    required this.write,
    required this.numbers,
    required this.days,
  });

  final String page;
  final String block;
  final BridgeDatabase database;
  final Future<BridgeAnswer> Function(BridgeQuery query) read;
  final Future<BridgeApplied> Function(BridgeMutation change) write;
  final NumberFormat numbers;
  final String days;

  Map<String, BridgeOption> get options => {
    for (final field in database.fields)
      for (final option in field.options) option.id: option,
  };

  BridgeMutation change(
    BridgeMutationKind kind, {
    String? row,
    String? field,
    BridgeCell? cell,
    String? text,
    String? after,
    bool last = false,
    int color = 0,
    BridgeFieldKind? fieldKind,
  }) => mutation(
    kind,
    page: page,
    block: block,
    row: row,
    field: field,
    cell: cell,
    text: text,
    after: after,
    last: last,
    color: color,
    fieldKind: fieldKind,
  );

  Future<void> setCell(String row, String field, BridgeCell cell) => write(
    change(BridgeMutationKind.setCell, row: row, field: field, cell: cell),
  );

  Future<String?> addRow(Map<String, BridgeCell> cells) async {
    final added = await write(change(BridgeMutationKind.addRow, last: true));
    for (final MapEntry(key: field, value: cell) in cells.entries) {
      await setCell(added.row!, field, cell);
    }
    return added.row;
  }
}

BridgeCell? cellOf(BridgeRow row, String field) =>
    row.cells.where((cell) => cell.field == field).firstOrNull?.value;

String rowTitle(BridgeRow row, BridgeDatabase database) {
  final first = database.fields.firstOrNull;
  final text = first == null ? null : cellOf(row, first.id)?.text;
  return text == null || text.isEmpty ? untitled : text;
}

const emptyValue = BridgeCell(
  kind: BridgeCellKind.empty,
  number: 0,
  checked: false,
  files: [],
);

BridgeCell textValue(BridgeCellKind kind, String text) => BridgeCell(
  kind: kind,
  text: text,
  number: 0,
  checked: false,
  files: const [],
);

class RowPopover extends StatefulWidget {
  const RowPopover({
    super.key,
    required this.data,
    required this.row,
    required this.onClose,
    required this.group,
  });

  final TableData data;
  final BridgeRow row;
  final VoidCallback onClose;

  final Object group;

  static const width = 360.0;

  @override
  State<RowPopover> createState() => _RowPopoverState();
}

class _RowPopoverState extends State<RowPopover> {
  final _focus = FocusNode(debugLabel: 'Row popover');
  final _picks = <String, OverlayPortalController>{};
  final _pickText = TextEditingController();
  final _texts = <String, TextEditingController>{};
  final _typedLast = <String, String>{};

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    _pickText.dispose();
    for (final text in _texts.values) {
      text.dispose();
    }
    super.dispose();
  }

  TableData get _data => widget.data;

  void _write(BridgeField field, BridgeCell cell) => unawaited(
    _data
        .setCell(widget.row.id, field.id, cell)
        .then((_) {}, onError: (Object _) {}),
  );

  void _typed(BridgeField field, String text) {
    final trimmed = text.trim();
    if (_typedLast[field.id] == trimmed) return;
    _typedLast[field.id] = trimmed;
    final held = cellOf(widget.row, field.id);
    if (field.kind == BridgeFieldKind.number) {
      final number = trimmed.isEmpty ? null : _data.numbers.tryParse(trimmed);
      if (trimmed.isNotEmpty && number == null) return;
      if (number == null) {
        if (held != null) _write(field, emptyValue);
      } else if (held?.number != number) {
        _write(
          field,
          BridgeCell(
            kind: BridgeCellKind.number,
            number: number.toDouble(),
            checked: false,
            files: const [],
          ),
        );
      }
      return;
    }
    if (trimmed == (held?.text ?? '')) return;
    _write(
      field,
      trimmed.isEmpty ? emptyValue : textValue(BridgeCellKind.text, trimmed),
    );
  }

  Future<void> _create(BridgeField field, String name) async {
    try {
      final added = await _data.write(
        _data.change(
          BridgeMutationKind.addOption,
          field: field.id,
          text: name,
          color: nextColor(field),
        ),
      );
      _write(field, textValue(BridgeCellKind.select, added.option!));
    } on Object {
      return;
    }
  }

  void _closePick() {
    for (final pick in _picks.values) {
      if (pick.isShowing) pick.hide();
    }
  }

  Widget _value(BridgeField field) {
    final ws = context.ws;
    final held = cellOf(widget.row, field.id);
    final days = shownDays(_data.days);
    switch (field.kind) {
      case BridgeFieldKind.text || BridgeFieldKind.number:
        final text = _texts.putIfAbsent(
          field.id,
          () => TextEditingController(
            text: field.kind == BridgeFieldKind.number
                ? (held == null ? '' : numberText(_data.numbers, held.number))
                : held?.text ?? '',
          ),
        );
        return Focus(
          onFocusChange: (focused) {
            if (!focused) _typed(field, text.text);
          },
          child: TextField(
            controller: text,
            onSubmitted: (typed) {
              _typed(field, typed);
              _focus.requestFocus();
            },
            textAlign: TextAlign.left,
            style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
            decoration: InputDecoration.collapsed(
              hintText: 'Empty',
              hintStyle: TextStyle(color: ws.textFaint),
            ),
          ),
        );
      case BridgeFieldKind.checkbox:
        return Align(
          alignment: Alignment.centerLeft,
          child: GestureDetector(
            onTap: () => _write(
              field,
              BridgeCell(
                kind: BridgeCellKind.checkbox,
                number: 0,
                checked: !(held?.checked ?? false),
                files: const [],
              ),
            ),
            child: CellView(
              cell:
                  held ??
                  const BridgeCell(
                    kind: BridgeCellKind.checkbox,
                    number: 0,
                    checked: false,
                    files: [],
                  ),
              kind: field.kind,
              options: const {},
              numbers: _data.numbers,
              days: days,
            ),
          ),
        );
      case BridgeFieldKind.select || BridgeFieldKind.date:
        final date = field.kind == BridgeFieldKind.date;
        final shown = held == null
            ? Text(
                'Empty',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textFaint,
                ),
              )
            : CellView(
                cell: held,
                kind: field.kind,
                options: _data.options,
                numbers: _data.numbers,
                days: days,
              );
        return AnchoredMenu(
          controller: _picks.putIfAbsent(field.id, OverlayPortalController.new),
          group: widget.group,
          height: date
              ? DatePicker.height
              : SelectPicker.heightFor(field.options.length),
          onTapOutside: (_) => _closePick(),
          menu: (_) => date
              ? DatePicker(
                  text: _pickText,
                  locale: _data.days,
                  onDay: (day) {
                    _write(
                      field,
                      textValue(BridgeCellKind.date, storedDays.format(day)),
                    );
                    _closePick();
                  },
                  onEnter: () {
                    final typed = _pickText.text.trim();
                    final day = parseDay(typed, _data.days);
                    if (typed.isEmpty) {
                      if (held != null) _write(field, emptyValue);
                    } else if (day != null) {
                      _write(
                        field,
                        textValue(BridgeCellKind.date, storedDays.format(day)),
                      );
                    }
                    _closePick();
                  },
                  onClear: () {
                    _write(field, emptyValue);
                    _closePick();
                  },
                  onClose: _closePick,
                  onTab: (_) => _closePick(),
                )
              : SelectPicker(
                  field: field,
                  text: _pickText,
                  onPick: (option) {
                    _write(field, textValue(BridgeCellKind.select, option.id));
                    _closePick();
                  },
                  onCreate: (name) {
                    unawaited(_create(field, name));
                    _closePick();
                  },
                  onClose: _closePick,
                  onTab: (_) => _closePick(),
                ),
          child: GestureDetector(
            key: Key('pick ${field.id}'),
            behavior: HitTestBehavior.opaque,
            onTap: () {
              _closePick();
              final day = date ? DateTime.tryParse(held?.text ?? '') : null;
              final typed = day == null ? '' : days.format(day);
              _pickText.text = typed;
              _picks[field.id]?.show();
            },
            child: Align(alignment: Alignment.centerLeft, child: shown),
          ),
        );
      case BridgeFieldKind.file:
        return Align(
          alignment: Alignment.centerLeft,
          child: held == null
              ? Text(
                  'Empty',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textFaint,
                  ),
                )
              : CellView(
                  cell: held,
                  kind: field.kind,
                  options: const {},
                  numbers: _data.numbers,
                  days: days,
                ),
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Focus(
      focusNode: _focus,
      onKeyEvent: (_, event) {
        if (event is KeyDownEvent &&
            event.logicalKey == LogicalKeyboardKey.escape) {
          widget.onClose();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: Container(
        key: const Key('row popover'),
        width: RowPopover.width,
        padding: const EdgeInsets.all(WsMetrics.space3),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: Material(
          type: MaterialType.transparency,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      rowTitle(widget.row, _data.database),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsProse,
                        fontWeight: WsMetrics.fwSemibold,
                        color: ws.text,
                      ),
                    ),
                  ),
                  MoreMenu(
                    key: const Key('row popover actions'),
                    label: 'Row actions',
                    items: [
                      (
                        LucideIcons.trash,
                        'Delete row',
                        () {
                          widget.onClose();
                          unawaited(
                            _data
                                .write(
                                  _data.change(
                                    BridgeMutationKind.removeRow,
                                    row: widget.row.id,
                                  ),
                                )
                                .then((_) {}, onError: (Object _) {}),
                          );
                        },
                      ),
                    ],
                  ),
                ],
              ),
              const SizedBox(height: WsMetrics.space2),
              for (final field in _data.database.fields)
                SizedBox(
                  height: 30,
                  child: Row(
                    children: [
                      SizedBox(
                        width: 120,
                        child: Row(
                          children: [
                            Icon(
                              fieldKindIcon(field.kind),
                              size: 14,
                              color: ws.textFaint,
                            ),
                            const SizedBox(width: WsMetrics.space2),
                            Flexible(
                              child: Text(
                                field.name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  fontSize: WsMetrics.fsCompact,
                                  color: ws.textMuted,
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space2),
                      Expanded(child: _value(field)),
                    ],
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
