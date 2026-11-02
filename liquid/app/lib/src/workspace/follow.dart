import 'dart:async';

import '../rust/api/workspace.dart';

/// Subscription identities, unique for the life of the app.
int _next = 0;

/// Follows [resource]'s changes. The first change carries its current count and marks that
/// Rust registered the subscription, so a reader queries after it and misses no later change.
/// flutter_rust_bridge does not tell Rust when a Dart stream is cancelled, so cancelling this
/// one closes the subscription explicitly (client architecture rule 14).
Stream<BridgeChange> follow(
  LiquidWorkspace workspace,
  BridgeResource resource,
) {
  final subscription = _next++;
  StreamSubscription<BridgeChange>? changes;
  late final StreamController<BridgeChange> controller;
  controller = StreamController<BridgeChange>(
    onListen: () {
      changes = workspace
          .subscribe(subscription: subscription, resource: resource)
          .listen(
            controller.add,
            onError: controller.addError,
            onDone: controller.close,
          );
    },
    // Closing in Rust first drops its end of the stream, which the cancellation waits for.
    onCancel: () async {
      await workspace.close(subscription: subscription);
      await changes?.cancel();
    },
  );
  return controller.stream;
}
