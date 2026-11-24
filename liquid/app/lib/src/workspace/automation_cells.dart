part of 'automation_editor.dart';

typedef _Test = (String label, BridgeMatch match, bool? checked);

const _settable = {
  BridgeFieldKind.text,
  BridgeFieldKind.number,
  BridgeFieldKind.select,
  BridgeFieldKind.date,
  BridgeFieldKind.checkbox,
};

List<_Test> _tests(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => const [
    ('is', BridgeMatch.is_, null),
    ('is not', BridgeMatch.isNot, null),
    ('contains', BridgeMatch.contains, null),
    ('is empty', BridgeMatch.empty, null),
    ('is not empty', BridgeMatch.notEmpty, null),
  ],
  BridgeFieldKind.number => const [
    ('is', BridgeMatch.is_, null),
    ('is not', BridgeMatch.isNot, null),
    ('is more than', BridgeMatch.above, null),
    ('is less than', BridgeMatch.below, null),
    ('is empty', BridgeMatch.empty, null),
    ('is not empty', BridgeMatch.notEmpty, null),
  ],
  BridgeFieldKind.date => const [
    ('is', BridgeMatch.is_, null),
    ('is before', BridgeMatch.below, null),
    ('is after', BridgeMatch.above, null),
    ('is empty', BridgeMatch.empty, null),
    ('is not empty', BridgeMatch.notEmpty, null),
  ],
  BridgeFieldKind.select => const [
    ('is', BridgeMatch.is_, null),
    ('is not', BridgeMatch.isNot, null),
    ('is empty', BridgeMatch.empty, null),
    ('is not empty', BridgeMatch.notEmpty, null),
  ],
  BridgeFieldKind.checkbox => const [
    ('is checked', BridgeMatch.is_, true),
    ('is not checked', BridgeMatch.is_, false),
  ],
  _ => const [
    ('is empty', BridgeMatch.empty, null),
    ('is not empty', BridgeMatch.notEmpty, null),
  ],
};

BridgeCell _cell(
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

const _empty = BridgeCell(
  kind: BridgeCellKind.empty,
  number: 0,
  checked: false,
  files: [],
);

final _day = DateFormat('yyyy-MM-dd', 'en_US');

BridgeCell? _firstValue(BridgeField field) => switch (field.kind) {
  BridgeFieldKind.number => _cell(BridgeCellKind.number),
  BridgeFieldKind.date => _cell(
    BridgeCellKind.date,
    text: _day.format(DateTime.now()),
  ),
  BridgeFieldKind.select => switch (field.options.firstOrNull) {
    final option? => _cell(BridgeCellKind.select, text: option.id),
    null => null,
  },
  _ => _cell(BridgeCellKind.text, text: ''),
};

BridgeFilter _firstCondition(BridgeField field) =>
    field.kind == BridgeFieldKind.checkbox
    ? BridgeFilter(
        field: field.id,
        matches: BridgeMatch.is_,
        value: _cell(BridgeCellKind.checkbox, checked: true),
      )
    : BridgeFilter(field: field.id, matches: BridgeMatch.notEmpty);

bool _needsValue(BridgeMatch match) => switch (match) {
  BridgeMatch.empty || BridgeMatch.notEmpty || BridgeMatch.between => false,
  _ => true,
};

bool _conditionable(BridgeField field) =>
    field.kind != BridgeFieldKind.formula &&
    field.kind != BridgeFieldKind.button;

final _braced = RegExp(r'\{([^{}]+)\}');

String shownMessage(String stored, BridgeDatabase? database) =>
    stored.replaceAllMapped(_braced, (match) {
      final field = database?.fields
          .where((known) => known.id == match[1])
          .firstOrNull;
      return field == null ? match[0]! : '{${field.name}}';
    });

String storedMessage(String shown, BridgeDatabase? database) =>
    shown.replaceAllMapped(_braced, (match) {
      final field = database?.fields
          .where(
            (known) =>
                known.name == match[1] && known.kind != BridgeFieldKind.button,
          )
          .firstOrNull;
      return field == null ? match[0]! : '{${field.id}}';
    });
