import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/calculation.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/formula_field.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'page_kit.dart';

const amount = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const half = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

BridgeField fieldOf(
  String id,
  String name,
  BridgeFieldKind kind, [
  String formula = '',
]) => BridgeField(
  id: id,
  name: name,
  kind: kind,
  options: const [],
  formula: formula,
);

void main() {
  final kit = PageKit();
  final menu = find.byKey(const Key('field menu'));
  Finder inMenu(Finder finder) => find.descendant(of: menu, matching: finder);

  final table = TablePart(
    [
      BridgeRow(
        id: 'rent',
        cells: [
          BridgeFieldCell(
            field: 'name',
            value: cell(BridgeCellKind.text, text: 'Rent'),
          ),
          BridgeFieldCell(
            field: amount,
            value: cell(BridgeCellKind.number, number: 950.5),
          ),
        ],
        computed: const [
          BridgeComputed(
            field: half,
            value: '475.25',
            kind: BridgeValueKind.number,
            number: 475.25,
          ),
        ],
      ),
    ],
    fields: [
      fieldOf('name', 'Name', BridgeFieldKind.text),
      fieldOf(amount, 'Amount', BridgeFieldKind.number),
      fieldOf(half, 'Half', BridgeFieldKind.formula, '{$amount} / 2'),
    ],
  );

  testWidgets('a formula field shows each row\'s value and takes no typing', (
    tester,
  ) async {
    await kit.show(tester, [table]);
    final value = kit.inTable(find.text('475.25'));
    expect(value, findsOneWidget);
    await tester.tap(value);
    await tester.pump();
    await tester.tap(value);
    await tester.pumpAndSettle();
    for (final typed in [
      LogicalKeyboardKey.keyX,
      LogicalKeyboardKey.enter,
      LogicalKeyboardKey.backspace,
    ]) {
      await tester.sendKeyEvent(typed);
      await tester.pumpAndSettle();
    }
    expect(kit.applied(BridgeMutationKind.setCell), isEmpty);
  });

  testWidgets('the field menu edits the formula with chips for fields', (
    tester,
  ) async {
    await kit.show(tester, [table]);
    final header = kit.inTable(find.text('Half'));
    await tester.tap(header, buttons: 2);
    await tester.pumpAndSettle();
    expect(inMenu(find.text('Formula')), findsOneWidget);
    expect(inMenu(find.text('Amount')), findsOneWidget, reason: 'a chip');

    final editor = inMenu(
      find.descendant(
        of: find.byType(FormulaField),
        matching: find.byType(TextField),
      ),
    );
    await tester.tap(editor);
    await tester.pump();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '=Am',
        selection: TextSelection.collapsed(offset: 3),
      ),
    );
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pumpAndSettle();
    final chipped = tester.widget<TextField>(editor).controller!.text;
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: '$chipped * 2',
        selection: TextSelection.collapsed(offset: chipped.length + 4),
      ),
    );
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();

    expect(
      kit.applied(BridgeMutationKind.setFieldFormula).map((set) => set.text),
      ['{$amount} * 2'],
    );
  });

  testWidgets('a field formula that reads another Table opens in the menu', (
    tester,
  ) async {
    await kit.show(tester, [table]);
    final block = kit.fake.views[kit.page]!.first.block;
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.setFieldFormula,
        page: kit.page,
        block: block,
        field: half,
        text: 'SUM({$block.$amount}) / 2',
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(kit.inTable(find.text('Half')), buttons: 2);
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(inMenu(find.text('Expenses.Amount')), findsOneWidget);
  });
}
