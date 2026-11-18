import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/moves.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/backend.dart';
import 'package:liquid/src/session/move.dart';
import 'package:liquid/src/session/move_rule.dart';
import 'package:liquid/src/session/session_state.dart';

import 'fake_backend.dart';

const studio = 'ws://studio.local:3774';
const sourceRoot = '/work/acme-api';
const targetRoot = '/home/me/acme-api';
const flowCalls = 5;
const recoveryCalls = 9;
const recoveryCallsWhileStarting = 6;
const _lost = MoveConnectionLost(
  'Meta-Harness closed the connection before it answered.',
);
const _kinds = [
  'lost',
  'quitBefore',
  'quitAfter',
  'restartSource',
  'restartSourceLate',
  'restartTarget',
  'restartTargetLate',
  'restartTargetLanded',
  'restartTargetDelivering',
];

class Fault {
  const Fault(this.kind);

  final String kind;
}

class Msg {
  const Msg(this.id, this.text);

  final String id;
  final String text;
}

class Commit {
  Commit(this.id, this.files);

  final String id;
  final List<String> files;
}

class Checkout {
  Checkout({Set<String>? uncommitted, List<Commit>? commits})
    : uncommitted = uncommitted ?? <String>{},
      commits = commits ?? <Commit>[];

  final Set<String> uncommitted;
  final List<Commit> commits;
}

class WorldThread {
  WorldThread(this.id, {List<Msg> queued = const [], this.root, this.worktree})
    : queued = List.of(queued),
      held = {for (final msg in queued) msg.id};

  final String id;
  final List<Msg> queued;
  final List<Msg> saved = [];
  final Set<String> held;
  final List<String> ran = [];
  final String? root;
  final String? worktree;
  BridgeMoving? moving;
  bool sealed = false;
}

class Arrival {
  Arrival(this.phase, [this.thread]);

  final String phase;
  final String? thread;
  int delivered = 0;
}

String snapshotOf(Checkout checkout) => [
  (checkout.uncommitted.toList()..sort()).join(','),
  for (final commit in checkout.commits) commit.id,
].join('|');

class World {
  World({bool sourceIsThisMac = true}) {
    local = WorldHost(this, null);
    remote = WorldHost(this, studio);
    source = sourceIsThisMac ? local : remote;
    target = sourceIsThisMac ? remote : local;
    source.checkouts[sourceRoot] = Checkout(
      uncommitted: {'port.dart'},
      commits: [
        Commit('c-base', ['readme.md']),
      ],
    );
    target.checkouts[targetRoot] = Checkout(
      uncommitted: {'notes.md'},
      commits: [
        Commit('c-target', ['index.md']),
      ],
    );
    source.threadsById['t1'] = WorldThread(
      't1',
      queued: const [Msg('src-1', 'one'), Msg('src-2', 'two')],
      root: sourceRoot,
    );
    before[sourceRoot] = snapshotOf(source.checkouts[sourceRoot]!);
    before[targetRoot] = snapshotOf(target.checkouts[targetRoot]!);
  }

  final before = <String, String>{};

  late final WorldHost local;
  late final WorldHost remote;
  late final WorldHost source;
  late final WorldHost target;
  DateTime now = DateTime(2026, 11, 18, 12);
  final packages = <String, List<Msg>>{};
  int calls = 0;
  Map<int, Fault> faults = const {};
  final fired = <int>[];

  WorldHost hostFor(String? address) => address == null ? local : remote;

  MovePlan get plan => MovePlan(
    title: 'Fix the flaky port test',
    sourceName: source.address == null ? 'This Mac' : 'studio',
    targetName: target.address == null ? 'This Mac' : 'studio',
    branch: 'fix/ports',
    uncommitted: 1,
    running: false,
    commit: false,
    root: targetRoot,
    cloneRemote: null,
    bringing: null,
  );

  void phase(Map<int, Fault> next) {
    calls = 0;
    faults = next;
    fired.clear();
  }

  Fault? enter() {
    final index = calls++;
    final fault = faults[index];
    if (fault != null) fired.add(index);
    return fault;
  }

  Future<void> restoreAndResume() async {
    local.down = false;
    remote.down = false;
    for (final host in [local, remote]) {
      await host.resumeDeliveries();
    }
  }

  int undelivered(String transfer) =>
      (packages[transfer]?.length ?? 0) -
      (target.arrivals[transfer]?.delivered ?? 0);

  void drain() {
    for (final host in [local, remote]) {
      for (final thread in host.threadsById.values) {
        if (thread.sealed) continue;
        while (thread.queued.isNotEmpty) {
          thread.ran.add(thread.queued.removeAt(0).text);
        }
      }
    }
  }
}

class WorldHost extends FakeProject {
  WorldHost(this.world, this.address) {
    environment = address == null ? 'env-local' : 'env-studio';
  }

  final World world;
  final String? address;
  final threadsById = <String, WorldThread>{};
  final arrivals = <String, Arrival>{};
  final checkouts = <String, Checkout>{};
  bool down = false;

  WorldThread _thread(String id) =>
      threadsById[id] ?? (throw StateError('no thread $id on $address'));

  WorldHost _restarted(String kind) =>
      kind.contains('Source') ? world.source : world.target;

  Future<Fault?> _begin() async {
    if (down) throw _lost;
    final fault = world.enter();
    if (fault?.kind.startsWith('restart') ?? false) {
      _restarted(fault!.kind).down = true;
    }
    if (fault?.kind == 'quitBefore') await Completer<void>().future;
    return fault;
  }

  int? _crash(Fault? fault) {
    if (fault == null || !fault.kind.startsWith('restart')) return null;
    if (_restarted(fault.kind) != this) return null;
    return switch (fault.kind) {
      'restartTargetLanded' => 3,
      'restartTargetDelivering' => 4,
      _ => fault.kind.endsWith('Late') ? 2 : 1,
    };
  }

  void _step(int? crash, int step) {
    if (crash != null && step >= crash) throw _lost;
  }

  Future<void> _end(Fault? fault, int? crash) async {
    if (crash != null || fault?.kind == 'lost') throw _lost;
    if (fault?.kind == 'quitAfter') await Completer<void>().future;
  }

  @override
  Future<List<BridgeThreadSummary>> threads() async {
    final fault = await _begin();
    final listed = [
      for (final thread in threadsById.values)
        if (!thread.sealed)
          threadOf(
            thread.id,
            title: 'Fix the flaky port test',
            moving: thread.moving,
          ),
    ];
    await _end(fault, _crash(fault));
    return listed;
  }

  @override
  Future<BridgeMoveCheck> moveCheck(String id) async {
    final fault = await _begin();
    final thread = _thread(id);
    final check = moveCheckWith(
      moveCheckAnswer,
      moved: thread.sealed,
      moving: thread.moving,
    );
    await _end(fault, _crash(fault));
    return check;
  }

  @override
  Future<CarriedMove> moveOut(
    String id, {
    required String transfer,
    required String targetAddress,
    required String targetName,
    required String targetEnvironment,
    required bool commit,
  }) async {
    final fault = await _begin();
    final crash = _crash(fault);
    final thread = _thread(id);
    final open = thread.moving;
    if (open != null && open.transfer != transfer) {
      throw 'This session is already moving to another host.';
    }
    if (open == null) {
      final since = world.now.millisecondsSinceEpoch;
      thread.moving = BridgeMoving(
        transfer: transfer,
        phase: BridgeMovePhase.preparing,
        targetAddress: targetAddress,
        targetName: targetName,
        targetEnvironment: targetEnvironment,
        sinceMs: since,
      );
      _step(crash, 1);
      final checkout = checkouts[thread.root]!;
      if (commit && checkout.uncommitted.isNotEmpty) {
        checkout.commits.add(
          Commit('c-$transfer', checkout.uncommitted.toList()),
        );
        checkout.uncommitted.clear();
      }
      thread.saved
        ..clear()
        ..addAll(thread.queued);
      thread.queued.clear();
      _step(crash, 2);
      thread.moving = BridgeMoving(
        transfer: transfer,
        phase: BridgeMovePhase.handed,
        targetAddress: targetAddress,
        targetName: targetName,
        targetEnvironment: targetEnvironment,
        sinceMs: since,
      );
      world.packages[transfer] = [
        for (var index = 0; index < thread.saved.length; index++)
          Msg('queued-$transfer-$index', thread.saved[index].text),
      ];
    }
    await _end(fault, crash);
    return FakeCarriedMove(transfer);
  }

  bool launchedWorktree = false;

  WorldThread _launch(String transfer, String threadId) {
    final worktree = 'wt-$transfer';
    checkouts[worktree] = Checkout();
    launchedWorktree = true;
    return WorldThread(threadId, worktree: worktree);
  }

  int _undelivered(String transfer) =>
      world.packages[transfer]!.length - arrivals[transfer]!.delivered;

  Future<void> _deliver(String transfer, int? crash) async {
    final arrival = arrivals[transfer]!;
    final thread = threadsById[arrival.thread]!;
    final package = world.packages[transfer]!;
    while (arrival.delivered < package.length) {
      final msg = package[arrival.delivered];
      if (thread.held.add(msg.id)) thread.queued.add(msg);
      arrival.delivered++;
      if (crash == 4 && arrival.delivered == 1) throw _lost;
    }
  }

  Future<void> resumeDeliveries() async {
    for (final transfer in arrivals.keys.toList()) {
      if (arrivals[transfer]!.phase == 'landed') {
        await _deliver(transfer, null);
      }
    }
  }

  @override
  Future<SessionBackend> moveIn(
    CarriedMove move, {
    required String root,
  }) async {
    final fault = await _begin();
    final crash = _crash(fault);
    final transfer = move.transfer;
    final known = arrivals[transfer];
    if (known?.phase == 'refused') {
      throw 'This transfer was refused, so the session did not move.';
    }
    if (known?.phase != 'landed') {
      final threadId = known?.thread ?? 'thread-$transfer';
      arrivals[transfer] = Arrival('starting', threadId);
      _step(crash, 1);
      threadsById.putIfAbsent(threadId, () => _launch(transfer, threadId));
      _step(crash, 2);
      arrivals[transfer] = Arrival('landed', threadId);
      _step(crash, 3);
    }
    final threadId = arrivals[transfer]!.thread!;
    await _deliver(transfer, crash);
    if (_undelivered(transfer) > 0) {
      throw 'The moved messages are not all delivered yet, so the session has not finished moving; try again.';
    }
    await _end(fault, crash);
    return FakeSession(id: threadId);
  }

  @override
  Future<BridgeArrival> moveStatus(String transfer) async {
    final fault = await _begin();
    final crash = _crash(fault);
    final known = arrivals[transfer];
    final BridgeArrival answer;
    if (known?.phase == 'landed') {
      await _deliver(transfer, crash);
      answer = _undelivered(transfer) > 0
          ? const BridgeArrival(kind: BridgeArrivalKind.starting)
          : BridgeArrival(
              kind: BridgeArrivalKind.landed,
              thread: known!.thread,
            );
    } else if (known?.phase == 'starting' &&
        threadsById.containsKey(known!.thread)) {
      answer = const BridgeArrival(kind: BridgeArrivalKind.starting);
    } else {
      answer = const BridgeArrival(kind: BridgeArrivalKind.unknown);
    }
    await _end(fault, crash);
    return answer;
  }

  @override
  Future<BridgeArrival> moveAbort(String transfer) async {
    final fault = await _begin();
    final crash = _crash(fault);
    final known = arrivals[transfer];
    if (known?.phase == 'landed') {
      await _deliver(transfer, crash);
      final answer = _undelivered(transfer) > 0
          ? const BridgeArrival(kind: BridgeArrivalKind.starting)
          : BridgeArrival(
              kind: BridgeArrivalKind.landed,
              thread: known!.thread,
            );
      await _end(fault, crash);
      return answer;
    }
    final threadId = known?.thread;
    if (threadId != null) threadsById.remove(threadId);
    _step(crash, 1);
    arrivals[transfer] = Arrival('refused');
    await _end(fault, crash);
    return const BridgeArrival(kind: BridgeArrivalKind.refused);
  }

  @override
  Future<void> moveFinish(String id, String transfer, String? toHost) async {
    final fault = await _begin();
    final crash = _crash(fault);
    final thread = _thread(id);
    if (toHost != null) {
      if (world.undelivered(transfer) > 0) {
        throw 'This move has messages that are not delivered yet, so it cannot be sealed.';
      }
      if (thread.moving == null && !thread.sealed) {
        throw 'This move is not handed over yet, so it cannot be sealed.';
      }
      if (thread.moving?.phase == BridgeMovePhase.preparing) {
        throw 'This move is not handed over yet, so it cannot be sealed.';
      }
      thread.sealed = true;
      thread.moving = null;
      thread.saved.clear();
    } else {
      if (thread.sealed) {
        throw 'This move was sealed, so it cannot be released.';
      }
      if (thread.moving?.transfer == transfer) {
        thread.queued.insertAll(0, thread.saved);
        thread.saved.clear();
        thread.moving = null;
      }
    }
    await _end(fault, crash);
  }

  @override
  Future<SessionBackend> open(String id) async {
    final fault = await _begin();
    await _end(fault, _crash(fault));
    return FakeSession(id: id);
  }
}

void liquidMoves(World world) {
  final flow = MoveFlow(
    open: (address, root) async => world.hostFor(address),
    from: (address: world.source.address, root: sourceRoot),
    to: (address: world.target.address, root: targetRoot),
    threadId: 't1',
    plan: world.plan,
    running: <String>{},
    started: <String>{},
    newTransfer: () => 'transfer-1',
  );
  unawaited(flow.run().then<void>((_) {}, onError: (_) {}));
}

Future<void> recover(World world, {bool paired = true}) async {
  final state = SessionState(world.local)
    ..clock = (() => world.now)
    ..openProject = (address, root) async {
      if (address == studio && !paired) {
        throw 'studio is not paired with this Mac';
      }
      return world.hostFor(address);
    };
  if (paired) unawaited(state.addHost(studio, world.remote));
  unawaited(state.loadStart());
  await pumpEventQueue();
  await state.dispose();
}

Future<void> userCancels(World world) async {
  final state = SessionState(world.local)
    ..clock = (() => world.now)
    ..openProject = (address, root) async => world.hostFor(address);
  await state.addHost(studio, world.remote);
  await pumpEventQueue();
  await state.open('t1', address: world.source.address);
  await pumpEventQueue();
  await state.cancelMove();
  await pumpEventQueue();
  await state.dispose();
}

Future<void> staleDeviceRetriesArrival(World world) async {
  if (!world.packages.containsKey('transfer-1')) return;
  try {
    await world.target.moveIn(FakeCarriedMove('transfer-1'), root: targetRoot);
  } on Object catch (_) {}
}

bool targetHoldsSession(World world) {
  final arrival = world.target.arrivals['transfer-1'];
  return arrival?.phase == 'landed' &&
      world.target.threadsById.containsKey(arrival!.thread);
}

void expectSettled(World world) {
  final source = world.source.threadsById['t1']!;
  expect(source.moving, isNull, reason: 'the move is settled');
  expect(
    [!source.sealed, targetHoldsSession(world)].where((live) => live).length,
    1,
    reason: 'the session is live on exactly one host',
  );
}

void expectEachMessageRunsOnce(World world) {
  world.drain();
  final ran = [
    for (final host in [world.local, world.remote])
      for (final thread in host.threadsById.values) ...thread.ran,
  ]..sort();
  expect(ran, ['one', 'two'], reason: 'each queued message ran exactly once');
}

void expectWorkKept(World world) {
  expect(
    snapshotOf(world.source.checkouts[sourceRoot]!),
    world.before[sourceRoot],
    reason: 'no checkout changes on the source: no commit, no removal',
  );
  expect(
    snapshotOf(world.target.checkouts[targetRoot]!),
    world.before[targetRoot],
    reason: 'the reused target checkout is unchanged',
  );
  expect(
    world.target.checkouts.containsKey('wt-transfer-1'),
    world.target.launchedWorktree,
    reason: 'withdrawal never removes the worktree a launch made',
  );
}

Future<void> runMove(
  World world, {
  Map<int, Fault> flow = const {},
  Map<int, Fault> recovery = const {},
  bool paired = true,
}) async {
  world.phase(flow);
  liquidMoves(world);
  await pumpEventQueue();
  expect(
    world.fired,
    unorderedEquals(flow.keys),
    reason: 'every flow fault fired',
  );
  await world.restoreAndResume();
  world.now = world.now.add(const Duration(minutes: 6));
  world.phase(recovery);
  await recover(world, paired: paired);
  expect(
    world.fired,
    unorderedEquals(recovery.keys),
    reason: 'every recovery fault fired',
  );
  await world.restoreAndResume();
  world.phase({});
  await recover(world, paired: paired);
  if (paired && world.source.threadsById['t1']!.moving != null) {
    await userCancels(world);
  }
  expectSettled(world);
  await staleDeviceRetriesArrival(world);
  expectSettled(world);
  expectEachMessageRunsOnce(world);
  expectWorkKept(world);
}

void main() {
  test(
    'the matrix covers every host call the move and its recovery make',
    () async {
      final done = World();
      done.phase({});
      liquidMoves(done);
      await pumpEventQueue();
      expect(done.calls, flowCalls);

      final open = World();
      open.phase({0: const Fault('quitAfter')});
      liquidMoves(open);
      await pumpEventQueue();
      await open.restoreAndResume();
      open.now = open.now.add(const Duration(minutes: 6));
      open.phase({});
      await recover(open);
      expect(open.calls, recoveryCalls);

      final starting = World();
      starting.phase({1: const Fault('restartTargetLate')});
      liquidMoves(starting);
      await pumpEventQueue();
      await starting.restoreAndResume();
      starting.now = starting.now.add(const Duration(minutes: 6));
      starting.phase({});
      await recover(starting);
      expect(starting.calls, recoveryCallsWhileStarting);
    },
  );

  for (final kind in _kinds) {
    for (var index = 0; index < flowCalls; index++) {
      test(
        '$kind at move call $index: after recovery the session is live on exactly one host, each message ran once, and no work was lost',
        () => runMove(World(), flow: {index: Fault(kind)}),
      );
      test(
        '$kind at move call $index, the target is This Mac: after recovery the session is live on exactly one host, each message ran once, and no work was lost',
        () =>
            runMove(World(sourceIsThisMac: false), flow: {index: Fault(kind)}),
      );
    }
  }

  for (final (flow, calls) in <(Map<int, Fault>, int)>[
    ({0: const Fault('quitAfter')}, recoveryCalls),
    ({1: const Fault('restartTargetLate')}, recoveryCallsWhileStarting),
  ]) {
    for (var index = 0; index < calls; index++) {
      for (final kind in [
        'lost',
        'quitBefore',
        'restartSource',
        'restartTarget',
      ]) {
        test(
          'a fault during recovery ($kind at recovery call $index, after the move left open at flow call ${flow.keys.single}): recovery settles the move once it is run again',
          () => runMove(World(), flow: flow, recovery: {index: Fault(kind)}),
        );
      }
    }
  }

  test('a move-in that fails while the target delivers waits on the target, is not released, and a later recovery seals it once every message is delivered', () async {
    final world = World();
    world.phase({1: const Fault('restartTargetDelivering')});
    liquidMoves(world);
    await pumpEventQueue();
    expect(world.fired, [1]);

    final source = world.source.threadsById['t1']!;
    expect(source.moving, isNotNull, reason: 'the move waits');
    expect(source.sealed, isFalse);
    expect(source.saved.map((msg) => msg.text), [
      'one',
      'two',
    ], reason: 'nothing is released');
    expect(
      world.target.arrivals['transfer-1']!.delivered,
      1,
      reason: 'one carried message is delivered, the other waits',
    );

    await world.restoreAndResume();
    world.now = world.now.add(const Duration(minutes: 6));
    world.phase({});
    await recover(world);
    expectSettled(world);
    await staleDeviceRetriesArrival(world);
    expectSettled(world);
    expectEachMessageRunsOnce(world);
    expectWorkKept(world);
  });

  test('a healthy move another device made is left alone until it is five minutes old, then recovered', () async {
    final world = World();
    world.phase({0: const Fault('quitAfter')});
    liquidMoves(world);
    await pumpEventQueue();
    await world.restoreAndResume();

    world.phase({});
    await recover(world);
    expect(
      world.source.threadsById['t1']!.moving?.phase,
      BridgeMovePhase.handed,
    );
    expect(world.target.arrivals, isEmpty, reason: 'nothing moved in');

    world.now = world.now.add(recoveryAge);
    await recover(world);
    expectSettled(world);
    await staleDeviceRetriesArrival(world);
    expectSettled(world);
    expectEachMessageRunsOnce(world);
    expectWorkKept(world);
  });

  test('a move to a target this device cannot reach changes nothing until the target answers', () async {
    final world = World();
    world.phase({0: const Fault('quitAfter')});
    liquidMoves(world);
    await pumpEventQueue();
    await world.restoreAndResume();
    world.now = world.now.add(const Duration(minutes: 6));

    world.phase({});
    await recover(world, paired: false);
    expect(
      world.source.threadsById['t1']!.moving,
      isNotNull,
      reason: 'the move stays open',
    );
    expect(world.target.arrivals, isEmpty);
    expect(world.source.threadsById['t1']!.saved.map((msg) => msg.text), [
      'one',
      'two',
    ], reason: 'the queued messages stay saved for the move');

    await recover(world);
    expectSettled(world);
    await staleDeviceRetriesArrival(world);
    expectSettled(world);
    expectEachMessageRunsOnce(world);
    expectWorkKept(world);
  });
}
