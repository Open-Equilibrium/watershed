import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/automation.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_cells.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

const buttonFields = [
  ...expenseFields,
  BridgeField(
    id: 'flag',
    name: 'Flag',
    kind: BridgeFieldKind.button,
    options: [],
    formula: '',
  ),
];

BridgeAutomationSetup flagLarge(String table) => BridgeAutomationSetup(
  name: 'Flag large expenses',
  trigger: BridgeTrigger(
    kind: BridgeTriggerKind.fieldChanged,
    table: table,
    field: 'amount',
  ),
  conditions: [
    BridgeFilter(
      field: 'amount',
      matches: BridgeMatch.above,
      value: cell(BridgeCellKind.number, number: 500),
    ),
  ],
  steps: const [
    BridgeStep(
      kind: BridgeStepKind.notify,
      cells: [],
      message: 'Large expense: {name}',
    ),
  ],
);

Future<String> showBudget(
  WidgetTester tester,
  PageKit kit, {
  List<BridgeRow> rows = const [],
}) async {
  await kit.show(tester, [TablePart(rows, fields: buttonFields)]);
  return kit.fake.databases.keys.single;
}

Future<void> openAutomations(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('sidebar automations')));
  await tester.pumpAndSettle();
}

Future<void> choose(WidgetTester tester, String token, String choice) async {
  await tester.tap(find.byKey(Key('token $token')));
  await tester.pumpAndSettle();
  await tester.tap(find.text(choice).last);
  await tester.pumpAndSettle();
}

BridgeAutomationSetup lastSetup(PageKit kit) =>
    kit.applied(BridgeMutationKind.setAutomation).last.automationSetup!;

void main() {
  testWidgets('the Automations dialog lists each one with its trigger, owner '
      'and status, and the sidebar counts those that need attention', (
    tester,
  ) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    kit.fake.automations
      ..add(
        BridgeWorkspaceAutomation(
          id: 'a1',
          setup: flagLarge(table),
          on_: true,
          owner: table,
        ),
      )
      ..add(
        const BridgeWorkspaceAutomation(
          id: 'a2',
          setup: BridgeAutomationSetup(
            name: 'Monthly summary',
            conditions: [],
            steps: [],
          ),
          on_: false,
          attention: BridgeAttention(
            kind: BridgeAttentionKind.tableDeleted,
            table: 'Bills',
          ),
        ),
      );
    await kit.workspace.apply(
      mutation(BridgeMutationKind.setAutomationOn, automation: 'a1', on: true),
    );
    await tester.pumpAndSettle();
    expect(
      find.descendant(
        of: find.byKey(const Key('sidebar automations')),
        matching: find.text('1'),
      ),
      findsOneWidget,
    );

    await openAutomations(tester);
    Finder listed(String text) => find.descendant(
      of: find.byKey(const Key('automations')),
      matching: find.text(text),
    );
    for (final shown in [
      'Flag large expenses',
      '“Amount” changes',
      'Expenses',
      'On',
      'Monthly summary',
      'Workspace',
      'Needs attention',
    ]) {
      expect(listed(shown), findsOneWidget, reason: shown);
    }

    await tester.tap(find.text('Monthly summary'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Turned off: its Database “Bills” was deleted'),
      findsOneWidget,
    );
    final on = tester.widget<Switch>(find.byKey(const Key('automation on')));
    expect(on.onChanged, isNull);
  });

  testWidgets('a new Automation is built as a sentence: when, if and then', (
    tester,
  ) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    await openAutomations(tester);
    await tester.tap(find.text('New Automation'));
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.createAutomation), hasLength(1));

    await choose(tester, 'when', 'a field changes');
    await choose(tester, 'table', 'Expenses');
    await choose(tester, 'field', 'Amount');
    expect(
      lastSetup(kit).trigger,
      BridgeTrigger(
        kind: BridgeTriggerKind.fieldChanged,
        table: table,
        field: 'amount',
      ),
    );

    await choose(tester, 'add condition', 'Amount');
    await choose(tester, 'test 0', 'is more than');
    await tester.enterText(find.byKey(const Key('value 0')), '500');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    final condition = lastSetup(kit).conditions.single;
    expect(condition.matches, BridgeMatch.above);
    expect(condition.value?.number, 500);

    await choose(tester, 'condition field 0', 'Paid');
    final paid = lastSetup(kit).conditions.single;
    expect(
      (paid.field, paid.matches, paid.value?.checked),
      ('paid', BridgeMatch.is_, true),
    );
    expect(find.byKey(const Key('token test 0')), findsOneWidget);
    expect(find.text('is checked'), findsOneWidget);
    await choose(tester, 'condition field 0', 'Amount');
    await choose(tester, 'test 0', 'is more than');
    await tester.enterText(find.byKey(const Key('value 0')), '500');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();

    await choose(tester, 'add action', 'show a notification');
    await tester.enterText(
      find.byKey(const Key('message 0')),
      'Large expense: {Name}',
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(lastSetup(kit).steps.single.message, 'Large expense: {name}');

    await choose(tester, 'add action', 'set a field');
    await choose(tester, 'set field 1', 'Category');
    await choose(tester, 'set value 1', 'Home');
    final set = lastSetup(kit).steps.last;
    expect(
      (set.kind, set.field, set.value?.text),
      (BridgeStepKind.set_, 'category', 'home'),
    );

    await tester.enterText(
      find.byKey(const Key('automation name')),
      'Flag large expenses',
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(lastSetup(kit).name, 'Flag large expenses');
  });

  testWidgets('a button cell runs its Automations and their notices and '
      'failures show as toasts; Test and the switch reach the core', (
    tester,
  ) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit, rows: expenses);
    kit.fake.automations.add(
      BridgeWorkspaceAutomation(
        id: 'a1',
        setup: flagLarge(table),
        on_: true,
        owner: table,
      ),
    );
    await kit.workspace.apply(
      mutation(BridgeMutationKind.setAutomationOn, automation: 'a1', on: true),
    );
    await tester.pumpAndSettle();

    kit.fake.ranNext = const [
      BridgeRan(
        automation: 'a1',
        name: 'Flag large expenses',
        notices: ['Large expense: Groceries'],
      ),
    ];
    await tester.tap(kit.inTable(find.byType(CellButton)).first);
    await tester.pump();
    expect(kit.fake.pressed.single, (table, 'r1', 'flag'));
    expect(find.text('Large expense: Groceries'), findsOneWidget);
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    expect(find.text('Large expense: Groceries'), findsNothing);

    await openAutomations(tester);
    await tester.tap(find.text('Flag large expenses'));
    await tester.pumpAndSettle();
    kit.fake.ranNext = const [
      BridgeRan(
        automation: 'a1',
        name: 'Flag large expenses',
        notices: [],
        failure: 'not found',
      ),
    ];
    await tester.tap(find.text('Test'));
    await tester.pump();
    expect(kit.fake.tested, ['a1']);
    expect(
      find.text('“Flag large expenses” did not run: not found'),
      findsOneWidget,
    );

    await tester.tap(find.byKey(const Key('automation on')));
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.setAutomationOn).last.on_, isFalse);
    await tester.pump(const Duration(seconds: 6));
  });

  testWidgets('a refused new Automation shows why and stays in the list', (
    tester,
  ) async {
    final kit = PageKit();
    await showBudget(tester, kit);
    await openAutomations(tester);
    kit.fake.refuseKinds = {BridgeMutationKind.createAutomation};
    await tester.tap(find.text('New Automation'));
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Liquid did not save this change'),
      findsOneWidget,
    );
    expect(find.text('New Automation'), findsOneWidget);
  });

  testWidgets('moving the trigger to another Table keeps a notification, '
      'its fields named as text', (tester) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.createTable,
        area: kit.workspace.areas.value.first.id,
        text: 'Bills',
      ),
    );
    kit.fake.automations.add(
      BridgeWorkspaceAutomation(
        id: 'a1',
        setup: BridgeAutomationSetup(
          name: 'Log',
          trigger: BridgeTrigger(
            kind: BridgeTriggerKind.rowAdded,
            table: table,
          ),
          conditions: const [],
          steps: const [
            BridgeStep(
              kind: BridgeStepKind.notify,
              cells: [],
              message: '{name} added',
            ),
          ],
        ),
        on_: true,
        owner: table,
      ),
    );
    await kit.workspace.apply(
      mutation(BridgeMutationKind.setAutomationOn, automation: 'a1', on: true),
    );
    await openAutomations(tester);
    await tester.tap(find.text('Log'));
    await tester.pumpAndSettle();
    await choose(tester, 'table', 'Bills');
    expect(lastSetup(kit).steps.single.message, '{Name} added');
  });
}
