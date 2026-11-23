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
import 'automation_tokens.dart';
import 'automations.dart';
import 'explorer.dart' show untitled;
import 'roles.dart' show RunsAs;
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

part 'automation_cells.dart';
part 'automation_sentence.dart';
part 'automation_values.dart';
part 'automation_widgets.dart';

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

  void _update(VoidCallback change) => setState(change);

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

  final _focused = <String>{};

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
                  onChanged: (on) => unawaited(
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
                                  _firstCondition(field),
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
                    ('Runs as', RunsAs(state: _state, automation: automation)),
                    (
                      'Belongs to',
                      Text(switch (automation.owner) {
                        final owner? =>
                          '${tableName(_state, owner)} · deleted together with it',
                        null =>
                          'Workspace · stays when Tables it uses are deleted',
                      }),
                    ),
                    ('Last run', Text(lastRun(automation.last))),
                  ],
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}
