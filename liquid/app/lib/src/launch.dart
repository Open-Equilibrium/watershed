import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'agent/sidebar.dart';
import 'agent_hosts/agent_hosts.dart';
import 'rust/api/session.dart';
import 'rust/api/replica.dart';
import 'session/backend.dart';
import 'session/session_state.dart';
import 'workspace/workspace_state.dart';

/// The Meta-Harness session the app opens, or why it opened none.
class Launch {
  const Launch.session(this.state, this.info) : problem = null;
  const Launch.problem(this.problem) : state = null, info = null;

  final SessionState? state;
  final SessionInfo? info;
  final String? problem;
}

/// The name the sidebar shows for `project`: its directory's name, or `/` for the root.
String projectName(String project) =>
    Directory(project).absolute.uri
        .normalizePath()
        .pathSegments
        .lastWhere((part) => part.isNotEmpty, orElse: () => '/');

/// Connects to Meta-Harness for the project the environment names; `META_HARNESS_HOME` may name its
/// data folder. The start page then begins or continues a session. With `activeAddress`, the
/// session runs on that agent host (ADR-0295), where the project folder is, so this Mac does not
/// look for it.
Future<Launch> launchFromEnvironment(
  Map<String, String> environment, {
  String? activeAddress,
}) async {
  final project = environment['LIQUID_PROJECT'];
  if (project == null) {
    return const Launch.problem(
      'Start Liquid with LIQUID_PROJECT set to a project directory.',
    );
  }
  final home = environment['META_HARNESS_HOME'];
  if (activeAddress == null) {
    if (!Directory(project).existsSync()) {
      return Launch.problem('LIQUID_PROJECT names no directory: $project');
    }
    // A relative folder would be looked up from wherever Liquid started.
    if (home != null && !File(home).isAbsolute) {
      return Launch.problem(
        'META_HARNESS_HOME must be an absolute path: $home',
      );
    }
  } else if (!Directory(project).isAbsolute) {
    return Launch.problem(
      'LIQUID_PROJECT must be an absolute path on the agent host $activeAddress: $project',
    );
  }
  final root = Directory(project).absolute.path;
  try {
    final host = activeAddress == null
        ? await AgentHost.connect(home: home)
        : await AgentHost.connectRemote(address: activeAddress);
    final state = SessionState(
      RustProjectBackend(host, await host.project(root: root)),
    );
    unawaited(state.loadStart());
    return Launch.session(
      state,
      SessionInfo(project: projectName(project), path: root),
    );
  } on Object catch (error) {
    if (activeAddress != null) {
      return Launch.problem(
        'Liquid could not connect to the agent host $activeAddress: ${messageOf(error)}.',
      );
    }
    // The bridge's error says that Meta-Harness is not running and where Liquid looked.
    return Launch.problem('$error');
  }
}

/// Shows the agent host at `address` (null: This Mac) as Liquid's agent area at once, and releases the
/// session it replaces. `connect` returns why the host cannot be shown, and then the area stays as it
/// was; the answer is null once it is shown.
Future<String?> showAgentHost(
  ValueNotifier<Launch> surface,
  Future<Launch> Function(String? address) connect,
  String? address,
) async {
  final next = await connect(address);
  if (next.state == null) {
    return next.problem ?? 'Liquid could not start sessions on it.';
  }
  final previous = surface.value.state;
  surface.value = next;
  if (previous != null) unawaited(previous.dispose());
  return null;
}

/// Opens the Workspace whose replica lives in `dir` and follows it, or says why it did not
/// open. `show` opens its first Page.
Future<(WorkspaceState?, String?)> openWorkspace(
  String dir, {
  required bool show,
}) async {
  final LiquidWorkspace workspace;
  try {
    workspace = await LiquidWorkspace.open(dir: dir);
  } on Object catch (error) {
    return (null, 'The Workspace did not open: $error');
  }
  return startWorkspace(workspace, show: show);
}

/// Follows an open Workspace; a failed first read is said in the sidebar, since no Page may
/// be open to say it.
Future<(WorkspaceState, String?)> startWorkspace(
  LiquidWorkspace workspace, {
  required bool show,
}) async {
  final state = WorkspaceState(workspace);
  try {
    await state.start(show: show);
  } on Object catch (error) {
    return (state, 'The Workspace did not load: $error');
  }
  return (state, null);
}
