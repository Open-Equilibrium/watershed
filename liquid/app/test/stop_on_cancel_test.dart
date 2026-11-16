import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/session/backend.dart';

void main() {
  test('cancelling calls stop at once and cancels the source', () async {
    final source = StreamController<int>();
    final received = <int>[];
    var stops = 0;
    final subscription = stopOnCancel(source.stream, () async {
      stops++;
    }).listen(received.add);

    source.add(1);
    await pumpEventQueue();
    expect(received, [1]);

    final cancelled = subscription.cancel();
    expect(stops, 1, reason: 'stop is called within the cancel');
    await cancelled;
    expect(source.hasListener, isFalse);
    await source.close();
  });
}
