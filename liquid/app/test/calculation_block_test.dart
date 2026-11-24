import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/calculation.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/calculation_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

const amount = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

void expectOne(Finder finder) => expect(finder, findsOneWidget);
void expectNone(Finder finder) => expect(finder, findsNothing);

void main() {
  final kit = PageKit();

  BridgeLineSetup setup(
    String label,
    String formula, [
    BridgeLineStyle style = BridgeLineStyle.plain,
  ]) => BridgeLineSetup(label: label, formula: formula, style: style);

  String blockOf(int index) => kit.fake.views[kit.page]![index].block;
  List<BridgeLine> linesOf(int index) => kit.fake.calculations[blockOf(index)]!;

  Finder inBlock(Finder matching) =>
      find.descendant(of: find.byType(CalculationBlock), matching: matching);
  Finder field(String hint) => inBlock(
    find.byWidgetPredicate(
      (widget) => widget is TextField && widget.decoration?.hintText == hint,
    ),
  );
  Finder value() => field('A value, or = and a formula');
  String valueText(WidgetTester tester) =>
      tester.widget<TextField>(value()).controller!.text;

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> type(WidgetTester tester, String text) async {
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('/calc adds a Calculation Block whose line takes a label and a '
      'value', (tester) async {
    await kit.show(tester, [const TextPart('Trip'), const TextPart('')]);
    await tester.tap(block(1));
    await tester.pump();
    await type(tester, '/');
    await type(tester, '/calc');
    expectOne(find.byKey(const ValueKey('choice Calculation')));
    await key(tester, LogicalKeyboardKey.enter);

    expect(kit.fake.views[kit.page]!.map((view) => view.kind), [
      BridgeBlockKind.text,
      BridgeBlockKind.calculation,
      BridgeBlockKind.text,
    ]);
    await tester.tap(field('Label'));
    await tester.pump();
    await type(tester, 'Flights');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(linesOf(1).single.label, 'Flights');
    expectOne(value());
    await type(tester, '420');
    await key(tester, LogicalKeyboardKey.enter);

    expect(linesOf(1).single.formula, '420');
    expectOne(inBlock(find.text('420')));
    expectNone(value());

    await tap(tester, inBlock(find.text('Add a line')));
    expect(linesOf(1).length, 2);
    final label = tester.widget<TextField>(field('Label').last);
    expect(label.focusNode!.hasFocus, isTrue, reason: 'the new line is ready');
  });

  testWidgets('= suggests lines, fields and functions; a chosen reference is '
      'a chip stored by its identity', (tester) async {
    await kit.show(tester, [
      const TablePart(
        [],
        fields: [
          BridgeField(
            id: amount,
            name: 'Amount',
            kind: BridgeFieldKind.number,
            options: [],
            formula: '',
          ),
        ],
      ),
      CalculationPart([setup('Budget', '2000'), setup('Left', '')]),
    ]);
    final table = blockOf(0);
    final column = '{$table.$amount}';
    kit.fake.referenceNames[column] = 'Expenses.Amount';
    await tap(tester, find.byKey(const ValueKey('value line 1')));
    await type(tester, '=Bud');
    expectOne(inBlock(find.text('Line')));
    await key(tester, LogicalKeyboardKey.tab);
    await type(tester, '${valueText(tester)} − SU');
    expectOne(inBlock(find.text('SUMIF')));
    await key(tester, LogicalKeyboardKey.enter);
    final typed = valueText(tester);
    expect(typed, '=￼ − SUM(');
    await type(tester, '${typed}Expenses.Am');
    await tap(tester, inBlock(find.text('Expenses.Amount')));
    await type(tester, '${valueText(tester)})');
    await key(tester, LogicalKeyboardKey.enter);

    final budget = linesOf(1).first.id;
    expect(
      linesOf(1)[1].formula,
      '{${kit.page}.${blockOf(1)}.$budget} − SUM($column)',
    );
  });

  testWidgets('a formula that cannot be read keeps its editor; the line menu '
      'restyles and removes a line', (tester) async {
    await kit.show(
      tester,
      before: () => kit.fake.computed['SUM(ABOVE)'] = '3650',
      [
        CalculationPart([
          setup('Income', '', BridgeLineStyle.heading),
          setup('Salary', '3200'),
          setup('Side project', '450'),
          setup('Total income', 'SUM(ABOVE)'),
        ], title: 'October budget'),
      ],
    );
    expectOne(inBlock(find.text('3650')));
    expectOne(find.text('October budget'));

    await tap(tester, find.byKey(const ValueKey('value line 2')));
    await type(tester, '=1 +');
    await key(tester, LogicalKeyboardKey.enter);
    expectOne(value());
    expect(linesOf(0)[2].formula, '450');
    await key(tester, LogicalKeyboardKey.escape);
    expectNone(value());

    final actions = find.descendant(
      of: find.byKey(const ValueKey('line 3')),
      matching: find.byKey(const Key('Line actions')),
    );
    await tap(tester, actions);
    await tap(tester, find.text('Subtotal'));
    expect(linesOf(0)[3].style, BridgeLineStyle.subtotal);
    expect(linesOf(0)[3].formula, 'SUM(ABOVE)');

    await tap(tester, actions);
    await tap(tester, find.text('Remove line'));
    expect(linesOf(0).map((line) => line.label), [
      'Income',
      'Salary',
      'Side project',
    ]);
  });

  testWidgets('a Calculation reads again when a Table its formula reads '
      'changes', (tester) async {
    await kit.show(tester, [
      TablePart(expenses),
      CalculationPart([setup('Spent', '')]),
    ]);
    final table = blockOf(0);
    final formula = 'SUM({$table.amount})';
    kit.fake.computed[formula] = '180.2';
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setLine,
        page: kit.page,
        block: blockOf(1),
        line: 'line 0',
        lineSetup: setup('Spent', formula),
      ),
    );
    await tester.pumpAndSettle();
    expectOne(inBlock(find.text('180.2')));

    kit.fake.computed[formula] = '200.2';
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setCell,
        block: table,
        row: 'r1',
        field: 'amount',
        cell: cell(BridgeCellKind.number, number: 104.2),
      ),
    );
    await tester.pumpAndSettle();
    expectOne(inBlock(find.text('200.2')));
  });

  testWidgets('leaving an unchanged literal line keeps it as written', (
    tester,
  ) async {
    await kit.show(tester, [
      CalculationPart([setup('Note', '"=1+2"')]),
    ]);
    await tap(tester, find.byKey(const ValueKey('value line 0')));
    expect(valueText(tester), '=1+2');
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    expect(linesOf(0).single.formula, '"=1+2"');
  });

  testWidgets('a label and a restyle made before the lines reload both '
      'persist', (tester) async {
    await kit.show(tester, [
      CalculationPart([setup('Old', '')]),
    ]);
    final line = find.byKey(const ValueKey('line 0'));
    final label = find.descendant(of: line, matching: find.byType(TextField));
    kit.fake.deferredKinds = {BridgeQueryKind.calculation};
    kit.fake.deferred = [];
    await tap(tester, label);
    await type(tester, 'New');
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    await tap(
      tester,
      find.descendant(
        of: line,
        matching: find.byKey(const Key('Line actions')),
      ),
    );
    await tap(tester, find.text('Total'));
    kit.fake.deferredKinds = {};
    for (final release in [...kit.fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(linesOf(0).single.label, 'New');
    expect(linesOf(0).single.style, BridgeLineStyle.total);
  });

  testWidgets('a label typed and not submitted is saved when its block is '
      'left', (tester) async {
    await kit.show(tester, [
      CalculationPart([setup('Old', '')]),
    ]);
    final block = blockOf(0);
    await tap(
      tester,
      find.descendant(
        of: find.byKey(const ValueKey('line 0')),
        matching: find.byType(TextField),
      ),
    );
    await type(tester, 'New');
    kit.workspace.open.value = null;
    await tester.pumpAndSettle();
    expect(kit.fake.calculations[block]!.single.label, 'New');
  });

  testWidgets('enter then leaving a changed line saves it once', (
    tester,
  ) async {
    await kit.show(tester, [
      CalculationPart([setup('Note', '')]),
    ]);
    kit.fake.gate = Completer<void>();
    kit.fake.gatedKinds = {BridgeMutationKind.setLine};
    await tap(tester, find.byKey(const ValueKey('value line 0')));
    await type(tester, '420');
    await key(tester, LogicalKeyboardKey.enter);
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(kit.applied(BridgeMutationKind.setLine), hasLength(1));
  });
}
