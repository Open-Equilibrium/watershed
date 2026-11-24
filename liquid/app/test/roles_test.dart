import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/automation.dart';
import 'package:liquid/src/rust/api/role.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/theme/tokens.g.dart';
import 'package:liquid/src/workspace/roles.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'automations_test.dart' show buttonFields, flagLarge;
import 'fake_workspace.dart';
import 'page_kit.dart';
import 'focus_kit.dart';

Future<String> showBudget(WidgetTester tester, PageKit kit) async {
  await kit.show(tester, [TablePart(expenses, fields: buttonFields)]);
  return kit.fake.databases.keys.single;
}

BridgeRoleSetup lastRole(PageKit kit) =>
    kit.applied(BridgeMutationKind.setRole).last.roleSetup!;

void main() {
  testWidgets('Settings › Roles lists the templates and builds a Role from '
      'areas and Tables, one capability at a time', (tester) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    await tap(tester, find.text('Settings'));
    await tap(tester, find.text('Roles'));
    for (final name in ['Can manage', 'Can edit', 'Can view']) {
      expect(find.text(name), findsWidgets, reason: name);
    }
    await tap(tester, find.text('Can manage').first);
    final name = tester.widget<TextField>(find.byKey(const Key('role name')));
    expect(name.enabled, isFalse);

    await tap(tester, find.text('New Role'));
    expect(kit.applied(BridgeMutationKind.createRole), hasLength(1));
    await tester.enterText(find.byKey(const Key('role name')), 'Budget');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(lastRole(kit).name, 'Budget');

    await tap(tester, find.byKey(const Key('token add grant')));
    await tap(tester, find.text('Expenses').last);
    final grant = lastRole(kit).grants.single;
    expect(
      (grant.kind, grant.id, grant.allows.read),
      (BridgeTargetKind.table, table, true),
    );
    await tap(tester, find.byKey(const Key('allow Apply 0')));
    expect(lastRole(kit).grants.single.allows.apply, isTrue);
    await tap(tester, find.byKey(const Key('remove grant 0')));
    expect(lastRole(kit).grants, isEmpty);

    await tap(tester, find.byTooltip('Role actions'));
    await tap(tester, find.text('Delete Role'));
    expect(kit.applied(BridgeMutationKind.removeRole), hasLength(1));
  });

  testWidgets('the Roles grid keeps each capability header on one line at '
      'the minimum width', (tester) async {
    final kit = PageKit();
    await showBudget(tester, kit);
    await tap(tester, find.text('Settings'));
    await tap(tester, find.text('Roles'));
    await tester.binding.setSurfaceSize(const Size(1050, 640));
    await tester.pumpAndSettle();
    for (final capability in capabilities) {
      expect(
        tester.getSize(find.text(capability).first).height,
        lessThan(WsMetrics.fsSmall * 2),
        reason: capability,
      );
    }
  });

  testWidgets('a Table is protected for the Roles chosen for it and shows a '
      'lock until it stops', (tester) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    kit.workspace.openTable(table);
    await tester.pumpAndSettle();
    await tap(tester, find.byTooltip('Table actions'));
    await tap(tester, find.text('Protect…'));
    expect(find.text('Protect “Expenses”?'), findsOneWidget);
    expect(find.text('always'), findsOneWidget);
    await tap(tester, find.byKey(const Key('keep $canEdit')));
    await tap(tester, find.text('Protect'));
    final protect = kit.applied(BridgeMutationKind.protectTable).single;
    expect(protect.block, table);
    expect(protect.roles, [canEdit]);
    expect(find.text('Protected'), findsWidgets);

    await tap(tester, find.byTooltip('Table actions'));
    await tap(tester, find.text('Stop protecting'));
    expect(kit.applied(BridgeMutationKind.unprotectTable), hasLength(1));
    expect(find.text('Protected'), findsNothing);
  });

  testWidgets('two grant edits before the Roles list refreshes both stay', (
    tester,
  ) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    await tap(tester, find.text('Settings'));
    await tap(tester, find.text('Roles'));
    await tap(tester, find.text('New Role'));
    await tap(tester, find.byKey(const Key('token add grant')));
    await tap(tester, find.text('Expenses').last);
    kit.fake.deferredKinds = {BridgeQueryKind.roles};
    kit.fake.deferred = [];
    await tap(tester, find.byKey(const Key('allow Apply 0')));
    await tap(tester, find.byKey(const Key('allow Approve 0')));
    final grant = lastRole(kit).grants.single;
    expect(
      (grant.kind, grant.id, grant.allows.apply, grant.allows.approve),
      (BridgeTargetKind.table, table, true, true),
    );
    for (final release in [...kit.fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
  });

  testWidgets('an incoming rename of the open Role replaces the name in the '
      'field', (tester) async {
    final kit = PageKit();
    await showBudget(tester, kit);
    await tap(tester, find.text('Settings'));
    await tap(tester, find.text('Roles'));
    await tap(tester, find.text('New Role'));
    final role = kit.fake.roles.last.id;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setRole,
        role: role,
        roleSetup: const BridgeRoleSetup(name: 'Renamed elsewhere', grants: []),
      ),
    );
    await tester.pumpAndSettle();
    final name = tester.widget<TextField>(find.byKey(const Key('role name')));
    expect(name.controller!.text, 'Renamed elsewhere');
  });

  testWidgets('an incoming rename leaves a name being typed alone', (
    tester,
  ) async {
    final kit = PageKit();
    await showBudget(tester, kit);
    await tap(tester, find.text('Settings'));
    await tap(tester, find.text('Roles'));
    await tap(tester, find.text('New Role'));
    final role = kit.fake.roles.last.id;
    await tester.enterText(find.byKey(const Key('role name')), 'Typed');
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setRole,
        role: role,
        roleSetup: const BridgeRoleSetup(name: 'Renamed elsewhere', grants: []),
      ),
    );
    await tester.pumpAndSettle();
    final name = tester.widget<TextField>(find.byKey(const Key('role name')));
    expect(name.controller!.text, 'Typed');
  });

  testWidgets('an Automation runs as the Role chosen for it and says what '
      'its Role does not allow', (tester) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    kit.fake.automations.add(
      BridgeWorkspaceAutomation(
        id: 'a1',
        setup: flagLarge(table),
        on_: true,
        owner: table,
        role: canEdit,
      ),
    );
    kit.fake.allowances[table] = {canView: nothing, canEdit: everything};
    await kit.workspace.apply(
      mutation(BridgeMutationKind.setAutomationOn, automation: 'a1', on: true),
    );
    await tap(tester, find.byKey(const Key('sidebar automations')));
    expect(find.text('Can edit'), findsOneWidget);
    await tap(tester, find.text('Flag large expenses'));
    expect(find.text('allows every step'), findsOneWidget);

    await tap(tester, find.byKey(const Key('token runs as')));
    await tap(tester, find.text('Can view').last);
    final assigned = kit.applied(BridgeMutationKind.setAutomationRole).single;
    expect((assigned.automation, assigned.role), ('a1', canView));
    expect(find.text('allows no Read on Expenses'), findsOneWidget);
  });
}
