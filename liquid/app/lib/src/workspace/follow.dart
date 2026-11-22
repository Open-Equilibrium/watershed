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

class FollowedSources {
  FollowedSources(
    this.workspace, {
    required this.changed,
    required this.failed,
  });

  final LiquidWorkspace workspace;
  final void Function() changed;
  final void Function(Object error) failed;
  final _followed = <BridgeResource, StreamSubscription<BridgeChange>>{};

  void sync(List<BridgeResource> resources) {
    final wanted = resources.toSet();
    for (final gone in [
      for (final followed in _followed.keys)
        if (!wanted.contains(followed)) followed,
    ]) {
      unawaited(_followed.remove(gone)!.cancel());
    }
    for (final resource in wanted) {
      _followed[resource] ??= follow(
        workspace,
        resource,
      ).skip(1).listen((_) => changed(), onError: failed);
    }
  }

  void dispose() {
    for (final followed in _followed.values) {
      unawaited(followed.cancel());
    }
    _followed.clear();
  }
}
