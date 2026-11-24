import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/session/move_rule.dart';

void main() {
  const healthy = Duration(minutes: 4, seconds: 59);
  const stale = Duration(minutes: 5);
  const landed = TargetAnswer.landed;
  const refused = TargetAnswer.refused;
  const notLanded = TargetAnswer.notLanded;
  const unreachable = TargetAnswer.unreachable;
  const starting = TargetAnswer.starting;
  const nothing = NextMove.nothing;
  const seal = NextMove.seal;
  const release = NextMove.release;
  const abort = NextMove.abort;
  const wait = NextMove.wait;

  void nextMoveIs(
    String name,
    TargetAnswer target,
    NextMove expected, {
    bool open = true,
    bool started = true,
    Duration age = Duration.zero,
  }) {
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

  void elsewhere(
    String name,
    TargetAnswer target,
    NextMove expected,
    Duration age,
  ) => nextMoveIs(name, target, expected, started: false, age: age);

  nextMoveIs('no open move', landed, nothing, open: false);
  nextMoveIs(
    'no open move, another device',
    notLanded,
    nothing,
    open: false,
    started: false,
    age: stale,
  );
  nextMoveIs('the target landed', landed, seal);
  nextMoveIs('the target refused', refused, release);
  nextMoveIs('the target has not landed', notLanded, abort);
  nextMoveIs('the target does not answer', unreachable, wait);
  elsewhere('a healthy move on another device', notLanded, wait, healthy);
  elsewhere('a healthy move on another device, landed', landed, wait, healthy);
  elsewhere('a stale move on another device, landed', landed, seal, stale);
  elsewhere(
    'a stale move on another device, not landed',
    notLanded,
    abort,
    stale,
  );
  elsewhere('a stale move on another device, refused', refused, release, stale);
  nextMoveIs(
    'a move this device started, not landed: aborted without waiting',
    notLanded,
    abort,
    age: healthy,
  );
  nextMoveIs(
    'a move this device started, no answer: waits',
    unreachable,
    wait,
    age: healthy,
  );
  nextMoveIs(
    'the target is starting: the move waits, it is neither aborted nor released',
    starting,
    wait,
  );
  elsewhere(
    'a stale move on another device, starting: waits',
    starting,
    wait,
    stale,
  );
  elsewhere(
    'a stale move on another device, no answer',
    unreachable,
    wait,
    stale,
  );
}
