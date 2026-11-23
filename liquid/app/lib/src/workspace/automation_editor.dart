import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/automation.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'automations.dart';
import 'chart_block.dart' show ChoiceList;
import 'explorer.dart' show untitled;
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

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

bool _needsValue(BridgeMatch match) => switch (match) {
  BridgeMatch.empty || BridgeMatch.notEmpty || BridgeMatch.between => false,
  _ => true,
};

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

class AutomationEditor extends StatefulWidget {
  const AutomationEditor({
    super.key,
    required this.state,
    required this.automation,
    required this.onBack,
    required this.onClose,
  });

  final WorkspaceState state;
  final String automation;
  final VoidCallback onBack;
  final VoidCallback onClose;

  @override
  State<AutomationEditor> createState() => _AutomationEditorState();
}

class _AutomationEditorState extends State<AutomationEditor> {
  final _databases = <String, BridgeDatabase>{};
  final _texts = <String, TextEditingController>{};
  final _name = TextEditingController();
  String? _sentName;
  BridgeTriggerKind? _kind;
  String? _table;
  String? _field;
  BridgeTrigger? _stored;

  WorkspaceState get _state => widget.state;

  BridgeWorkspaceAutomation? get _automation => _state.automations.value
      .where((kept) => kept.id == widget.automation)
      .firstOrNull;

  BridgeAutomationSetup get _setup => _automation?.setup ?? newAutomation;

  BridgeDatabase? get _database => _databases[_table];

  @override
  void initState() {
    super.initState();
    _state.automations.addListener(_changed);
    _state.tables.addListener(_changed);
    _sync();
  }

  @override
  void dispose() {
    _state.automations.removeListener(_changed);
    _state.tables.removeListener(_changed);
    _name.dispose();
    for (final text in _texts.values) {
      text.dispose();
    }
    super.dispose();
  }

  bool _seen = false;

  void _changed() {
    if (_automation == null && _seen) return widget.onBack();
    _sync();
    if (mounted) setState(() {});
  }

  void _sync() {
    final automation = _automation;
    if (automation == null) return;
    if (!_seen) writeText(_name, TextEditingValue(text: automation.setup.name));
    _seen = true;
    final trigger = automation.setup.trigger;
    if (trigger != null && trigger != _stored) {
      _stored = trigger;
      _kind = trigger.kind;
      _table = trigger.table;
      _field = trigger.field;
    }
    for (final table in {
      ?_table,
      for (final step in automation.setup.steps) ?step.table,
    }) {
      _need(table);
    }
  }

  void _need(String table) {
    if (_databases.containsKey(table)) return;
    _databases[table] = const BridgeDatabase(name: '', fields: []);
    unawaited(() async {
      try {
        final answer = await _state.read(
          query(BridgeQueryKind.database, block: table),
        );
        if (!mounted) return;
        setState(
          () => _databases[table] =
              answer.database ?? const BridgeDatabase(name: '', fields: []),
        );
      } on Object catch (error) {
        _state.readFailed(error);
      }
    }());
  }

  void _write({
    String? name,
    BridgeTrigger? trigger,
    List<BridgeFilter>? conditions,
    List<BridgeStep>? steps,
  }) {
    final setup = _setup;
    unawaited(
      _state
          .apply(
            mutation(
              BridgeMutationKind.setAutomation,
              automation: widget.automation,
              automationSetup: BridgeAutomationSetup(
                name: name ?? setup.name,
                trigger: trigger ?? setup.trigger,
                conditions: conditions ?? setup.conditions,
                steps: steps ?? setup.steps,
              ),
            ),
          )
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  void _rename(String name) {
    if (name == (_sentName ?? _setup.name)) return;
    _sentName = name;
    _write(name: name);
  }

  void _setTrigger({BridgeTriggerKind? kind, String? table, String? field}) {
    setState(() {
      if (kind != null && kind != _kind) {
        _kind = kind;
        _field = null;
      }
      if (table != null && table != _table) {
        _table = table;
        _field = null;
        _need(table);
      }
      if (field != null) _field = field;
    });
    final trigger = switch ((_kind, _table, _field)) {
      (BridgeTriggerKind.rowAdded, final table?, _) => BridgeTrigger(
        kind: BridgeTriggerKind.rowAdded,
        table: table,
      ),
      (final kind?, final table?, final field?) => BridgeTrigger(
        kind: kind,
        table: table,
        field: field,
      ),
      _ => null,
    };
    if (trigger == null) return;
    final moved = _setup.trigger?.table != trigger.table;
    _write(
      trigger: trigger,
      conditions: moved ? const [] : null,
      steps: moved
          ? [
              for (final step in _setup.steps)
                if (step.kind == BridgeStepKind.addRow) step,
            ]
          : null,
    );
  }

  List<BridgeField> _fields(bool Function(BridgeField) keep) => [
    for (final field in _database?.fields ?? const <BridgeField>[])
      if (keep(field)) field,
  ];

  BridgeField? _fieldOf(String? id) =>
      _database?.fields.where((field) => field.id == id).firstOrNull;

  void _setCondition(int index, BridgeFilter condition) => _write(
    conditions: [
      for (final (at, kept) in _setup.conditions.indexed)
        at == index ? condition : kept,
    ],
  );

  void _setStep(int index, BridgeStep step) => _write(
    steps: [
      for (final (at, kept) in _setup.steps.indexed) at == index ? step : kept,
    ],
  );

  TextEditingController _text(String key, String stored) {
    final text = _texts.putIfAbsent(key, TextEditingController.new);
    if (!_focused.contains(key) && text.text != stored) {
      writeText(text, TextEditingValue(text: stored));
    }
    return text;
  }

  final _focused = <String>{};

  BridgeCell? _typed(BridgeField field, String typed) => switch (field.kind) {
    BridgeFieldKind.number => switch (double.tryParse(
      typed.trim().replaceAll(',', '.'),
    )) {
      final number? => _cell(BridgeCellKind.number, number: number),
      null => null,
    },
    BridgeFieldKind.date => switch (DateTime.tryParse(typed.trim())) {
      final day? => _cell(BridgeCellKind.date, text: _day.format(day)),
      null => null,
    },
    _ => _cell(BridgeCellKind.text, text: typed),
  };

  String _cellText(BridgeCell? cell) => switch (cell?.kind) {
    BridgeCellKind.number when cell!.number == cell.number.truncateToDouble() =>
      cell.number.toInt().toString(),
    BridgeCellKind.number => cell!.number.toString(),
    BridgeCellKind.text || BridgeCellKind.date => cell!.text ?? '',
    _ => '',
  };

  Widget _value(
    String key,
    BridgeField? field,
    BridgeCell? value,
    ValueChanged<BridgeCell> onSet, {
    bool clear = false,
  }) {
    if (field == null) return const SizedBox.shrink();
    switch (field.kind) {
      case BridgeFieldKind.select:
        return Token(
          name: key,
          label:
              field.options
                  .where((option) => option.id == value?.text)
                  .firstOrNull
                  ?.name ??
              (clear ? 'empty' : 'an option'),
          choices: [
            if (clear)
              (
                'empty',
                value?.kind == BridgeCellKind.empty,
                () => onSet(_empty),
              ),
            for (final option in field.options)
              (
                option.name,
                option.id == value?.text,
                () => onSet(_cell(BridgeCellKind.select, text: option.id)),
              ),
          ],
        );
      case BridgeFieldKind.checkbox:
        return Token(
          name: key,
          label: value?.checked ?? false ? 'checked' : 'not checked',
          choices: [
            (
              'checked',
              value?.checked ?? false,
              () => onSet(_cell(BridgeCellKind.checkbox, checked: true)),
            ),
            (
              'not checked',
              !(value?.checked ?? false),
              () => onSet(_cell(BridgeCellKind.checkbox, checked: false)),
            ),
          ],
        );
      default:
        final text = _text(key, _cellText(value));
        void submit(String typed) {
          if (typed == _cellText(value)) return;
          if (clear && typed.trim().isEmpty) return onSet(_empty);
          final cell = _typed(field, typed);
          if (cell == null) {
            _state.report(
              field.kind == BridgeFieldKind.date
                  ? 'Type a date as 2026-11-30.'
                  : 'Type a number.',
            );
            return;
          }
          onSet(cell);
        }

        return TokenField(
          key: Key(key),
          controller: text,
          hint: switch (field.kind) {
            BridgeFieldKind.number => '0',
            BridgeFieldKind.date => '2026-11-30',
            _ => clear ? 'empty' : 'text',
          },
          onFocus: (focused) {
            focused ? _focused.add(key) : _focused.remove(key);
            if (!focused) submit(text.text);
          },
          onSubmitted: submit,
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final automation = _automation;
    if (automation == null) return const SizedBox.shrink();
    final setup = automation.setup;
    final attention = automation.attention;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        DialogHeader(
          onBack: widget.onBack,
          onClose: widget.onClose,
          title: Row(
            children: [
              Text(
                'Automations ›',
                style: TextStyle(
                  fontWeight: WsMetrics.fwRegular,
                  color: ws.textMuted,
                ),
              ),
              const SizedBox(width: WsMetrics.space2),
              Expanded(
                child: Focus(
                  onFocusChange: (focused) {
                    if (!focused) _rename(_name.text);
                  },
                  child: TextField(
                    key: const Key('automation name'),
                    controller: _name,
                    onSubmitted: _rename,
                    decoration: InputDecoration.collapsed(
                      hintText: untitled,
                      hintStyle: TextStyle(color: ws.textFaint),
                    ),
                  ),
                ),
              ),
            ],
          ),
          actions: [
            SecondaryButton(
              label: 'Test',
              icon: LucideIcons.play,
              onPressed: setup.trigger == null
                  ? null
                  : () => unawaited(_state.testAutomation(widget.automation)),
            ),
            Text(
              automation.on_ ? 'On' : 'Off',
              style: TextStyle(color: ws.textMuted),
            ),
            SizedBox(
              width: 36,
              height: 24,
              child: FittedBox(
                child: Switch(
                  key: const Key('automation on'),
                  materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  value: automation.on_,
                  onChanged: attention != null
                      ? null
                      : (on) => unawaited(
                          _state
                              .apply(
                                mutation(
                                  BridgeMutationKind.setAutomationOn,
                                  automation: widget.automation,
                                  on: on,
                                ),
                              )
                              .then<void>((_) {}, onError: (Object _) {}),
                        ),
                ),
              ),
            ),
            MoreMenu(
              label: 'More',
              items: [
                (
                  LucideIcons.trash,
                  'Delete Automation',
                  () => unawaited(
                    _state
                        .apply(
                          mutation(
                            BridgeMutationKind.removeAutomation,
                            automation: widget.automation,
                          ),
                        )
                        .then<void>((_) {}, onError: (Object _) {}),
                  ),
                ),
              ],
            ),
          ],
        ),
        Flexible(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space7,
              WsMetrics.space6,
              WsMetrics.space7,
              WsMetrics.space7,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: WsMetrics.space4,
              children: [
                if (attention != null) _Attention(attention: attention),
                _when(),
                if (setup.trigger != null) ...[
                  for (final (index, condition) in setup.conditions.indexed)
                    _condition(index, condition),
                  _Line(
                    keyword: setup.conditions.isEmpty ? 'If' : 'and',
                    children: [
                      Token(
                        name: 'add condition',
                        label: 'and',
                        add: true,
                        choices: [
                          for (final field in _fields(
                            (field) =>
                                field.kind != BridgeFieldKind.formula &&
                                field.kind != BridgeFieldKind.button,
                          ))
                            (
                              field.name,
                              false,
                              () => _write(
                                conditions: [
                                  ...setup.conditions,
                                  BridgeFilter(
                                    field: field.id,
                                    matches:
                                        field.kind == BridgeFieldKind.checkbox
                                        ? BridgeMatch.is_
                                        : BridgeMatch.notEmpty,
                                    value:
                                        field.kind == BridgeFieldKind.checkbox
                                        ? _cell(
                                            BridgeCellKind.checkbox,
                                            checked: true,
                                          )
                                        : null,
                                  ),
                                ],
                              ),
                            ),
                        ],
                      ),
                    ],
                  ),
                  for (final (index, step) in setup.steps.indexed)
                    _step(index, step),
                  _Line(
                    keyword: setup.steps.isEmpty ? 'Then' : 'and',
                    children: [
                      Token(
                        name: 'add action',
                        label: 'action',
                        add: true,
                        choices: [
                          (
                            'set a field',
                            false,
                            () => _write(
                              steps: [
                                ...setup.steps,
                                if (_fields(
                                      (field) => _settable.contains(field.kind),
                                    ).firstOrNull
                                    case final field?)
                                  BridgeStep(
                                    kind: BridgeStepKind.set_,
                                    field: field.id,
                                    value: _empty,
                                    cells: const [],
                                  ),
                              ],
                            ),
                          ),
                          (
                            'add a row',
                            false,
                            () => _write(
                              steps: [
                                ...setup.steps,
                                BridgeStep(
                                  kind: BridgeStepKind.addRow,
                                  table: _table,
                                  cells: const [],
                                ),
                              ],
                            ),
                          ),
                          (
                            'show a notification',
                            false,
                            () => _write(
                              steps: [
                                ...setup.steps,
                                const BridgeStep(
                                  kind: BridgeStepKind.notify,
                                  cells: [],
                                  message: '',
                                ),
                              ],
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                ],
                const SizedBox(height: WsMetrics.space2),
                _Facts(
                  facts: [
                    ('Runs as', '$runsAs · you'),
                    (
                      'Belongs to',
                      switch (automation.owner) {
                        final owner? =>
                          '${tableName(_state, owner)} · deleted together with it',
                        null =>
                          'Workspace · stays when Blocks it uses are deleted',
                      },
                    ),
                    ('Last run', lastRun(automation.last)),
                  ],
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }

  Widget _when() {
    final kind = _kind;
    final table = _table;
    final field = _fieldOf(_field);
    final tables = [
      for (final known in _state.tables.value)
        (
          known.name.isEmpty ? untitled : known.name,
          known.id == table,
          () => _setTrigger(table: known.id),
        ),
    ];
    final tableToken = Token(
      name: 'table',
      label: table == null ? 'a Table' : tableName(_state, table),
      reference: table != null,
      danger:
          table != null &&
          !_state.tables.value.any((known) => known.id == table),
      choices: tables,
    );
    final fieldToken = Token(
      name: 'field',
      label:
          field?.name ??
          (_field == null
              ? kind == BridgeTriggerKind.clicked
                    ? 'a button'
                    : 'a field'
              : 'a removed field'),
      reference: _field != null,
      danger: _field != null && field == null,
      choices: [
        for (final known in _fields(
          (known) => kind == BridgeTriggerKind.clicked
              ? known.kind == BridgeFieldKind.button
              : known.kind != BridgeFieldKind.formula &&
                    known.kind != BridgeFieldKind.button,
        ))
          (known.name, known.id == _field, () => _setTrigger(field: known.id)),
      ],
    );
    return _Line(
      keyword: 'When',
      children: [
        Token(
          name: 'when',
          label: switch (kind) {
            null => 'choose when',
            BridgeTriggerKind.rowAdded => 'a row is added',
            BridgeTriggerKind.fieldChanged => 'the field',
            BridgeTriggerKind.clicked => 'the button',
          },
          choices: [
            for (final (label, choice) in const [
              ('a row is added', BridgeTriggerKind.rowAdded),
              ('a field changes', BridgeTriggerKind.fieldChanged),
              ('a button is clicked', BridgeTriggerKind.clicked),
            ])
              (label, choice == kind, () => _setTrigger(kind: choice)),
          ],
        ),
        if (kind == BridgeTriggerKind.rowAdded) ...[
          const Text('to'),
          tableToken,
        ],
        if (kind == BridgeTriggerKind.fieldChanged ||
            kind == BridgeTriggerKind.clicked) ...[
          fieldToken,
          const Text('in'),
          tableToken,
          Text(kind == BridgeTriggerKind.clicked ? 'is clicked' : 'changes'),
        ],
      ],
    );
  }

  Widget _condition(int index, BridgeFilter condition) {
    final field = _fieldOf(condition.field);
    final tests = _tests(field?.kind ?? BridgeFieldKind.file);
    final shown =
        tests
            .where(
              (test) =>
                  test.$2 == condition.matches &&
                  (test.$3 == null || test.$3 == condition.value?.checked),
            )
            .firstOrNull
            ?.$1 ??
        'is';
    return _Line(
      keyword: index == 0 ? 'If' : 'and',
      onRemove: () => _write(
        conditions: [
          for (final (at, kept) in _setup.conditions.indexed)
            if (at != index) kept,
        ],
      ),
      removeKey: 'remove condition $index',
      children: [
        Token(
          name: 'condition field $index',
          label: field?.name ?? 'a removed field',
          reference: true,
          danger: field == null,
          choices: [
            for (final known in _fields(
              (known) =>
                  known.kind != BridgeFieldKind.formula &&
                  known.kind != BridgeFieldKind.button,
            ))
              (
                known.name,
                known.id == condition.field,
                () => _setCondition(
                  index,
                  BridgeFilter(field: known.id, matches: BridgeMatch.notEmpty),
                ),
              ),
          ],
        ),
        Token(
          name: 'test $index',
          label: shown,
          choices: [
            for (final (label, match, checked) in tests)
              (
                label,
                label == shown,
                () {
                  final value = checked != null
                      ? _cell(BridgeCellKind.checkbox, checked: checked)
                      : !_needsValue(match) || field == null
                      ? null
                      : condition.value != null &&
                            _needsValue(condition.matches)
                      ? condition.value
                      : match == BridgeMatch.contains
                      ? _cell(BridgeCellKind.text, text: '')
                      : _firstValue(field);
                  if (_needsValue(match) && value == null) return;
                  _setCondition(
                    index,
                    BridgeFilter(
                      field: condition.field,
                      matches: match,
                      value: value,
                    ),
                  );
                },
              ),
          ],
        ),
        if (_needsValue(condition.matches) &&
            field?.kind != BridgeFieldKind.checkbox)
          _value(
            'value $index',
            condition.matches == BridgeMatch.contains
                ? BridgeField(
                    id: condition.field,
                    name: '',
                    kind: BridgeFieldKind.text,
                    options: const [],
                    formula: '',
                  )
                : field,
            condition.value,
            (value) => _setCondition(
              index,
              BridgeFilter(
                field: condition.field,
                matches: condition.matches,
                value: value,
              ),
            ),
          ),
      ],
    );
  }

  Widget _step(int index, BridgeStep step) {
    final keyword = index == 0 ? 'Then' : 'and';
    void remove() => _write(
      steps: [
        for (final (at, kept) in _setup.steps.indexed)
          if (at != index) kept,
      ],
    );
    final removeKey = 'remove step $index';
    switch (step.kind) {
      case BridgeStepKind.set_:
        final field = _fieldOf(step.field);
        return _Line(
          keyword: keyword,
          onRemove: remove,
          removeKey: removeKey,
          children: [
            const Text('set'),
            Token(
              name: 'set field $index',
              label: field?.name ?? 'a removed field',
              reference: true,
              danger: field == null,
              choices: [
                for (final known in _fields(
                  (known) => _settable.contains(known.kind),
                ))
                  (
                    known.name,
                    known.id == step.field,
                    () => _setStep(
                      index,
                      BridgeStep(
                        kind: BridgeStepKind.set_,
                        field: known.id,
                        value: _empty,
                        cells: const [],
                      ),
                    ),
                  ),
              ],
            ),
            const Text('to'),
            _value(
              'set value $index',
              field,
              step.value,
              (value) => _setStep(
                index,
                BridgeStep(
                  kind: BridgeStepKind.set_,
                  field: step.field,
                  value: value,
                  cells: const [],
                ),
              ),
              clear: true,
            ),
          ],
        );
      case BridgeStepKind.addRow:
        final table = step.table;
        return _Line(
          keyword: keyword,
          onRemove: remove,
          removeKey: removeKey,
          children: [
            const Text('add a row to'),
            Token(
              name: 'row table $index',
              label: table == null ? 'a Table' : tableName(_state, table),
              reference: true,
              danger:
                  table != null &&
                  !_state.tables.value.any((known) => known.id == table),
              choices: [
                for (final known in _state.tables.value)
                  (
                    known.name.isEmpty ? untitled : known.name,
                    known.id == table,
                    () => _setStep(
                      index,
                      BridgeStep(
                        kind: BridgeStepKind.addRow,
                        table: known.id,
                        cells: const [],
                      ),
                    ),
                  ),
              ],
            ),
          ],
        );
      case BridgeStepKind.notify:
        final key = 'message $index';
        final text = _text(key, shownMessage(step.message ?? '', _database));
        void submit(String typed) {
          final stored = storedMessage(typed, _database);
          if (stored == step.message) return;
          _setStep(
            index,
            BridgeStep(
              kind: BridgeStepKind.notify,
              cells: const [],
              message: stored,
            ),
          );
        }

        return _Line(
          keyword: keyword,
          onRemove: remove,
          removeKey: removeKey,
          children: [
            Icon(LucideIcons.bell, size: 14, color: context.ws.textMuted),
            const Text('show a notification'),
            TokenField(
              key: Key(key),
              controller: text,
              width: 320,
              hint: 'Message; {Field} shows its value',
              onFocus: (focused) {
                focused ? _focused.add(key) : _focused.remove(key);
                if (!focused) submit(text.text);
              },
              onSubmitted: submit,
            ),
          ],
        );
    }
  }
}

String lastRun(BridgeLastRun? last) {
  if (last == null) return 'Not run yet';
  final at = DateTime.fromMillisecondsSinceEpoch(last.at.toInt() * 1000);
  final now = DateTime.now();
  final day = DateUtils.isSameDay(at, now)
      ? 'today'
      : DateFormat.yMMMd('en_US').format(at);
  String rows(int count, String verb) =>
      '$verb $count ${count == 1 ? 'row' : 'rows'}';
  final did = [
    if (last.set_ > 0) rows(last.set_.toInt(), 'set'),
    if (last.added > 0) rows(last.added.toInt(), 'added'),
    if (last.notified > 0)
      '${last.notified} ${last.notified == 1 ? 'notification' : 'notifications'}',
  ];
  return [
    '$day ${DateFormat.Hm('en_US').format(at)}',
    if (did.isEmpty) 'changed nothing' else did.join(' · '),
  ].join(' · ');
}

class _Attention extends StatelessWidget {
  const _Attention({required this.attention});

  final BridgeAttention attention;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final reason = switch (attention) {
      BridgeAttention(kind: BridgeAttentionKind.tableDeleted, :final table) =>
        'Turned off: its Database “$table” was deleted. Choose another '
            'Database to repair it.',
      BridgeAttention(:final table, :final field) =>
        'Turned off: its field “$field” of “$table” was removed. Choose '
            'another field to repair it.',
    };
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: ws.warningSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: WsMetrics.space3,
        children: [
          Icon(LucideIcons.triangleAlert, size: 16, color: ws.warning),
          Expanded(
            child: Text(reason, style: TextStyle(color: ws.warning)),
          ),
        ],
      ),
    );
  }
}

class _Facts extends StatelessWidget {
  const _Facts({required this.facts});

  final List<(String, String)> facts;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space2,
      children: [
        for (final (label, value) in facts)
          Row(
            children: [
              SizedBox(
                width: 120,
                child: Text(label, style: TextStyle(color: ws.textMuted)),
              ),
              Expanded(child: Text(value)),
            ],
          ),
      ],
    );
  }
}

class _Line extends StatelessWidget {
  const _Line({
    required this.keyword,
    required this.children,
    this.onRemove,
    this.removeKey,
  });

  final String keyword;
  final List<Widget> children;
  final VoidCallback? onRemove;
  final String? removeKey;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return DefaultTextStyle.merge(
      style: TextStyle(fontSize: WsMetrics.fsProse, color: ws.text),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 64,
            height: 30,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                keyword,
                style: TextStyle(
                  color: ws.textMuted,
                  fontWeight: WsMetrics.fwSemibold,
                ),
              ),
            ),
          ),
          Expanded(
            child: Wrap(
              spacing: WsMetrics.space2,
              runSpacing: WsMetrics.space2,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                ...children,
                if (onRemove case final remove?)
                  Tooltip(
                    message: 'Remove',
                    child: InkWell(
                      key: Key(removeKey ?? 'remove'),
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      onTap: remove,
                      child: SizedBox.square(
                        dimension: 30,
                        child: Icon(
                          LucideIcons.x,
                          size: 14,
                          color: ws.textFaint,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class Token extends StatefulWidget {
  const Token({
    super.key,
    required this.name,
    required this.label,
    required this.choices,
    this.reference = false,
    this.danger = false,
    this.add = false,
  });

  final String name;
  final String label;
  final List<(String, bool, VoidCallback)> choices;
  final bool reference;
  final bool danger;
  final bool add;

  @override
  State<Token> createState() => _TokenState();
}

class _TokenState extends State<Token> {
  final _menu = OverlayPortalController();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (fill, color, border) = widget.danger
        ? (ws.dangerSubtle, ws.danger, Colors.transparent)
        : widget.reference
        ? (ws.accentSubtle, ws.accent, Colors.transparent)
        : widget.add
        ? (Colors.transparent, ws.textFaint, ws.borderStrong)
        : (ws.subtle, ws.text, ws.border);
    return AnchoredMenu(
      controller: _menu,
      height: 32.0 * widget.choices.length + 8,
      onTapOutside: (_) => setState(_menu.hide),
      menu: (_) => ChoiceList(
        choices: [
          for (final (label, chosen, onTap) in widget.choices)
            (
              label,
              chosen,
              () {
                setState(_menu.hide);
                if (!chosen) onTap();
              },
            ),
        ],
      ),
      child: Semantics(
        button: true,
        child: InkWell(
          key: Key('token ${widget.name}'),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          onTap: widget.choices.isEmpty ? null : () => setState(_menu.toggle),
          child: Container(
            height: 30,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: fill,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              border: Border.all(color: border),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (widget.add) ...[
                  Icon(LucideIcons.plus, size: 14, color: color),
                  const SizedBox(width: WsMetrics.space1),
                ],
                Text(
                  widget.label,
                  style: TextStyle(
                    fontSize: WsMetrics.fsUi,
                    color: color,
                    fontWeight: widget.reference ? WsMetrics.fwMedium : null,
                    decoration: widget.danger
                        ? TextDecoration.lineThrough
                        : null,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class TokenField extends StatelessWidget {
  const TokenField({
    super.key,
    required this.controller,
    required this.hint,
    required this.onFocus,
    required this.onSubmitted,
    this.width = 140,
  });

  final TextEditingController controller;
  final String hint;
  final ValueChanged<bool> onFocus;
  final ValueChanged<String> onSubmitted;
  final double width;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      width: width,
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        border: Border.all(color: ws.border),
      ),
      child: Focus(
        onFocusChange: onFocus,
        child: TextField(
          controller: controller,
          onSubmitted: onSubmitted,
          style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
          decoration: InputDecoration.collapsed(
            hintText: hint,
            hintStyle: TextStyle(color: ws.textFaint),
          ),
        ),
      ),
    );
  }
}
