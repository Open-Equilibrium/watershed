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
    expect(find.byKey(const ValueKey('choice Calculation')), findsOneWidget);
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
    expect(field('A value, or = and a formula'), findsOneWidget);
    await type(tester, '420');
    await key(tester, LogicalKeyboardKey.enter);

    expect(linesOf(1).single.formula, '420');
    expect(inBlock(find.text('420')), findsOneWidget);
    expect(field('A value, or = and a formula'), findsNothing);

    await tester.tap(inBlock(find.text('Add a line')));
    await tester.pumpAndSettle();
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
    await tester.tap(find.byKey(const ValueKey('value line 1')));
    await tester.pumpAndSettle();
    await type(tester, '=Bud');
    expect(inBlock(find.text('Line')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.tab);
    await type(
      tester,
      '${tester.widget<TextField>(field('A value, or = and a formula')).controller!.text} − SU',
    );
    expect(inBlock(find.text('SUMIF')), findsOneWidget);
    await key(tester, LogicalKeyboardKey.enter);
    final typed = tester
        .widget<TextField>(field('A value, or = and a formula'))
        .controller!
        .text;
    expect(typed, '=￼ − SUM(');
    await type(tester, '${typed}Expenses.Am');
    await tester.tap(inBlock(find.text('Expenses.Amount')));
    await tester.pumpAndSettle();
    final chipped = tester
        .widget<TextField>(field('A value, or = and a formula'))
        .controller!
        .text;
    await type(tester, '$chipped)');
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
    expect(inBlock(find.text('3650')), findsOneWidget);
    expect(find.text('October budget'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('value line 2')));
    await tester.pumpAndSettle();
    await type(tester, '=1 +');
    await key(tester, LogicalKeyboardKey.enter);
    expect(field('A value, or = and a formula'), findsOneWidget);
    expect(linesOf(0)[2].formula, '450');
    await key(tester, LogicalKeyboardKey.escape);
    expect(field('A value, or = and a formula'), findsNothing);

    final actions = find.descendant(
      of: find.byKey(const ValueKey('line 3')),
      matching: find.byKey(const Key('Line actions')),
    );
    await tester.tap(actions);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Subtotal'));
    await tester.pumpAndSettle();
    expect(linesOf(0)[3].style, BridgeLineStyle.subtotal);
    expect(linesOf(0)[3].formula, 'SUM(ABOVE)');

    await tester.tap(actions);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove line'));
    await tester.pumpAndSettle();
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
    expect(inBlock(find.text('180.2')), findsOneWidget);

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
    expect(inBlock(find.text('200.2')), findsOneWidget);
  });
}
