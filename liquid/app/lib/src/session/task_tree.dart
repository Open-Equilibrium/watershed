import 'package:flutter/foundation.dart';

import '../rust/api/events.dart';

const instanceSeparator = ' › ';

@immutable
class TaskNode {
  const TaskNode({
    required this.label,
    required this.status,
    this.instance = false,
    this.children = const [],
  });

  final String label;
  final BridgeStepStatus status;
  final bool instance;
  final List<TaskNode> children;
}

List<TaskNode> taskTree(List<BridgeTaskStep> steps) {
  final phases = <String>{
    for (final step in steps)
      if (!step.text.contains(instanceSeparator)) step.text,
  };
  final children = <String, List<TaskNode>>{};
  for (final step in steps) {
    final (phase, name) = _split(step.text);
    if (name != null && phases.contains(phase)) {
      (children[phase] ??= []).add(
        TaskNode(label: name, status: step.status, instance: true),
      );
    }
  }
  return [
    for (final step in steps)
      if (!step.text.contains(instanceSeparator))
        TaskNode(
          label: step.text,
          status: step.status,
          children: children[step.text] ?? const [],
        )
      else if (!phases.contains(_split(step.text).$1))
        TaskNode(label: step.text, status: step.status),
  ];
}

String taskSummary(List<BridgeTaskStep> steps) {
  final done = steps
      .where((step) => step.status == BridgeStepStatus.completed)
      .length;
  final current =
      steps
          .where((step) => step.status == BridgeStepStatus.running)
          .firstOrNull ??
      steps
          .where((step) => step.status == BridgeStepStatus.pending)
          .firstOrNull;
  final head = 'Tasks $done/${steps.length}';
  return current == null ? head : '$head · ${current.text}';
}

typedef RunningInstance = ({String name, String phase});

List<RunningInstance> instancesOf(List<BridgeTaskStep> steps) => [
  for (final step in steps)
    if (step.status == BridgeStepStatus.running)
      if (_split(step.text) case (final phase, final name?))
        (name: name, phase: phase),
];

(String, String?) _split(String text) {
  final at = text.indexOf(instanceSeparator);
  return at < 0
      ? (text, null)
      : (text.substring(0, at), text.substring(at + instanceSeparator.length));
}
