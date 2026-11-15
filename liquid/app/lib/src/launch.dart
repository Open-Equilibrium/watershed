import 'dart:async';
import 'dart:io';

import 'agent/sidebar.dart';
import 'rust/api/session.dart';
import 'rust/api/workspace.dart';
import 'session/backend.dart';
import 'session/session_state.dart';
import 'workspace/workspace_state.dart';

/// The agent host the app opens, or why it opened none.
class Launch {
  const Launch.session(this.state, this.info, this.host) : problem = null;
  const Launch.problem(this.problem) : state = null, info = null, host = null;

  final SessionState? state;
  final SessionInfo? info;

  /// The T3 server, which stops when the app quits.
  final AgentHost? host;
  final String? problem;
}

/// The name the sidebar shows for `project`: its directory's name, or `/` for the root.
String projectName(String project) =>
    Directory(project).absolute.uri
        .normalizePath()
        .pathSegments
        .lastWhere((part) => part.isNotEmpty, orElse: () => '/');

/// Starts the pinned T3 for the project the environment names, with its data in
/// `LIQUID_T3_HOME` or else `defaultHome`; the start page then begins or continues a session.
Future<Launch> launchFromEnvironment(
  Map<String, String> environment, {
  required String defaultHome,
}) async {
  final project = environment['LIQUID_PROJECT'];
  final binary = environment['LIQUID_T3'];
  // A relative program would be looked up from wherever Liquid started.
  if (project == null || binary == null || !File(binary).isAbsolute) {
    return const Launch.problem(
      'Start Liquid with LIQUID_PROJECT set to a project directory and LIQUID_T3 set to the '
      'absolute path of the pinned t3 executable.',
    );
  }
  if (!Directory(project).existsSync()) {
    return Launch.problem('LIQUID_PROJECT names no directory: $project');
  }
  final home = environment['LIQUID_T3_HOME'] ?? defaultHome;
  if (!File(home).isAbsolute) {
    return Launch.problem('LIQUID_T3_HOME must be an absolute path: $home');
  }
  final root = Directory(project).absolute.path;
  try {
    final host = await AgentHost.start(binary: binary, home: home);
    final state = SessionState(
      RustProjectBackend(host, await host.project(root: root)),
    );
    unawaited(state.loadStart());
    return Launch.session(
      state,
      SessionInfo(project: projectName(project), path: root),
      host,
    );
  } on Object catch (error) {
    return Launch.problem('T3 Code did not start: $error');
  }
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
