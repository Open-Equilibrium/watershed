part of 'automation_editor.dart';

extension _SentenceEditing on _AutomationEditorState {
  void _setTrigger({BridgeTriggerKind? kind, String? table, String? field}) {
    _update(() {
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
    final left = _setup.trigger?.table;
    final moved = left != trigger.table;
    _write(
      trigger: trigger,
      conditions: moved ? const [] : null,
      steps: moved
          ? [
              for (final step in _setup.steps)
                if (step.kind == BridgeStepKind.addRow)
                  step
                else if (step.kind == BridgeStepKind.notify)
                  BridgeStep(
                    kind: BridgeStepKind.notify,
                    cells: const [],
                    message: shownMessage(step.message ?? '', _databases[left]),
                  ),
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
                () => _setCondition(index, _firstCondition(known)),
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
