import 'dart:async';

import 'package:flutter/foundation.dart';

import '../agent_hosts/agent_hosts.dart';
import '../diff/turn_scope.dart';
import '../format.dart';
import '../rust/api/details.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/moves.dart';
import '../rust/api/session.dart';
import '../rust/api/thread.dart';
import 'agent_host_set.dart';
import 'agents_state.dart';
import 'move_rule.dart';
import 'move_settle.dart';
import 'task_tree.dart';
import 'backend.dart';
import 'chat_items.dart';
import 'git_state.dart';

export 'chat_items.dart';
export 'git_state.dart';

part 'session_actions.dart';
part 'session_events.dart';
part 'session_hosts.dart';
part 'session_lifecycle.dart';
part 'session_moves.dart';
part 'session_panels.dart';

enum RunStatus { idle, running, waitingForYou }

enum NoticeLevel { info, warning, error }

@immutable
class Notice {
  const Notice(this.level, this.text, this.at);

  final NoticeLevel level;
  final String text;
  final DateTime at;
}

const noticeLimit = 50;

@immutable
class HostThread {
  const HostThread({required this.address, required this.thread, this.machine});

  final String? address;
  final BridgeThreadSummary thread;

  final String? machine;
}

class SessionState {
  SessionState([ProjectBackend? thisMac, String? startsOn])
    : _startAddress = startsOn {
    if (thisMac != null) _agents.put(null, thisMac, null);
    hideWhitespace.addListener(() => unawaited(readChanges()));
    recent.addListener(_syncMoving);
  }

  final _agents = AgentHostSet();

  late final agents = AgentsState(this);

  final lastVisit = ValueNotifier<int?>(null);

  String hostNameOf(String? address) => _agents.nameOf(address);

  ProjectOpener? openProject;

  final runningMoves = <String>{};

  final startedMoves = <String>{};

  DateTime Function() clock = DateTime.now;

  final moveWaits = ValueNotifier<({String transfer, String? waitingFor})?>(
    null,
  );

  Timer? _recoveryTimer;

  final moving = ValueNotifier<BridgeMoving?>(null);

  String Function(String? address)? hostLabel;

  BridgeMoveRecord? _moveRecord;
  List<ItemView> _prefix = const [];
  String? _startAddress;
  SessionBackend? _session;
  String? _sessionAddress;
  StreamSubscription<BridgeEvent>? _subscription;

  ValueNotifier<Map<String?, String>> get hostProblems => _agents.problems;

  final sessionProblem = ValueNotifier<String?>(null);

  void Function(String? address)? onHostLost;

  bool _sessionLost = false;

  String? get startAddress => _startAddress;
  String? get sessionAddress => _sessionAddress;

  final _follows = <StreamSubscription<dynamic>>[];

  int _generation = 0;

  int _switchRequests = 0;

  SessionBackend? get session => _session;

  ProjectBackend? get project => _agents[_startAddress];

  ProjectBackend? projectOf(String? address) => _agents[address];

  List<String?> get hostAddresses => _agents.addresses;

  final status = ValueNotifier(RunStatus.idle);
  final items = ValueNotifier<List<ItemView>>(const []);

  final request = ValueNotifier<BridgeRequest?>(null);
  final sessionTitle = ValueNotifier<String?>(null);

  final draft = ValueNotifier<String?>(null);
  final runningInstances = ValueNotifier<List<RunningInstance>>(const []);
  final mention = ValueNotifier<String?>(null);
  final notice = ValueNotifier<Notice?>(null);
  final notices = ValueNotifier<List<Notice>>(const []);
  final totals = ValueNotifier(const SessionTotals());

  final queued = ValueNotifier<List<BridgeTurn>>(const []);
  final turns = ValueNotifier<List<BridgeTurn>>(const []);
  final statistics = ValueNotifier<BridgeStatistics?>(null);

  final statisticsProblem = ValueNotifier<({String title, String text})?>(null);
  final contextWindow = ValueNotifier<BridgeContext?>(null);
  final statisticsOpen = ValueNotifier(false);

  final servers = ValueNotifier<List<BridgeLocalServer>>(const []);
  final automations = ValueNotifier<List<BridgeAutomation>>(const []);
  final lineage = ValueNotifier<BridgeLineage?>(null);
  final limits = ValueNotifier<BridgeUsageLimits?>(null);
  final hostName = ValueNotifier<String?>(null);

  final scrollChatTo = ValueNotifier<int?>(null);

  final recent = ValueNotifier<List<HostThread>>(const []);
  final providers = ValueNotifier<List<BridgeProvider>>(const []);

  final model = ValueNotifier<BridgeModelChoice?>(null);
  final permissions = ValueNotifier(BridgePermissions.fullAccess);

  final newWorktree = ValueNotifier(true);

  final base = ValueNotifier<String?>(null);

  final worktree = ValueNotifier<String?>(null);

  late final git = ValueNotifier<GitState>(
    _gitFor(project?.git() ?? const NoCheckout()),
  );

  GitState _gitFor(GitBackend backend) {
    final state = GitState(
      backend,
      onError: (title, message) =>
          notify(NoticeLevel.error, '$title: $message'),
    );
    state.status.addListener(() {
      final scope = diffScope.value;
      if (scope is UncommittedChanges || scope is BranchChanges) {
        unawaited(readChanges());
      }
    });
    return state;
  }

  int _listings = 0;

  final Map<int, ItemView> _byId = {};
  final Map<String, BridgeRequest> _requests = {};
  int _sequence = 0;

  final diffScope = ValueNotifier<DiffScope>(const LatestTurn());
  final shown = ValueNotifier<Compared?>(null);
  final shownScope = ValueNotifier<DiffScope>(const LatestTurn());
  final changed = ValueNotifier<List<BridgeFileChange>?>(null);
  final changesProblem = ValueNotifier<({String title, String text})?>(null);
  final hideWhitespace = ValueNotifier(false);

  int _diffRequests = 0;

  void notify(NoticeLevel level, String text) {
    final next = Notice(level, text, DateTime.now());
    notice.value = next;
    final all = [...notices.value, next];
    notices.value = all.length > noticeLimit ? all.skip(1).toList() : all;
  }

  void dismissNotice() => notice.value = null;
  void clearNotices() {
    notice.value = null;
    notices.value = const [];
  }

  int _applied = 0;
  int _confirmed = 0;

  int? _confirmingFor;

  Future<void> dispose() async {
    _recoveryTimer?.cancel();
    await _detach();
    git.value.dispose();
    _agents.closeAll();
  }
}
