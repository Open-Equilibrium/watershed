import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/session/move_rule.dart';

void main() {
  const healthy = Duration(minutes: 4, seconds: 59);
  const stale = Duration(minutes: 5);

  final table = <(String, bool, TargetAnswer, bool, Duration, NextMove)>[
    (
      'no open move',
      false,
      TargetAnswer.landed,
      true,
      Duration.zero,
      NextMove.nothing,
    ),
    (
      'no open move, another device',
      false,
      TargetAnswer.notLanded,
      false,
      stale,
      NextMove.nothing,
    ),
    (
      'the target landed',
      true,
      TargetAnswer.landed,
      true,
      Duration.zero,
      NextMove.seal,
    ),
    (
      'the target refused',
      true,
      TargetAnswer.refused,
      true,
      Duration.zero,
      NextMove.release,
    ),
    (
      'the target has not landed',
      true,
      TargetAnswer.notLanded,
      true,
      Duration.zero,
      NextMove.abort,
    ),
    (
      'the target does not answer',
      true,
      TargetAnswer.unreachable,
      true,
      Duration.zero,
      NextMove.wait,
    ),
    (
      'a healthy move on another device',
      true,
      TargetAnswer.notLanded,
      false,
      healthy,
      NextMove.wait,
    ),
    (
      'a healthy move on another device, landed',
      true,
      TargetAnswer.landed,
      false,
      healthy,
      NextMove.wait,
    ),
    (
      'a stale move on another device, landed',
      true,
      TargetAnswer.landed,
      false,
      stale,
      NextMove.seal,
    ),
    (
      'a stale move on another device, not landed',
      true,
      TargetAnswer.notLanded,
      false,
      stale,
      NextMove.abort,
    ),
    (
      'a stale move on another device, refused',
      true,
      TargetAnswer.refused,
      false,
      stale,
      NextMove.release,
    ),
    (
      'a move this device started, not landed: aborted without waiting',
      true,
      TargetAnswer.notLanded,
      true,
      healthy,
      NextMove.abort,
    ),
    (
      'a move this device started, no answer: waits',
      true,
      TargetAnswer.unreachable,
      true,
      healthy,
      NextMove.wait,
    ),
    (
      'the target is starting: the move waits, it is neither aborted nor released',
      true,
      TargetAnswer.starting,
      true,
      Duration.zero,
      NextMove.wait,
    ),
    (
      'a stale move on another device, starting: waits',
      true,
      TargetAnswer.starting,
      false,
      stale,
      NextMove.wait,
    ),
    (
      'a stale move on another device, no answer',
      true,
      TargetAnswer.unreachable,
      false,
      stale,
      NextMove.wait,
    ),
  ];

  for (final (name, open, target, started, age, expected) in table) {
    test('$name: ${expected.name}', () {
      expect(
        nextMove(
          openOnSource: open,
          target: target,
          startedHere: started,
          age: age,
        ),
        expected,
      );
    });
  }
}
