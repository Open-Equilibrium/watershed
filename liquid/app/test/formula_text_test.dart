import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/workspace/formula_text.dart';

const line =
    '{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.cccccccccccccccccccccccccccccccc}';

void main() {
  test('a value or formula keeps its chips through the editor', () {
    for (final (stored, names, shown, chips) in [
      ('', <String?>[], '', 0),
      ('420', <String?>[], '420', 0),
      ('"to pay"', <String?>[], 'to pay', 0),
      ('"{1a}" & $line', ['Flights'], '="{1a}" & ￼', 1),
      ('$line * 2', [null], '=￼ * 2', 1),
    ]) {
      final (text, held) = editable(stored, names);
      expect((text, held.length), (shown, chips), reason: stored);
      expect(storedFormula(text, held), stored, reason: stored);
    }
    expect(namedFormula('"{1a}" & $line', ['Flights']), '"{1a}" & Flights');
    expect(tokenNames('"{1a}" & $line', (ids) => ids.join('/')), [
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/cccccccccccccccccccccccccccccccc',
    ]);
    expect(storedFormula('to "pay"', const []), '"to ""pay"""');
  });

  test('deleting a leading operand keeps the reference that remains', () {
    final text = FormulaText()
      ..load('=￼ + ￼', const [Reference('{a1}', 'A'), Reference('{b2}', 'B')]);
    text.value = const TextEditingValue(
      text: '=￼',
      selection: TextSelection.collapsed(offset: 1),
    );
    expect(storedFormula(text.text, text.chips), '{b2}');
  });
}
