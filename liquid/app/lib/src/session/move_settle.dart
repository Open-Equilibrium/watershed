import '../rust/api/moves.dart';
import 'backend.dart';
import 'move_rule.dart';

typedef HostPlace = ({String? address, String root});

abstract interface class HostLink {
  Future<ProjectBackend> get backend;

  void drop();
}

class OwnConnection implements HostLink {
  OwnConnection(this._open, this.place);

  final ProjectOpener _open;
  final HostPlace place;
  ProjectBackend? _live;

  @override
  Future<ProjectBackend> get backend async =>
      _live ??= await _open(place.address, place.root);

  @override
  void drop() {
    _live?.close();
    _live = null;
  }
}

class VerifiedLink implements HostLink {
  VerifiedLink(this._link, this._environment);

  final HostLink _link;
  final String _environment;
  bool _checked = false;

  @override
  Future<ProjectBackend> get backend async {
    final project = await _link.backend;
    if (!_checked) {
      if (await project.environmentId() != _environment) {
        _link.drop();
        throw StateError('This host is not the one the move was made to.');
      }
      _checked = true;
    }
    return project;
  }

  @override
  void drop() {
    _checked = false;
    _link.drop();
  }
}

class UnreachableLink implements HostLink {
  UnreachableLink(this._environment);

  final String _environment;

  @override
  Future<ProjectBackend> get backend async =>
      throw StateError('No connected host has the environment $_environment.');

  @override
  void drop() {}
}

enum Settled { sealed, released, unfinished }

typedef Outcome = ({Settled settled, String? waitingFor});

const _settleRounds = 4;

Future<Outcome> settleMove({
  required HostLink source,
  required HostLink target,
  required String sourceName,
  required String targetName,
  required String threadId,
  required String transfer,
  required bool startedHere,
  required Duration age,
}) async {
  var refused = false;
  for (var round = 0; round < _settleRounds; round++) {
    final ProjectBackend from;
    try {
      from = await source.backend;
    } on Object {
      return (settled: Settled.unfinished, waitingFor: sourceName);
    }
    final BridgeMoveCheck check;
    try {
      check = await from.moveCheck(threadId);
    } on Object {
      source.drop();
      return (settled: Settled.unfinished, waitingFor: sourceName);
    }
    if (check.moved) return (settled: Settled.sealed, waitingFor: null);
    final moving = check.moving;
    if (moving == null || moving.transfer != transfer) {
      return (settled: Settled.released, waitingFor: null);
    }
    final answer = refused
        ? TargetAnswer.refused
        : await _answer(target, transfer);
    final next = nextMove(
      openOnSource: true,
      target: answer,
      startedHere: startedHere,
      age: age,
    );
    switch (next) {
      case NextMove.nothing:
        return (settled: Settled.released, waitingFor: null);
      case NextMove.wait:
        return (
          settled: Settled.unfinished,
          waitingFor: answer == TargetAnswer.unreachable ? targetName : null,
        );
      case NextMove.seal:
        try {
          await from.moveFinish(threadId, transfer, targetName);
          return (settled: Settled.sealed, waitingFor: null);
        } on Object {
          source.drop();
        }
        break;
      case NextMove.release:
        try {
          await from.moveFinish(threadId, transfer, null);
          return (settled: Settled.released, waitingFor: null);
        } on Object {
          source.drop();
        }
        break;
      case NextMove.abort:
        final BridgeArrival arrival;
        try {
          arrival = await (await target.backend).moveAbort(transfer);
        } on Object {
          target.drop();
          return (settled: Settled.unfinished, waitingFor: targetName);
        }
        switch (arrival.kind) {
          case BridgeArrivalKind.refused:
            refused = true;
            break;
          case BridgeArrivalKind.landed:
            break;
          case BridgeArrivalKind.starting:
            return (settled: Settled.unfinished, waitingFor: null);
          case BridgeArrivalKind.unknown:
            return (settled: Settled.unfinished, waitingFor: targetName);
        }
        break;
    }
  }
  return (settled: Settled.unfinished, waitingFor: sourceName);
}

Future<TargetAnswer> _answer(HostLink target, String transfer) async {
  try {
    final arrival = await (await target.backend).moveStatus(transfer);
    return switch (arrival.kind) {
      BridgeArrivalKind.landed => TargetAnswer.landed,
      BridgeArrivalKind.starting => TargetAnswer.starting,
      _ => TargetAnswer.notLanded,
    };
  } on Object {
    target.drop();
    return TargetAnswer.unreachable;
  }
}
