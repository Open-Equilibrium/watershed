import 'dart:async';
import 'dart:io';

import 'agent/shell.dart';
import 'rust/api/session.dart';
import 'session/backend.dart';
import 'session/session_state.dart';

/// The session the app opens, or why it opened none.
class Launch {
  const Launch.session(this.state, this.info) : problem = null;
  const Launch.problem(this.problem) : state = null, info = null;

  final SessionState? state;
  final SessionInfo? info;
  final String? problem;
}

/// pi's arguments: RPC mode with Watershed's extension, reopening the stored `session` file
/// when given, so its Trajectory and branches come back after a restart.
List<String> piArguments(String extension, String? session) => [
  '--mode',
  'rpc',
  '-e',
  extension,
  if (session != null) ...['--session', session],
];

/// Starts a session from the environment until the app has an approved flow for new sessions.
/// Without Watershed's pi extension there would be no approvals, so the app refuses to start pi.
Future<Launch> launchFromEnvironment(Map<String, String> environment) async {
  final project = environment['LIQUID_PROJECT'];
  final extension = environment['LIQUID_PI_EXTENSION'];
  if (project == null || extension == null) {
    return const Launch.problem(
      'Start Liquid with LIQUID_PROJECT set to a project directory and LIQUID_PI_EXTENSION set '
      'to meta-harness/pi-extension/watershed.ts.',
    );
  }
  try {
    final session = await AgentSession.start(
      config: SessionConfig(
        program: environment['LIQUID_PI'] ?? 'pi',
        args: piArguments(extension, environment['LIQUID_PI_SESSION']),
        cwd: project,
      ),
    );
    final state = SessionState(RustSessionBackend(session));
    // A reopened session shows its current branch right away.
    unawaited(state.loadChat());
    return Launch.session(
      state,
      SessionInfo(
        title: 'Agent session',
        project: Directory(project).uri.pathSegments
            .where((part) => part.isNotEmpty)
            .last,
        harness: 'pi',
        device: Platform.localHostname,
      ),
    );
  } on Object catch (error) {
    return Launch.problem('pi did not start: $error');
  }
}
