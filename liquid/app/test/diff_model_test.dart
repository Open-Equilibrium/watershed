import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_model.dart';

void main() {
  test('the width bound covers a plain line that no sample includes', () {
    final lines = [
      for (var index = 0; index < 8; index++)
        DiffLine(DiffKind.added, '\u200B' * (990 + index)),
      DiffLine(DiffKind.added, 'x' * 900),
    ];

    expect(widestLines(lines).columns, greaterThanOrEqualTo(900));
  });
}
