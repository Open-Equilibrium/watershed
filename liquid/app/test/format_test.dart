import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/format.dart';

void main() {
  test('formats durations, costs and counts', () {
    expect(
      [formatDuration(18), formatDuration(3400), formatDuration(125000)],
      ['18 ms', '3.4 s', '2m 05s'],
    );
    expect(
      [formatDuration(59949), formatDuration(59950), formatDuration(60000)],
      ['59.9 s', '1m 00s', '1m 00s'],
      reason: 'a minute once seconds round to 60',
    );
    expect([formatCost(20000), formatCost(1000)], ['\$0.02', '\$0.001']);
    expect(
      [formatCount(6140), formatCompact(6140), formatCompact(820)],
      ['6,140', '6.1k', '820'],
    );
  });
}
