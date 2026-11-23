import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/automation.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'automation_editor.dart';
import 'explorer.dart' show untitled;
import 'workspace_state.dart';

const automationsWidth = 1000.0;

const newAutomation = BridgeAutomationSetup(
  name: '',
  conditions: [],
  steps: [],
);

String tableName(WorkspaceState state, String table) => switch (state
    .tables
    .value
    .where((known) => known.id == table)
    .firstOrNull) {
  final known? => known.name.isEmpty ? untitled : known.name,
  null => 'a deleted Table',
};

String fieldName(BridgeDatabase? database, String? field) =>
    database?.fields.where((known) => known.id == field).firstOrNull?.name ??
    'a removed field';

String whenText(
  WorkspaceState state,
  BridgeTrigger? trigger,
  BridgeDatabase? database,
) => switch (trigger) {
  null => 'Not set yet',
  BridgeTrigger(kind: BridgeTriggerKind.rowAdded) => 'a row is added',
  BridgeTrigger(kind: BridgeTriggerKind.fieldChanged, :final field) =>
    '“${fieldName(database, field)}” changes',
  BridgeTrigger(kind: BridgeTriggerKind.clicked, :final field, :final table) =>
    '“${fieldName(database, field)}” in ${tableName(state, table)} is clicked',
};

class AutomationsDialog extends StatefulWidget {
  const AutomationsDialog({
    super.key,
    required this.state,
    required this.onClose,
  });

  final WorkspaceState state;
  final VoidCallback onClose;

  @override
  State<AutomationsDialog> createState() => _AutomationsDialogState();
}

class _AutomationsDialogState extends State<AutomationsDialog> {
  String? _open;
  final _databases = <String, BridgeDatabase>{};

  WorkspaceState get _state => widget.state;

  @override
  void initState() {
    super.initState();
    _state.automations.addListener(_readDatabases);
    _readDatabases();
  }

  @override
  void dispose() {
    _state.automations.removeListener(_readDatabases);
    super.dispose();
  }

  void _readDatabases() {
    for (final automation in _state.automations.value) {
      final table = automation.setup.trigger?.table;
      if (table == null || _databases.containsKey(table)) continue;
      unawaited(_readDatabase(table));
    }
  }

  Future<void> _readDatabase(String table) async {
    try {
      final answer = await _state.read(
        query(BridgeQueryKind.database, block: table),
      );
      if (!mounted) return;
      if (answer.database case final database?) {
        setState(() => _databases[table] = database);
      }
    } on Object catch (error) {
      _state.readFailed(error);
    }
  }

  Future<void> _create() async {
    final created = await _state.apply(
      mutation(
        BridgeMutationKind.createAutomation,
        automationSetup: newAutomation,
      ),
    );
    if (mounted) setState(() => _open = created.automation);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): widget.onClose,
      },
      child: Stack(
        children: [
          Positioned.fill(
            child: GestureDetector(
              onTap: widget.onClose,
              child: ColoredBox(color: Colors.black.withValues(alpha: 0.18)),
            ),
          ),
          Center(
            child: Focus(
              autofocus: true,
              child: Container(
                key: const Key('automations'),
                width: automationsWidth,
                constraints: const BoxConstraints(maxHeight: 640),
                decoration: BoxDecoration(
                  color: ws.surface,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
                  boxShadow: ws.shadow,
                ),
                child: Material(
                  type: MaterialType.transparency,
                  child: switch (_open) {
                    final open? => AutomationEditor(
                      key: ValueKey(open),
                      state: _state,
                      automation: open,
                      onBack: () => setState(() => _open = null),
                      onClose: widget.onClose,
                    ),
                    null => _list(context),
                  },
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _list(BuildContext context) {
    final ws = context.ws;
    final head = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    Widget row(List<Widget> cells, {VoidCallback? onTap, Key? key}) => InkWell(
      key: key,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(
          children: [
            for (final (index, cell) in cells.indexed)
              Expanded(flex: const [14, 10, 12, 10, 8][index], child: cell),
          ],
        ),
      ),
    );
    Widget text(String shown, {Color? color}) => Text(
      shown,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(fontSize: WsMetrics.fsUi, color: color ?? ws.text),
    );
    return ValueListenableBuilder(
      valueListenable: _state.automations,
      builder: (context, automations, _) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          DialogHeader(
            title: const Text('Automations'),
            actions: [
              PrimaryButton(
                label: 'New Automation',
                onPressed: () => unawaited(_create()),
              ),
            ],
            onClose: widget.onClose,
          ),
          Flexible(
            child: ListView(
              shrinkWrap: true,
              padding: const EdgeInsets.fromLTRB(
                WsMetrics.space7,
                WsMetrics.space2,
                WsMetrics.space7,
                WsMetrics.space6,
              ),
              children: [
                row([
                  Text('Name', style: head),
                  Text('Belongs to', style: head),
                  Text('When', style: head),
                  Text('Runs as', style: head),
                  Text('Status', style: head),
                ]),
                for (final automation in automations)
                  row(
                    key: Key('automation ${automation.id}'),
                    onTap: () => setState(() => _open = automation.id),
                    [
                      text(
                        automation.setup.name.isEmpty
                            ? untitled
                            : automation.setup.name,
                      ),
                      Row(
                        children: [
                          if (automation.owner != null) ...[
                            Icon(
                              LucideIcons.table,
                              size: 14,
                              color: ws.textFaint,
                            ),
                            const SizedBox(width: WsMetrics.space2),
                          ],
                          Flexible(
                            child: text(switch (automation.owner) {
                              final owner? => tableName(_state, owner),
                              null => 'Workspace',
                            }),
                          ),
                        ],
                      ),
                      text(
                        whenText(
                          _state,
                          automation.setup.trigger,
                          _databases[automation.setup.trigger?.table],
                        ),
                        color: ws.textMuted,
                      ),
                      text(runsAs, color: ws.textMuted),
                      AutomationStatus(automation: automation),
                    ],
                  ),
                if (automations.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(WsMetrics.space5),
                    child: Text(
                      'No Automations yet. An Automation runs when a row is '
                      'added, a field changes or a button is clicked.',
                      style: TextStyle(color: ws.textFaint),
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

const runsAs = 'Can manage';

class AutomationStatus extends StatelessWidget {
  const AutomationStatus({super.key, required this.automation});

  final BridgeWorkspaceAutomation automation;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (icon, label, color) = switch (automation) {
      BridgeWorkspaceAutomation(attention: _?) => (
        LucideIcons.triangleAlert,
        'Needs attention',
        ws.warning,
      ),
      BridgeWorkspaceAutomation(on_: true) => (null, 'On', ws.success),
      _ => (null, 'Off', ws.textFaint),
    };
    return Row(
      children: [
        if (icon == null)
          Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
          )
        else
          Icon(icon, size: 12, color: color),
        const SizedBox(width: WsMetrics.space2),
        Flexible(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: WsMetrics.fsUi, color: color),
          ),
        ),
      ],
    );
  }
}

class DialogHeader extends StatelessWidget {
  const DialogHeader({
    super.key,
    required this.title,
    required this.actions,
    required this.onClose,
    this.onBack,
  });

  final Widget title;
  final List<Widget> actions;
  final VoidCallback onClose;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 56,
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space5,
        0,
        WsMetrics.space4,
        0,
      ),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        spacing: WsMetrics.space3,
        children: [
          if (onBack case final back?)
            IconButton(
              key: const Key('automations back'),
              tooltip: 'All Automations',
              icon: Icon(LucideIcons.arrowLeft, size: 16, color: ws.textMuted),
              onPressed: back,
            ),
          Expanded(
            child: DefaultTextStyle.merge(
              style: TextStyle(
                fontSize: WsMetrics.fsProse,
                fontWeight: WsMetrics.fwSemibold,
                color: ws.text,
              ),
              child: title,
            ),
          ),
          ...actions,
          IconButton(
            tooltip: 'Close',
            icon: Icon(LucideIcons.x, size: 16, color: ws.textMuted),
            onPressed: onClose,
          ),
        ],
      ),
    );
  }
}
