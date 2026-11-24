import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/automation.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/table_cells.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';
import 'focus_kit.dart';

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

BridgeAutomationSetup flagLarge(
  String table, {
  String name = 'Flag large expenses',
  String field = 'amount',
  BridgeMatch matches = BridgeMatch.above,
  BridgeCell? value,
  List<BridgeStep> steps = const [
    BridgeStep(
      kind: BridgeStepKind.notify,
      cells: [],
      message: 'Large expense: {name}',
    ),
  ],
}) => BridgeAutomationSetup(
  name: name,
  trigger: BridgeTrigger(
    kind: BridgeTriggerKind.fieldChanged,
    table: table,
    field: field,
  ),
  conditions: [
    BridgeFilter(
      field: field,
      matches: matches,
      value: value ?? cell(BridgeCellKind.number, number: 500),
    ),
  ],
  steps: steps,
);

BridgeWorkspaceAutomation automationOf(
  String table,
  BridgeAutomationSetup setup,
) => BridgeWorkspaceAutomation(
  id: 'a1',
  setup: setup,
  on_: true,
  owner: table,
  role: canEdit,
);

Future<String> showBudget(
  WidgetTester tester,
  PageKit kit, {
  List<BridgeRow> rows = const [],
}) async {
  await kit.show(tester, [TablePart(rows, fields: buttonFields)]);
  return kit.fake.databases.keys.single;
}

Future<void> openAutomations(WidgetTester tester) =>
    tap(tester, find.byKey(const Key('sidebar automations')));

Future<void> choose(WidgetTester tester, String token, String choice) async {
  await tap(tester, find.byKey(Key('token $token')));
  await tap(tester, find.text(choice).last);
}

Future<void> turnOn(WidgetTester tester, PageKit kit) async {
  await kit.workspace.apply(
    mutation(BridgeMutationKind.setAutomationOn, automation: 'a1', on: true),
  );
  await tester.pumpAndSettle();
}

BridgeAutomationSetup lastSetup(PageKit kit) =>
    kit.applied(BridgeMutationKind.setAutomation).last.automationSetup!;

Future<void> rename(WidgetTester tester, String name) async {
  await tester.enterText(find.byKey(const Key('automation name')), name);
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await tester.pumpAndSettle();
}

Future<String> openEditor(
  WidgetTester tester,
  PageKit kit,
  BridgeAutomationSetup Function(String table) setup,
) async {
  final table = await showBudget(tester, kit);
  final automation = setup(table);
  kit.fake.automations.add(automationOf(table, automation));
  await turnOn(tester, kit);
  await openAutomations(tester);
  await tap(tester, find.text(automation.name));
  return table;
}

void main() {
  testWidgets('the Automations dialog lists each with its trigger, owner and '
      'status, and the sidebar counts those that need attention', (
    tester,
  ) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    kit.fake.automations
      ..add(automationOf(table, flagLarge(table)))
      ..add(
        const BridgeWorkspaceAutomation(
          id: 'a2',
          setup: BridgeAutomationSetup(
            name: 'Monthly summary',
            conditions: [],
            steps: [],
          ),
          on_: false,
          role: canEdit,
          attention: BridgeAttention(
            kind: BridgeAttentionKind.tableDeleted,
            table: 'Bills',
          ),
        ),
      );
    await turnOn(tester, kit);
    expectOne(
      find.descendant(
        of: find.byKey(const Key('sidebar automations')),
        matching: find.text('1'),
      ),
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

    await tap(tester, find.text('Monthly summary'));
    expectOne(find.textContaining('Turned off: its Table “Bills” was deleted'));
    await tap(tester, find.byKey(const Key('automation on')));
    expect(kit.applied(BridgeMutationKind.setAutomationOn).last.on_, isTrue);
  });

  testWidgets('a new Automation is built as a sentence: when, if and '
      'then', (tester) async {
    final kit = PageKit();
    final table = await showBudget(tester, kit);
    await openAutomations(tester);

    kit.fake.refuseKinds = {BridgeMutationKind.createAutomation};
    await tap(tester, find.text('New Automation'));
    expectOne(find.textContaining('Liquid did not save this change'));
    expectOne(find.text('New Automation'));

    kit.fake.refuseKinds = {};
    await tap(tester, find.text('New Automation'));
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
    expectOne(find.byKey(const Key('token test 0')));
    expectOne(find.text('is checked'));
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
    kit.fake.automations.add(automationOf(table, flagLarge(table)));
    await turnOn(tester, kit);

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
    expectOne(find.text('Large expense: Groceries'));
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    expectNone(find.text('Large expense: Groceries'));

    await openAutomations(tester);
    await tap(tester, find.text('Flag large expenses'));
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
    expectOne(find.text('“Flag large expenses” did not run: not found'));

    await tap(tester, find.byKey(const Key('automation on')));
    expect(kit.applied(BridgeMutationKind.setAutomationOn).last.on_, isFalse);
    await tester.pump(const Duration(seconds: 6));
  });

  testWidgets('a large number condition is shown without clamping to the '
      'integer range', (tester) async {
    final kit = PageKit();
    await openEditor(
      tester,
      kit,
      (table) => flagLarge(
        table,
        value: cell(BridgeCellKind.number, number: 1e20),
        steps: const [],
      ),
    );
    final shown = tester
        .widget<EditableText>(
          find.descendant(
            of: find.byKey(const Key('value 0')),
            matching: find.byType(EditableText),
          ),
        )
        .controller
        .text;
    expect(double.tryParse(shown), 1e20);
  });

  testWidgets('a date that does not exist is refused, not moved to the next '
      'day', (tester) async {
    final kit = PageKit();
    await openEditor(
      tester,
      kit,
      (table) => flagLarge(
        table,
        name: 'Due soon',
        field: 'date',
        matches: BridgeMatch.is_,
        value: cell(BridgeCellKind.date, text: '2026-11-01'),
        steps: const [],
      ),
    );
    await tester.enterText(find.byKey(const Key('value 0')), '2026-02-30');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.setAutomation), isEmpty);
    expectOne(find.text('Type a date as 2026-11-30.'));

    await tester.enterText(find.byKey(const Key('value 0')), '2026-02-28');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(lastSetup(kit).conditions.single.value?.text, '2026-02-28');
  });

  testWidgets('overlapping edits to an Automation are saved together', (
    tester,
  ) async {
    final kit = PageKit();
    await openEditor(tester, kit, flagLarge);
    kit.fake.gatedKinds = {BridgeMutationKind.setAutomation};
    kit.fake.gate = Completer<void>();
    await tester.enterText(find.byKey(const Key('value 0')), '600');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    await tester.enterText(
      find.byKey(const Key('automation name')),
      'Big expenses',
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(lastSetup(kit).name, 'Big expenses');
    expect(lastSetup(kit).conditions.single.value?.number, 600);
  });

  testWidgets('a rename Liquid did not save can be submitted again unchanged', (
    tester,
  ) async {
    final kit = PageKit();
    await openEditor(tester, kit, flagLarge);
    kit.fake.refuseKinds = {BridgeMutationKind.setAutomation};
    await rename(tester, 'Big expenses');
    expectOne(find.textContaining('Liquid did not save this change'));

    kit.fake.refuseKinds = {};
    await rename(tester, 'Big expenses');
    expect(lastSetup(kit).name, 'Big expenses');
  });

  testWidgets('a trigger field removed while the editor is open is replaced '
      'by a field added afterwards', (tester) async {
    final kit = PageKit();
    final table = await openEditor(tester, kit, flagLarge);
    await kit.workspace.apply(
      mutation(BridgeMutationKind.removeField, block: table, field: 'amount'),
    );
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.addField,
        block: table,
        text: 'Total',
        fieldKind: BridgeFieldKind.number,
      ),
    );
    await tester.pumpAndSettle();
    await choose(tester, 'condition field 0', 'Total');
    expect(
      lastSetup(kit).conditions.single.field,
      kit.fake.databases[table]!.fields
          .firstWhere((field) => field.name == 'Total')
          .id,
    );
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
      automationOf(
        table,
        BridgeAutomationSetup(
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
      ),
    );
    await turnOn(tester, kit);
    await openAutomations(tester);
    await tap(tester, find.text('Log'));
    await choose(tester, 'table', 'Bills');
    expect(lastSetup(kit).steps.single.message, '{Name} added');
  });
}
