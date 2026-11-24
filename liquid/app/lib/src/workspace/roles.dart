import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/automation.dart';
import '../rust/api/role.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'automation_tokens.dart' show Token;
import 'explorer.dart' show untitled;
import 'table_view_parts.dart' show MoreMenu;
import 'workspace_state.dart';

const capabilities = ['Read', 'Propose', 'Apply', 'Approve', 'Manage'];

bool allowed(BridgeAllows allows, String capability) => switch (capability) {
  'Read' => allows.read,
  'Propose' => allows.propose,
  'Apply' => allows.apply,
  'Approve' => allows.approve,
  _ => allows.manage,
};

BridgeAllows allowing(BridgeAllows allows, String capability, bool on) =>
    BridgeAllows(
      read: capability == 'Read' ? on : allows.read,
      propose: capability == 'Propose' ? on : allows.propose,
      apply: capability == 'Apply' ? on : allows.apply,
      approve: capability == 'Approve' ? on : allows.approve,
      manage: capability == 'Manage' ? on : allows.manage,
    );

String allowsText(BridgeAllows allows) {
  final listed = [
    for (final capability in capabilities)
      if (allowed(allows, capability)) capability,
  ];
  return listed.isEmpty ? 'nothing' : listed.join(', ');
}

const _read = BridgeAllows(
  read: true,
  propose: false,
  apply: false,
  approve: false,
  manage: false,
);

class RolesPane extends StatefulWidget {
  const RolesPane({super.key, required this.state});

  final WorkspaceState state;

  @override
  State<RolesPane> createState() => _RolesPaneState();
}

class _RolesPaneState extends State<RolesPane> {
  String? _chosen;
  final _name = TextEditingController();
  String? _named;

  WorkspaceState get _state => widget.state;

  BridgeRole? get _role {
    final roles = _state.roles.value;
    return roles.where((role) => role.id == _chosen).firstOrNull ??
        roles.firstOrNull;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _write(BridgeRole role, BridgeRoleSetup setup) => unawaited(
    _state
        .apply(
          mutation(BridgeMutationKind.setRole, role: role.id, roleSetup: setup),
        )
        .then<void>((_) {}, onError: (Object _) {}),
  );

  void _rename(BridgeRole role, String name) {
    if (name == role.setup.name) return;
    _write(role, BridgeRoleSetup(name: name, grants: role.setup.grants));
  }

  Future<void> _create() async {
    try {
      final created = await _state.apply(
        mutation(
          BridgeMutationKind.createRole,
          roleSetup: const BridgeRoleSetup(name: 'New Role', grants: []),
        ),
      );
      if (mounted) setState(() => _chosen = created.role);
    } on Object {
      return;
    }
  }

  void _assign(BridgeWorkspaceAutomation automation, String role) => unawaited(
    _state
        .apply(
          mutation(
            BridgeMutationKind.setAutomationRole,
            automation: automation.id,
            role: role,
          ),
        )
        .then<void>((_) {}, onError: (Object _) {}),
  );

  String _target(BridgeGrant grant) => switch (grant.kind) {
    BridgeTargetKind.workspace => 'All areas',
    BridgeTargetKind.area =>
      _state.areas.value
              .where((area) => area.id == grant.id)
              .firstOrNull
              ?.name ??
          'a deleted area',
    BridgeTargetKind.table => switch (_state.tables.value
        .where((table) => table.id == grant.id)
        .firstOrNull) {
      final table? => table.name.isEmpty ? untitled : table.name,
      null => 'a deleted Table',
    },
  };

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      _state.roles,
      _state.automations,
      _state.tables,
      _state.areas,
    ]),
    builder: (context, _) => _build(context),
  );

  Widget _build(BuildContext context) {
    final ws = context.ws;
    final role = _role;
    if (role != null && _named != role.id) {
      _named = role.id;
      writeText(_name, TextEditingValue(text: role.setup.name));
    }
    final automations = _state.automations.value;
    return Material(
      type: MaterialType.transparency,
      child: ListView(
        padding: const EdgeInsets.all(WsMetrics.space7),
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  'Roles',
                  style: TextStyle(
                    fontSize: WsMetrics.fsH3,
                    fontWeight: WsMetrics.fwSemibold,
                    color: ws.text,
                  ),
                ),
              ),
              SecondaryButton(
                label: 'New Role',
                icon: LucideIcons.plus,
                onPressed: () => unawaited(_create()),
              ),
            ],
          ),
          const SizedBox(height: WsMetrics.space5),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 200,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final listed in _state.roles.value)
                      _RoleRow(
                        role: listed,
                        chosen: listed.id == role?.id,
                        assigned: automations
                            .where((kept) => kept.role == listed.id)
                            .length,
                        onTap: () => setState(() => _chosen = listed.id),
                      ),
                  ],
                ),
              ),
              const SizedBox(width: WsMetrics.space6),
              if (role != null)
                Expanded(child: _editor(context, role, automations)),
            ],
          ),
        ],
      ),
    );
  }

  Widget _editor(
    BuildContext context,
    BridgeRole role,
    List<BridgeWorkspaceAutomation> automations,
  ) {
    final ws = context.ws;
    final fixed = role.template == BridgeTemplate.canManage;
    final grants = role.setup.grants;
    final assigned = [
      for (final automation in automations)
        if (automation.role == role.id) automation,
    ];
    final others = [
      for (final automation in automations)
        if (automation.role != role.id) automation,
    ];
    void setGrants(List<BridgeGrant> grants) =>
        _write(role, BridgeRoleSetup(name: role.setup.name, grants: grants));
    final muted = TextStyle(color: ws.textMuted);
    Widget label(String text) =>
        SizedBox(width: 110, child: Text(text, style: muted));
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space4,
      children: [
        Row(
          children: [
            label('Name'),
            Expanded(
              child: Focus(
                onFocusChange: (focused) {
                  if (!focused) _rename(role, _name.text);
                },
                child: TextField(
                  key: const Key('role name'),
                  controller: _name,
                  enabled: !fixed,
                  onSubmitted: (name) => _rename(role, name),
                  decoration: InputDecoration.collapsed(
                    hintText: untitled,
                    hintStyle: TextStyle(color: ws.textFaint),
                  ),
                ),
              ),
            ),
            if (role.template == null)
              MoreMenu(
                label: 'Role actions',
                items: [
                  (
                    LucideIcons.trash,
                    'Delete Role',
                    () => unawaited(
                      _state
                          .apply(
                            mutation(
                              BridgeMutationKind.removeRole,
                              role: role.id,
                            ),
                          )
                          .then<void>((_) {}, onError: (Object _) {}),
                    ),
                  ),
                ],
              ),
          ],
        ),
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            label('Assigned to'),
            Expanded(
              child: Wrap(
                spacing: WsMetrics.space2,
                runSpacing: WsMetrics.space2,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  for (final automation in assigned)
                    Chip(
                      avatar: Icon(LucideIcons.zap, size: 14, color: ws.accent),
                      label: Text(
                        automation.setup.name.isEmpty
                            ? untitled
                            : automation.setup.name,
                      ),
                      deleteIcon: Icon(
                        LucideIcons.x,
                        size: 14,
                        color: ws.textMuted,
                      ),
                      deleteButtonTooltipMessage: 'Run as Can edit',
                      onDeleted: role.template == BridgeTemplate.canEdit
                          ? null
                          : () => _assign(automation, canEditRole(_state)),
                    ),
                  Token(
                    name: 'assign',
                    label: 'Assign…',
                    add: true,
                    choices: [
                      for (final automation in others)
                        (
                          automation.setup.name.isEmpty
                              ? untitled
                              : automation.setup.name,
                          false,
                          () => _assign(automation, role.id),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          ],
        ),
        _Grid(
          grants: grants,
          fixed: fixed,
          target: _target,
          protected: (grant) =>
              grant.kind == BridgeTargetKind.table &&
              _state.tables.value.any(
                (table) => table.id == grant.id && table.protected,
              ),
          onAllow: (index, capability, on) => setGrants([
            for (final (at, grant) in grants.indexed)
              at == index
                  ? BridgeGrant(
                      kind: grant.kind,
                      id: grant.id,
                      allows: allowing(grant.allows, capability, on),
                    )
                  : grant,
          ]),
          onRemove: (index) => setGrants([
            for (final (at, grant) in grants.indexed)
              if (at != index) grant,
          ]),
        ),
        if (!fixed)
          Align(
            alignment: Alignment.centerLeft,
            child: Token(
              name: 'add grant',
              label: 'Add an area or Table',
              add: true,
              choices: [
                if (!grants.any(
                  (grant) => grant.kind == BridgeTargetKind.workspace,
                ))
                  (
                    'All areas',
                    false,
                    () => setGrants([
                      ...grants,
                      const BridgeGrant(
                        kind: BridgeTargetKind.workspace,
                        allows: _read,
                      ),
                    ]),
                  ),
                for (final area in _state.areas.value)
                  if (!grants.any((grant) => grant.id == area.id))
                    (
                      area.name,
                      false,
                      () => setGrants([
                        ...grants,
                        BridgeGrant(
                          kind: BridgeTargetKind.area,
                          id: area.id,
                          allows: _read,
                        ),
                      ]),
                    ),
                for (final table in _state.tables.value)
                  if (!grants.any((grant) => grant.id == table.id))
                    (
                      table.name.isEmpty ? untitled : table.name,
                      false,
                      () => setGrants([
                        ...grants,
                        BridgeGrant(
                          kind: BridgeTargetKind.table,
                          id: table.id,
                          allows: _read,
                        ),
                      ]),
                    ),
              ],
            ),
          ),
        Text(
          fixed
              ? 'Can manage holds every capability, and the person who made '
                    'this Workspace always holds it.'
              : 'Anything not listed is denied. Propose and Approve matter '
                    'once agents act; Manage covers rights, protection and '
                    'deletion of what it lists.',
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
        ),
      ],
    );
  }
}

String canEditRole(WorkspaceState state) => state.roles.value
    .firstWhere((role) => role.template == BridgeTemplate.canEdit)
    .id;

class _RoleRow extends StatelessWidget {
  const _RoleRow({
    required this.role,
    required this.chosen,
    required this.assigned,
    required this.onTap,
  });

  final BridgeRole role;
  final bool chosen;
  final int assigned;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final detail = [
      if (role.template != null) 'template',
      if (role.template == BridgeTemplate.canManage) 'you',
      if (assigned > 0) '$assigned',
    ].join(' · ');
    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      child: Container(
        height: WsMetrics.rowNav + 4,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          color: chosen ? ws.selected : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                role.setup.name.isEmpty ? untitled : role.setup.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            Text(
              detail,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Grid extends StatelessWidget {
  const _Grid({
    required this.grants,
    required this.fixed,
    required this.target,
    required this.protected,
    required this.onAllow,
    required this.onRemove,
  });

  final List<BridgeGrant> grants;
  final bool fixed;
  final String Function(BridgeGrant) target;
  final bool Function(BridgeGrant) protected;
  final void Function(int index, String capability, bool on) onAllow;
  final ValueChanged<int> onRemove;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final head = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    Widget row(List<Widget> cells) => Container(
      height: WsMetrics.rowTable,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        children: [
          Expanded(flex: 3, child: cells.first),
          for (final cell in cells.skip(1).take(capabilities.length))
            Expanded(child: Center(child: cell)),
          SizedBox(width: 32, child: cells.length > 6 ? cells.last : null),
        ],
      ),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row([
          Text('Allowed on', style: head),
          for (final capability in capabilities)
            FittedBox(
              fit: BoxFit.scaleDown,
              child: Text(capability, style: head),
            ),
        ]),
        for (final (index, grant) in grants.indexed)
          row([
            Row(
              children: [
                Icon(
                  grant.kind == BridgeTargetKind.table
                      ? LucideIcons.table
                      : LucideIcons.folder,
                  size: 14,
                  color: ws.textFaint,
                ),
                const SizedBox(width: WsMetrics.space2),
                Flexible(
                  child: Text(
                    target(grant),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                if (protected(grant)) ...[
                  const SizedBox(width: WsMetrics.space2),
                  const ProtectedMark(),
                ],
              ],
            ),
            for (final capability in capabilities)
              Checkbox(
                key: Key('allow $capability $index'),
                value: allowed(grant.allows, capability),
                onChanged: fixed
                    ? null
                    : (on) => onAllow(index, capability, on ?? false),
              ),
            if (!fixed)
              IconButton(
                key: Key('remove grant $index'),
                tooltip: 'Remove',
                visualDensity: VisualDensity.compact,
                icon: Icon(LucideIcons.x, size: 14, color: ws.textFaint),
                onPressed: () => onRemove(index),
              ),
          ]),
      ],
    );
  }
}

class ProtectedMark extends StatelessWidget {
  const ProtectedMark({super.key});

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Row(
      mainAxisSize: MainAxisSize.min,
      spacing: WsMetrics.space1,
      children: [
        Icon(LucideIcons.lock, size: 12, color: ws.textMuted),
        Text(
          'Protected',
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted),
        ),
      ],
    );
  }
}

Future<void> protectTable(
  BuildContext context,
  WorkspaceState state,
  String table,
  String name,
) async {
  final Map<String, BridgeAllows> allows;
  try {
    allows = await state.allowsOn(table);
  } on Object catch (error) {
    state.readFailed(error);
    return;
  }
  if (!context.mounted) return;
  final kept = await showDialog<List<String>>(
    context: context,
    builder: (_) => _ProtectDialog(
      name: name.isEmpty ? untitled : name,
      roles: state.roles.value,
      allows: allows,
    ),
  );
  if (kept == null) return;
  await state
      .apply(
        mutation(BridgeMutationKind.protectTable, block: table, roles: kept),
      )
      .then<void>((_) {}, onError: (Object _) {});
}

class _ProtectDialog extends StatefulWidget {
  const _ProtectDialog({
    required this.name,
    required this.roles,
    required this.allows,
  });

  final String name;
  final List<BridgeRole> roles;
  final Map<String, BridgeAllows> allows;

  @override
  State<_ProtectDialog> createState() => _ProtectDialogState();
}

class _ProtectDialogState extends State<_ProtectDialog> {
  final _kept = <String>{};

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    return ActionDialog(
      title: 'Protect “${widget.name}”?',
      actions: [
        PrimaryButton(
          label: 'Protect',
          onPressed: () => Navigator.of(context).pop([
            for (final role in widget.roles)
              if (_kept.contains(role.id)) role.id,
          ]),
        ),
      ],
      children: [
        Text(
          'It stops inheriting rights from its area. Only the Roles chosen '
          'here may change it.',
          style: TextStyle(color: ws.textMuted),
        ),
        Column(
          children: [
            for (final role in widget.roles)
              Row(
                children: [
                  Checkbox(
                    key: Key('keep ${role.id}'),
                    value:
                        role.template == BridgeTemplate.canManage ||
                        _kept.contains(role.id),
                    onChanged: role.template == BridgeTemplate.canManage
                        ? null
                        : (on) => setState(
                            () => on ?? false
                                ? _kept.add(role.id)
                                : _kept.remove(role.id),
                          ),
                  ),
                  Expanded(child: Text(role.setup.name)),
                  Text(switch (role) {
                    BridgeRole(template: BridgeTemplate.canManage) => 'always',
                    _ when _kept.contains(role.id) => allowsText(
                      widget.allows[role.id] ?? _read,
                    ),
                    _ when widget.allows[role.id]?.apply ?? false =>
                      'loses Apply here',
                    _ => allowsText(widget.allows[role.id] ?? _read),
                  }, style: faint),
                ],
              ),
          ],
        ),
        Text(
          'Reading still follows the area, and every device in this '
          'Workspace holds its content. To keep it off other people\'s '
          'devices, put it in a separate Workspace.',
          style: TextStyle(color: ws.textMuted),
        ),
      ],
    );
  }
}

class RunsAs extends StatefulWidget {
  const RunsAs({super.key, required this.state, required this.automation});

  final WorkspaceState state;
  final BridgeWorkspaceAutomation automation;

  @override
  State<RunsAs> createState() => _RunsAsState();
}

class _RunsAsState extends State<RunsAs> {
  final _allows = <String, Map<String, BridgeAllows>>{};
  int _reads = 0;

  @override
  void initState() {
    super.initState();
    widget.state.roles.addListener(_read);
    widget.state.tables.addListener(_read);
    _read();
  }

  @override
  void didUpdateWidget(RunsAs old) {
    super.didUpdateWidget(old);
    if (old.automation != widget.automation) _read();
  }

  @override
  void dispose() {
    widget.state.roles.removeListener(_read);
    widget.state.tables.removeListener(_read);
    super.dispose();
  }

  List<(String, String)> get _needs {
    final setup = widget.automation.setup;
    final table = setup.trigger?.table;
    if (table == null) return const [];
    return [
      ('Read', table),
      for (final step in setup.steps)
        if (step.kind == BridgeStepKind.set_)
          ('Apply', table)
        else if (step.kind == BridgeStepKind.addRow && step.table != null)
          ('Apply', step.table!),
    ];
  }

  void _read() {
    final read = ++_reads;
    unawaited(() async {
      final read_ = <String, Map<String, BridgeAllows>>{};
      try {
        for (final table in {for (final (_, table) in _needs) table}) {
          read_[table] = await widget.state.allowsOn(table);
        }
      } on Object {
        return;
      }
      if (!mounted || read != _reads) return;
      setState(
        () => _allows
          ..clear()
          ..addAll(read_),
      );
    }());
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final role = widget.automation.role;
    String? missing;
    for (final (capability, table) in _needs) {
      final allows = _allows[table]?[role];
      if (allows != null && !allowed(allows, capability)) {
        final name = widget.state.tables.value
            .where((known) => known.id == table)
            .firstOrNull
            ?.name;
        missing = 'allows no $capability on ${name ?? 'a deleted Table'}';
        break;
      }
    }
    final known = _needs.isNotEmpty && _allows.isNotEmpty;
    return Row(
      spacing: WsMetrics.space3,
      children: [
        Token(
          name: 'runs as',
          label: widget.state.roleName(role),
          choices: [
            for (final listed in widget.state.roles.value)
              (
                listed.setup.name,
                listed.id == role,
                () => unawaited(
                  widget.state
                      .apply(
                        mutation(
                          BridgeMutationKind.setAutomationRole,
                          automation: widget.automation.id,
                          role: listed.id,
                        ),
                      )
                      .then<void>((_) {}, onError: (Object _) {}),
                ),
              ),
          ],
        ),
        if (known)
          Row(
            mainAxisSize: MainAxisSize.min,
            spacing: WsMetrics.space1,
            children: [
              Icon(
                missing == null ? LucideIcons.check : LucideIcons.x,
                size: 14,
                color: missing == null ? ws.success : ws.danger,
              ),
              Text(
                missing ?? 'allows every step',
                style: TextStyle(
                  color: missing == null ? ws.success : ws.danger,
                ),
              ),
            ],
          ),
      ],
    );
  }
}
