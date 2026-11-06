import 'dart:async';
import 'dart:io';

import 'agent/sidebar.dart';
import 'rust/api/session.dart';
import 'rust/api/workspace.dart';
import 'session/backend.dart';
import 'session/branch_bases.dart';
import 'session/session_state.dart';
import 'workspace/workspace_state.dart';

/// The session the app opens, or why it opened none.
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

/// Starts pi for the project the environment names; the start page then begins or continues a
/// session.
/// Without Watershed's pi extension there would be no approvals, so the app refuses to start pi.
Future<Launch> launchFromEnvironment(
  Map<String, String> environment, {
  BranchBases? bases,
}) async {
  final project = environment['LIQUID_PROJECT'];
  final extension = environment['LIQUID_PI_EXTENSION'];
  // pi reads a relative extension path inside the project, which could supply its own.
  if (project == null || extension == null || !File(extension).isAbsolute) {
    return const Launch.problem(
      'Start Liquid with LIQUID_PROJECT set to a project directory and LIQUID_PI_EXTENSION set '
      'to the absolute path of meta-harness/pi-extension/watershed.ts.',
    );
  }
  // Starting pi in a missing directory fails like a missing pi.
  if (!Directory(project).existsSync()) {
    return Launch.problem('LIQUID_PROJECT names no directory: $project');
  }
  // pi starts inside the project, which could supply a relative program or session file; a
  // bare program name is looked up on PATH.
  if (environment['LIQUID_PI'] case final pi?
      when pi.contains('/') && !File(pi).isAbsolute) {
    return Launch.problem('LIQUID_PI must be an absolute path: $pi');
  }
  if (environment['LIQUID_PI_SESSION'] case final session?
      when !File(session).isAbsolute) {
    return Launch.problem(
      'LIQUID_PI_SESSION must be an absolute path: $session',
    );
  }
  final name = projectName(project);
  try {
    final session = await AgentSession.start(
      config: SessionConfig(
        program: environment['LIQUID_PI'] ?? 'pi',
        extensionPath: extension,
        // A stored session reopens with its Trajectory and branches after a restart.
        session: environment['LIQUID_PI_SESSION'],
        cwd: project,
      ),
    );
    final state = SessionState(RustSessionBackend(session), bases: bases);
    // A reopened session shows its current branch right away.
    unawaited(state.loadChat());
    return Launch.session(
      state,
      SessionInfo(
        project: name,
        harness: 'pi',
        device: Platform.localHostname,
        path: Directory(project).absolute.path,
      ),
    );
  } on Object catch (error) {
    return Launch.problem('pi did not start: $error');
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
