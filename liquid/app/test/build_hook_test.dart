import 'package:flutter_test/flutter_test.dart';

import '../hook/build.dart';

void main() {
  test('the build log leaves out the environment the Rust builder lists', () {
    final log = StringBuffer();
    buildLog(log)
      ..config({'GH_TOKEN': 'not-for-logs'})
      ..info('Running cargo build');

    expect(log.toString(), contains('Running cargo build'));
    expect(log.toString(), isNot(contains('not-for-logs')));
  });
}
