import 'dart:async';

import 'package:flutter/foundation.dart';

import '../rust/api/agents.dart';
import '../rust/api/details.dart';
import '../rust/api/session.dart';
import 'backend.dart';
import 'session_state.dart';

@immutable
class HostAgent {
  const HostAgent({required this.address, required this.agent});

  final String? address;
  final BridgeAgent agent;

  String get name => agent.name;
}

const mentionPoll = Duration(seconds: 2);

class AgentsState {
  AgentsState(this._state);

  final SessionState _state;

  final list = ValueNotifier<List<HostAgent>>(const []);
  final open = ValueNotifier<HostAgent?>(null);
  final changes = ValueNotifier<List<BridgeAgentChange>>(const []);
  final mentions = ValueNotifier<List<BridgeMention>>(const []);
  final composing = ValueNotifier(false);

  Timer? _poll;
  int _loads = 0;

  bool get earlier {
    final agent = open.value?.agent;
    return agent != null && _state.session?.id != agent.thread;
  }

  Set<String> get threads => {
    for (final hosted in list.value) ...hosted.agent.threads,
  };

  HostAgent? ofThread(String? id, String? address) {
    for (final hosted in list.value) {
      if (hosted.address == address && hosted.agent.threads.contains(id)) {
        return hosted;
      }
    }
    return null;
  }

  HostAgent? named(String name, String? address) {
    for (final hosted in list.value) {
      if (hosted.address == address && hosted.name == name) return hosted;
    }
    return null;
  }

  Future<void> load() async {
    final loading = ++_loads;
    final found = <HostAgent>[];
    for (final address in _state.hostAddresses) {
      final project = _state.projectOf(address);
      if (project == null) continue;
      try {
        for (final agent in await project.agents()) {
          found.add(HostAgent(address: address, agent: agent));
        }
      } on Object catch (_) {}
    }
    if (loading != _loads) return;
    list.value = found;
    final shown = open.value;
    if (shown != null) {
      open.value = ofThread(_state.session?.id, _state.sessionAddress);
    }
  }

  Future<void> show(HostAgent hosted) async {
    composing.value = false;
    await _state.open(hosted.agent.thread, address: hosted.address);
  }

  Future<void> compose() async {
    await _state.newSession();
    composing.value = true;
  }

  Future<bool> start(String request) async {
    final project = _state.project;
    final model = _state.model.value;
    if (project == null || model == null) {
      _state.notify(NoticeLevel.error, 'No model is available.');
      return false;
    }
    final navigations = _state.navigations;
    try {
      final agent = await project.agentStart(
        model,
        _state.permissions.value,
        request,
      );
      composing.value = false;
      await load();
      if (navigations != _state.navigations) return true;
      await show(HostAgent(address: _state.startAddress, agent: agent));
      return true;
    } on Object catch (error) {
      _state.notify(NoticeLevel.error, 'Could not start the agent: $error');
      return false;
    }
  }

  Future<void> attached(String thread, String? address) async {
    composing.value = false;
    _poll?.cancel();
    if (list.value.isEmpty) await load();
    if (_state.session?.id != thread || _state.sessionAddress != address) {
      return;
    }
    final hosted = ofThread(thread, address);
    open.value = hosted;
    changes.value = const [];
    mentions.value = const [];
    if (hosted != null) await readChanges();
    await readMentions();
  }

  void detached() {
    _poll?.cancel();
    open.value = null;
    changes.value = const [];
    mentions.value = const [];
  }

  Future<void> turnEnded() async {
    if (open.value != null) {
      await load();
      await readChanges();
    }
  }

  Future<void> readChanges() async {
    final hosted = open.value;
    if (hosted == null) return;
    try {
      final read = await _project(hosted).agentChanges(hosted.agent.id);
      if (open.value?.agent.id == hosted.agent.id) changes.value = read;
    } on Object catch (error) {
      _state.notify(NoticeLevel.warning, 'Could not read the agent: $error');
    }
  }

  Future<void> readMentions() async {
    final thread = _state.session?.id;
    final project = _state.projectOf(_state.sessionAddress);
    if (thread == null || project == null) return;
    try {
      final read = await project.agentMentions(thread);
      if (_state.session?.id != thread) return;
      mentions.value = read;
      _poll?.cancel();
      if (read.any((mention) => !mention.answered)) {
        _poll = Timer(mentionPoll, () => unawaited(readMentions()));
      }
    } on Object catch (_) {}
  }

  (HostAgent, String)? _mentioned(String text) {
    final match = RegExp(r'^@([a-z][a-z0-9-]*)\s+(\S[\s\S]*)$')
        .firstMatch(text);
    if (match == null) return null;
    final hosted = named(match.group(1)!, _state.sessionAddress);
    if (hosted == null || hosted.agent.threads.contains(_state.session?.id)) {
      return null;
    }
    return (hosted, match.group(2)!.trim());
  }

  bool isMention(String text) => _mentioned(text) != null;

  Future<bool> mention(String text) async {
    final found = _mentioned(text);
    final thread = _state.session?.id;
    if (found == null || thread == null) return false;
    final (hosted, said) = found;
    try {
      await _project(hosted).agentMention(hosted.agent.id, thread, said);
      await readMentions();
      return true;
    } on Object catch (error) {
      _state.notify(
        NoticeLevel.error,
        'Could not send the message to ${hosted.name}: $error',
      );
      return false;
    }
  }

  Future<void> apply(BridgeAgentChange change) => _change(
    'apply the change',
    (project, id) => project.agentApply(id, change.id),
  );

  Future<void> undo(BridgeAgentChange change) => _change(
    'undo the change',
    (project, id) => project.agentUndo(id, change.id),
  );

  Future<void> rename(String name) => _change(
    'rename the agent',
    (project, id) => project.agentUpdate(id, rename: name),
  );

  Future<void> setModel(BridgeModelChoice model) => _change(
    'change the model',
    (project, id) => project.agentUpdate(id, model: model),
  );

  Future<void> setPermissions(BridgePermissions permissions) => _change(
    'change the permissions',
    (project, id) => project.agentUpdate(id, permissions: permissions),
  );

  Future<String?> instructions() async {
    final hosted = open.value;
    if (hosted == null) return null;
    try {
      return await _project(hosted).agentInstructions(hosted.agent.id);
    } on Object catch (error) {
      _state.notify(
        NoticeLevel.error,
        'Could not read the instructions: $error',
      );
      return null;
    }
  }

  Future<void> setInstructions(String text) => _change(
    'save the instructions',
    (project, id) => project.agentSetInstructions(id, text),
  );

  Future<BridgeAddedRoutine?> addRoutine(
    String title,
    String prompt,
    BridgeSchedule schedule,
  ) async {
    final hosted = open.value;
    if (hosted == null) return null;
    try {
      final added = await _project(hosted)
          .agentRoutineAdd(hosted.agent.id, title, prompt, schedule);
      await load();
      return added;
    } on Object catch (error) {
      _state.notify(NoticeLevel.error, 'Could not add the routine: $error');
      return null;
    }
  }

  Future<String?> remove(HostAgent hosted) async {
    try {
      final memory = await _project(hosted).agentRemove(hosted.agent.id);
      if (open.value?.agent.id == hosted.agent.id) await _state.newSession();
      await load();
      return memory;
    } on Object catch (error) {
      _state.notify(NoticeLevel.error, 'Could not remove the agent: $error');
      return null;
    }
  }

  Future<void> _change(
    String what,
    Future<Object?> Function(ProjectBackend project, String id) action,
  ) async {
    final hosted = open.value;
    if (hosted == null) return;
    try {
      await action(_project(hosted), hosted.agent.id);
      await load();
      await readChanges();
    } on Object catch (error) {
      _state.notify(NoticeLevel.error, 'Could not $what: $error');
    }
  }

  ProjectBackend _project(HostAgent hosted) =>
      _state.projectOf(hosted.address) ??
      (throw StateError('Liquid is not connected to the agent host.'));

  void dispose() => _poll?.cancel();
}
