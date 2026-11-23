part of 'session_state.dart';

extension SessionMoves on SessionState {
  void retryMove() => unawaited(loadStart());

  Future<void> cancelMove() async {
    final id = _session?.id;
    final move = moving.value;
    final source = projectOf(_sessionAddress);
    if (id == null || move == null || source == null) return;
    final reach = await _linkTo(
      move.targetEnvironment,
      except: [_sessionAddress],
    );
    BridgeArrival? answer;
    try {
      answer = await (await reach.link.backend).moveAbort(move.transfer);
    } on Object {
      answer = null;
    } finally {
      reach.link.drop();
    }
    final kind = answer?.kind;
    if (kind == BridgeArrivalKind.starting ||
        kind == BridgeArrivalKind.unknown) {
      notify(
        NoticeLevel.warning,
        '${move.targetName} is still taking the session, so the move cannot be cancelled yet.',
      );
      return;
    }
    final landed = kind == BridgeArrivalKind.landed ? answer?.thread : null;
    try {
      if (landed == null) {
        await source.moveFinish(id, move.transfer, null);
        await loadStart();
      } else {
        await source.moveFinish(id, move.transfer, move.targetName);
        notify(
          NoticeLevel.info,
          '${move.targetName} already had this session, so the move finished.',
        );
        await open(landed, address: reach.address);
        await loadStart();
      }
    } on Object catch (error) {
      notify(NoticeLevel.error, 'Could not cancel the move: $error');
    }
  }

  void _syncMoving() {
    final id = _session?.id;
    moving.value = id == null
        ? null
        : recent.value
              .where(
                (listed) =>
                    listed.thread.id == id && listed.address == _sessionAddress,
              )
              .firstOrNull
              ?.thread
              .moving;
  }

  void _settledListing(HostThread listed) {
    final thread = listed.thread;
    recent.value = [
      for (final item in recent.value)
        if (identical(item, listed))
          HostThread(
            address: item.address,
            machine: item.machine,
            thread: BridgeThreadSummary(
              id: thread.id,
              title: thread.title,
              updatedMs: thread.updatedMs,
              branch: thread.branch,
              worktree: thread.worktree,
              running: thread.running,
              archived: thread.archived,
              movable: thread.movable,
              harnessKnown: thread.harnessKnown,
            ),
          )
        else
          item,
    ];
  }

  Future<void> recoverMoves() async {
    DateTime? wake;
    for (final listed in [...recent.value]) {
      final moving = listed.thread.moving;
      if (moving == null || runningMoves.contains(moving.transfer)) continue;
      final since = DateTime.fromMillisecondsSinceEpoch(moving.sinceMs.toInt());
      final age = clock().difference(since);
      final started = startedMoves.contains(moving.transfer);
      if (!started && age < recoveryAge) {
        final due = since.add(recoveryAge);
        if (wake == null || due.isBefore(wake)) wake = due;
      }
      final source = listed.address;
      final sourceLink = _ownConnection(source);
      runningMoves.add(moving.transfer);
      final targetLink = (await _linkTo(
        moving.targetEnvironment,
        except: [source],
      )).link;
      moveWaits.value = null;
      Outcome? outcome;
      try {
        outcome = await settleMove(
          source: sourceLink,
          target: targetLink,
          sourceName: hostNameOf(source),
          targetName: moving.targetName,
          threadId: listed.thread.id,
          transfer: moving.transfer,
          startedHere: started,
          age: age,
        );
        if (outcome.settled == Settled.sealed) {
          notify(
            NoticeLevel.info,
            'Moved “${listed.thread.title}” to ${moving.targetName}.',
          );
        } else if (outcome.settled == Settled.released) {
          notify(
            NoticeLevel.warning,
            'The move of “${listed.thread.title}” to ${moving.targetName} did not happen; the session stayed on ${hostNameOf(source)}.',
          );
        }
        if (outcome.settled != Settled.unfinished) _settledListing(listed);
      } finally {
        runningMoves.remove(moving.transfer);
        moveWaits.value = outcome?.settled == Settled.unfinished
            ? (transfer: moving.transfer, waitingFor: outcome!.waitingFor)
            : null;
        sourceLink.drop();
        targetLink.drop();
      }
    }
    _recoveryTimer?.cancel();
    _recoveryTimer = wake == null
        ? null
        : Timer(wake.difference(clock()), () {
            _recoveryTimer = null;
            unawaited(recoverMoves());
          });
  }

  String _labelOf(String name) {
    final known = _agents.names
        .where((entry) => entry.value.toLowerCase() == name.toLowerCase())
        .firstOrNull;
    if (known == null) return name;
    final label = hostLabel;
    if (label != null) return label(known.key);
    return _agents.nameOf(known.key);
  }

  ItemView _carriedView(BridgeTranscriptItem entry, int index) => _settled(
    ItemView(
      id: _moveRowsBase - index,
      kind: entry.item.kind,
      turn: 0,
      tool: entry.item.tool,
      target: entry.item.target,
    ),
    entry,
  );

  Future<BridgeMoveRecord?> _recordOf(ProjectBackend host, String id) async {
    try {
      return await host.moveRecord(id);
    } on Object {
      return null;
    }
  }
}

const _moveRowsBase = -(1 << 40);
const _moveMarkerId = _moveRowsBase - (1 << 30);
