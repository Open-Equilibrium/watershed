part of 'fake_workspace.dart';

extension _FakeRoles on FakeWorkspace {
  BridgeApplied? _rolesMutation(BridgeMutation mutation) {
    switch (mutation.kind) {
      case BridgeMutationKind.createAutomation:
        final id = _id();
        final setup = mutation.automationSetup!;
        automations.add(
          BridgeWorkspaceAutomation(
            id: id,
            setup: setup,
            on_: true,
            owner: setup.trigger?.table,
            role: canEdit,
          ),
        );
        _automationsChanged();
        return _done(automation: id);
      case BridgeMutationKind.setAutomation:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: mutation.automationSetup!,
            on_: kept.on_,
            last: kept.last,
            owner: mutation.automationSetup!.trigger?.table,
            role: kept.role,
          ),
        );
        return _done();
      case BridgeMutationKind.setAutomationOn:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: kept.setup,
            on_: mutation.on_,
            attention: kept.attention,
            last: kept.last,
            owner: kept.owner,
            role: kept.role,
          ),
        );
        return _done();
      case BridgeMutationKind.setAutomationRole:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: kept.setup,
            on_: kept.on_,
            attention: kept.attention,
            last: kept.last,
            owner: kept.owner,
            role: mutation.role!,
          ),
        );
        return _done();
      case BridgeMutationKind.createRole:
        final id = _id();
        roles.add(BridgeRole(id: id, setup: mutation.roleSetup!));
        _rolesChanged();
        return _done(role: id);
      case BridgeMutationKind.setRole:
        final at = roles.indexWhere((role) => role.id == mutation.role);
        roles[at] = BridgeRole(
          id: roles[at].id,
          setup: mutation.roleSetup!,
          template: roles[at].template,
        );
        _rolesChanged();
        return _done();
      case BridgeMutationKind.removeRole:
        roles.removeWhere((role) => role.id == mutation.role);
        _rolesChanged();
        return _done();
      case BridgeMutationKind.removeAutomation:
        automations.removeWhere((kept) => kept.id == mutation.automation);
        _automationsChanged();
        return _done();
      default:
        return null;
    }
  }

  void _rolesChanged() =>
      _changed(const BridgeResource(kind: BridgeResourceKind.roles));

  void _automationsChanged() =>
      _changed(const BridgeResource(kind: BridgeResourceKind.automations));

  void _replaceAutomation(
    String? id,
    BridgeWorkspaceAutomation Function(BridgeWorkspaceAutomation) change,
  ) {
    final at = automations.indexWhere((kept) => kept.id == id);
    if (at < 0) throw 'not found';
    automations[at] = change(automations[at]);
    _automationsChanged();
  }
}
