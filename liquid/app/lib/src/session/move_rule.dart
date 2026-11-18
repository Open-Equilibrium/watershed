enum TargetAnswer { landed, starting, notLanded, refused, unreachable }

enum NextMove { nothing, wait, seal, abort, release }

const recoveryAge = Duration(minutes: 5);

NextMove nextMove({
  required bool openOnSource,
  required TargetAnswer target,
  required bool startedHere,
  required Duration age,
}) {
  if (!openOnSource) return NextMove.nothing;
  if (!startedHere && age < recoveryAge) return NextMove.wait;
  return switch (target) {
    TargetAnswer.landed => NextMove.seal,
    TargetAnswer.starting => NextMove.wait,
    TargetAnswer.refused => NextMove.release,
    TargetAnswer.notLanded => NextMove.abort,
    TargetAnswer.unreachable => NextMove.wait,
  };
}
