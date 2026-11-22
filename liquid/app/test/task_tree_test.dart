import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/task_tree.dart';

import 'fake_backend.dart';

void main() {
  const running = BridgeStepStatus.running;
  const pending = BridgeStepStatus.pending;

  test(
    'instance steps hang under their Phase and other steps stay at the top',
    () {
      final tree = taskTree([
        step('Plan research', BridgeStepStatus.completed),
        step('Explore', running),
        step('Explore › scraper-1', running),
        step('Explore › scraper-2', pending),
        step('Missing › fetch-1', running),
        step('Write report', pending),
      ]);

      expect(tree.map((node) => node.label), [
        'Plan research',
        'Explore',
        'Missing › fetch-1',
        'Write report',
      ]);
      expect(tree[1].children.map((node) => (node.label, node.instance)), [
        ('scraper-1', true),
        ('scraper-2', true),
      ]);
      expect(tree[2].instance, false);
    },
  );

  test('a summary counts the completed steps and names the current one', () {
    expect(
      taskSummary([
        step('Plan research', BridgeStepStatus.completed),
        step('Explore', running),
        step('Write report', pending),
      ]),
      'Tasks 1/3 · Explore',
    );
    expect(
      taskSummary([step('Write report', pending)]),
      'Tasks 0/1 · Write report',
    );
    expect(
      taskSummary([step('Done', BridgeStepStatus.completed)]),
      'Tasks 1/1',
    );
  });
}
