import '../rust/api/history.dart';

/// Which turns the Diff tab compares (approved checkpoint mockups).
sealed class DiffScope {
  const DiffScope();
}

/// The latest turn that ended; the Diff tab starts there and follows new turns.
class LatestTurn extends DiffScope {
  const LatestTurn();
}

/// One turn, by its checkpoint id.
class OneTurn extends DiffScope {
  const OneTurn(this.id);

  final String id;

  @override
  bool operator ==(Object other) => other is OneTurn && other.id == id;

  @override
  int get hashCode => id.hashCode;
}

/// Every turn of the branch: the end of the latest against the start of the first.
class AllTurns extends DiffScope {
  const AllTurns();
}

/// The checkpoints a scope compares: the start of `from` and the end of `to`.
typedef Compared = ({BridgeTurn from, BridgeTurn to});

/// The turns that ended, whether or not their checkpoints could be taken.
List<BridgeTurn> finished(List<BridgeTurn> turns) => [
  for (final turn in turns)
    if (turn.endEntry != null) turn,
];

/// What `scope` compares among `turns`, oldest first; none before a turn ended. A turn whose
/// checkpoint failed stays, so the Diff tab says why. A turn that is no longer on the branch
/// falls back to the latest.
Compared? compared(List<BridgeTurn> turns, DiffScope scope) {
  final ended = finished(turns);
  if (ended.isEmpty) return null;
  final latest = (from: ended.last, to: ended.last);
  return switch (scope) {
    LatestTurn() => latest,
    OneTurn(:final id) =>
      [
            for (final turn in ended)
              if (turn.id == id) (from: turn, to: turn),
          ].firstOrNull ??
          latest,
    AllTurns() => (from: turns.first, to: ended.last),
  };
}

/// The turn's name, as the Trajectory numbers it.
String turnName(BridgeTurn turn) =>
    turn.number > 0 ? 'Turn ${turn.number}' : 'A turn without a prompt';
