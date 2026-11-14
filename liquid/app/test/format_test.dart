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
    expect(formatCount(6140), '6,140');
  });

  test('formats the session statistics as the card and panel show them', () {
    expect(
      [
        formatTokens(820),
        formatTokens(68200),
        formatTokens(212000),
        formatTokens(41000),
        formatTokens(999400),
        formatTokens(999600),
        formatTokens(1850000),
        formatTokens(12000000),
      ],
      [
        '820',
        '68.2 k',
        '212 k',
        '41.0 k',
        '999 k',
        '1.00 M',
        '1.85 M',
        '12.0 M',
      ],
      reason: 'three significant digits, with a space before the unit',
    );
    expect(
      [
        formatDuration(4100),
        formatDuration(23000),
        formatDuration(59600),
        formatDuration(125000),
        formatDuration(872000),
        formatDuration(3599000),
        formatDuration(3600000),
        formatDuration(7500000),
      ],
      [
        '4.1 s',
        '23.0 s',
        '59.6 s',
        '2m 05s',
        '14m 32s',
        '59m 59s',
        '1h 00m',
        '2h 05m',
      ],
      reason: 'one style for every duration, with hours from the first hour',
    );
    expect(
      [formatShare(95, 100), formatShare(1, 3), formatShare(0, 0)],
      ['95%', '33%', '0%'],
    );
  });
}
