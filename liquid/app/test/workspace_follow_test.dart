import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/workspace/follow.dart';

/// Records subscriptions and closes as the bridge receives them.
class FakeWorkspace implements LiquidWorkspace {
  final streams = <int, StreamController<BridgeChange>>{};
  final closed = <int>[];

  final _ended = <int, Completer<void>>{};

  /// Like the bridge's stream, cancelling it finishes only once Rust drops its end, which
  /// closing the subscription does.
  @override
  Stream<BridgeChange> subscribe({
    required int subscription,
    required BridgeResource resource,
  }) {
    final ended = _ended[subscription] = Completer<void>();
    final controller = StreamController<BridgeChange>(
      onCancel: () => ended.future,
    );
    streams[subscription] = controller;
    return controller.stream;
  }

  @override
  Future<void> close({required int subscription}) async {
    closed.add(subscription);
    _ended[subscription]?.complete();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

const areas = BridgeResource(kind: BridgeResourceKind.areas);

void main() {
  test(
    'cancelling a followed resource closes its subscription in Rust',
    () async {
      final workspace = FakeWorkspace();
      final seen = <int>[];
      final first = follow(
        workspace,
        areas,
      ).listen((change) => seen.add(change.seq));
      final second = follow(workspace, areas).listen((_) {});
      await pumpEventQueue();
      expect(workspace.streams.keys.toSet(), hasLength(2));

      final id = workspace.streams.keys.first;
      workspace.streams[id]!.add(const BridgeChange(resource: areas, seq: 3));
      await pumpEventQueue();
      await first.cancel().timeout(const Duration(seconds: 1));

      expect(seen, [3]);
      expect(workspace.closed, [id]);
      await second.cancel().timeout(const Duration(seconds: 1));
      expect(workspace.closed, hasLength(2));
      expect(workspace.closed.toSet(), workspace.streams.keys.toSet());
    },
  );

  test('a stream that is never listened to opens no subscription', () async {
    final workspace = FakeWorkspace();
    follow(workspace, areas);
    await pumpEventQueue();
    expect(workspace.streams, isEmpty);
  });
}
