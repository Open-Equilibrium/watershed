import '../rust/api/thread.dart';

/// What the Diff panel compares.
sealed class DiffScope {
  const DiffScope();
}

/// The latest turn that ended; the Diff panel starts there and follows new turns.
class LatestTurn extends DiffScope {
  const LatestTurn();
}

/// One turn, by its number.
class OneTurn extends DiffScope {
  const OneTurn(this.number);

  final int number;

  @override
  bool operator ==(Object other) => other is OneTurn && other.number == number;

  @override
  int get hashCode => number.hashCode;
}

/// Every turn: the end of the latest against the start of the first.
class AllTurns extends DiffScope {
  const AllTurns();
}

/// The checkout compared with its last commit, including untracked files.
class UncommittedChanges extends DiffScope {
  const UncommittedChanges();
}

/// The checkout's branch compared with its base, as T3's Changes scope.
class BranchChanges extends DiffScope {
  const BranchChanges();
}

/// The turns a scope compares: from the start of `from` to the end of `to`.
typedef Compared = ({BridgeTurn from, BridgeTurn to});

/// The turns that ended with a ready checkpoint.
List<BridgeTurn> finished(List<BridgeTurn> turns) => [
  for (final turn in turns)
    if (turn.checkpoint) turn,
];

/// What `scope` compares among `turns`, oldest first; none before a turn ended. A turn that is
/// no longer in the session falls back to the latest.
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

/// The turn's name.
String turnName(BridgeTurn turn) => 'Turn ${turn.number}';
