import 'dart:async';

import '../rust/api/replica.dart';
import '../rust/api/workspace.dart';

int _next = 0;

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
    onCancel: () async {
      await workspace.close(subscription: subscription);
      await changes?.cancel();
    },
  );
  return controller.stream;
}
