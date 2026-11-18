import '../rust/api/thread.dart';

sealed class DiffScope {
  const DiffScope();
}

class LatestTurn extends DiffScope {
  const LatestTurn();
}

class OneTurn extends DiffScope {
  const OneTurn(this.number);

  final int number;

  @override
  bool operator ==(Object other) => other is OneTurn && other.number == number;

  @override
  int get hashCode => number.hashCode;
}

class AllTurns extends DiffScope {
  const AllTurns();
}

class UncommittedChanges extends DiffScope {
  const UncommittedChanges();
}

class BranchChanges extends DiffScope {
  const BranchChanges();
}

typedef Compared = ({BridgeTurn from, BridgeTurn to});

List<BridgeTurn> finished(List<BridgeTurn> turns) => [
  for (final turn in turns)
    if (turn.checkpoint) turn,
];

Compared? compared(List<BridgeTurn> turns, DiffScope scope) {
  final ended = finished(turns);
  if (ended.isEmpty) return null;
  final latest = (from: ended.last, to: ended.last);
  return switch (scope) {
    LatestTurn() => latest,
    OneTurn(:final number) =>
      [
            for (final turn in ended)
              if (turn.number == number) (from: turn, to: turn),
          ].firstOrNull ??
          latest,
    AllTurns() => (from: ended.first, to: ended.last),
    UncommittedChanges() || BranchChanges() => null,
  };
}

String turnName(BridgeTurn turn) => 'Turn ${turn.number}';
