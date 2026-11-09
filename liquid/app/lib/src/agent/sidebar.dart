import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/logo.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/explorer.dart';
import '../workspace/workspace_state.dart';
import 'session_menus.dart';

/// What the sidebar shows about a session.
@immutable
class SessionInfo {
  const SessionInfo({
    required this.project,
    required this.harness,
    required this.device,
    this.path,
  });

  final String project;

  /// The project's folder, when known.
  final String? path;
  final String harness;
  final String device;
}

/// The open session's title, or "New session" before it has one.
class SessionTitle extends StatelessWidget {
  const SessionTitle({super.key, required this.state, this.style});

  final SessionState state;
  final TextStyle? style;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.sessionTitle,
    builder: (context, title, _) => Text(
      title ?? 'New session',
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: style,
    ),
  );
}

/// No Page is open without a Workspace.
final _noPage = ValueNotifier<OpenPage?>(null);

/// The Explorer above the agent sessions with project, harness and device per session, and
/// Settings in its footer (approved Pages mockups).
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.state,
    this.info,
    this.workspace,
    this.workspaceProblem,
    this.onSettings,
    this.onNewSession,
    this.onShowSession,
  });

  /// The agent session, if one started.
  final SessionState? state;
  final SessionInfo? info;

  /// The Workspace, or why it did not open.
  final WorkspaceState? workspace;
  final String? workspaceProblem;
  final VoidCallback? onSettings;

  /// Starts a new session as ⌘N does: in the chat, ready to describe the task.
  final VoidCallback? onNewSession;

  /// Shows the session in place of the open Page.
  final VoidCallback? onShowSession;

  /// Asks before deleting the open session, which a new session then replaces.
  Future<void> _deleteOpen(BuildContext context, SessionState state) async {
    final String? path;
    try {
      path = await state.sessionFile();
    } on Object catch (error) {
      state.notify(BridgeLevel.error, 'Could not read the session: $error');
      return;
    }
    if (!context.mounted) return;
    if (path == null) {
      state.notify(
        BridgeLevel.info,
        'pi keeps this session in memory, so it has no file.',
      );
      return;
    }
    await confirmDeleteSession(
      context,
      state,
      path: path,
      title: state.sessionTitle.value ?? 'New session',
    );
  }

  /// The project and the session's card, which shows the session when tapped.
  List<Widget> _session(
    BuildContext context,
    SessionState state,
    SessionInfo info,
    TextStyle small,
  ) {
    final ws = context.ws;
    return [
      RowMenu(
        actions: [
          if (info.path case final path?)
            RowAction(
              Platform.isMacOS ? 'Open in Finder' : 'Open in file manager',
              () => openInFileManager(state, path),
            ),
          RowAction(
            'New agent session',
            onNewSession ?? () => unawaited(state.newSession()),
            shortcut: Keymap.of(context).label(Command.newSession),
          ),
          null,
          RowAction(
            'Remove all checkpoints…',
            () => confirmRemoveCheckpoints(context, state, info.project),
            danger: true,
          ),
        ],
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: WsMetrics.space3,
            vertical: WsMetrics.space2,
          ),
          child: Text(
            info.project,
            style: TextStyle(
              color: ws.textMuted,
              fontWeight: WsMetrics.fwMedium,
            ),
          ),
        ),
      ),
      RowMenu(
        actions: [
          RowAction(
            'Delete session…',
            () => _deleteOpen(context, state),
            danger: true,
          ),
        ],
        // Marked while it shows rather than a Page.
        child: ValueListenableBuilder(
          valueListenable: workspace?.open ?? _noPage,
          builder: (context, open, card) => Semantics(
            selected: open == null,
            child: Shortcuts(
              shortcuts: activateKeys,
              child: InkWell(
                key: const Key('session-card'),
                onTap: onShowSession,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                child: Ink(
                  decoration: BoxDecoration(
                    color: open == null ? ws.hover : null,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: card,
                ),
              ),
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space6,
              WsMetrics.space2,
              WsMetrics.space3,
              WsMetrics.space2,
            ),
            child: ValueListenableBuilder(
              valueListenable: state.status,
              builder: (context, status, _) {
                final (label, color) = statusLabel(context, status);
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Container(
                          width: 7,
                          height: 7,
                          decoration: BoxDecoration(
                            color: color,
                            shape: BoxShape.circle,
                          ),
                        ),
                        const SizedBox(width: WsMetrics.space3),
                        Expanded(child: SessionTitle(state: state)),
                        Text(
                          label,
                          style: TextStyle(
                            color: color,
                            fontSize: WsMetrics.fsSmall,
                          ),
                        ),
                      ],
                    ),
                    Padding(
                      padding: const EdgeInsets.only(left: 15),
                      // The checkout's branch, live.
                      child: ValueListenableBuilder(
                        valueListenable: state.git.status,
                        builder: (context, git, _) => Text(
                          [
                            info.harness,
                            ?(git?.branch ?? git?.commit),
                            info.device,
                          ].join(' · '),
                          style: small,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                    ),
                  ],
                );
              },
            ),
          ),
        ),
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      color: ws.textFaint,
      fontSize: WsMetrics.fsSmall,
      height: WsMetrics.lhSmall / WsMetrics.fsSmall,
    );
    return Container(
      width: WsMetrics.widthSide,
      decoration: BoxDecoration(
        color: ws.subtle,
        border: Border(right: BorderSide(color: ws.border)),
      ),
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space3,
        WsMetrics.space5,
        WsMetrics.space3,
        WsMetrics.space4,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            child: Row(
              children: [
                const LiquidMark(),
                const SizedBox(width: WsMetrics.space3),
                Text(
                  'Liquid',
                  style: Theme.of(context).textTheme.labelMedium
                      ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                ),
              ],
            ),
          ),
          const SizedBox(height: WsMetrics.space3),
          Expanded(
            child: ListView(
              children: [
                if (workspace case final workspace?) Explorer(state: workspace),
                if (workspaceProblem case final problem?)
                  Padding(
                    padding: const EdgeInsets.only(top: WsMetrics.space3),
                    child: WarningNotice(problem),
                  ),
                // A Page shows the Workspace's latest problem itself; without one, it shows
                // here.
                if (workspace case final workspace?)
                  ListenableBuilder(
                    listenable: Listenable.merge([
                      workspace.problem,
                      workspace.open,
                    ]),
                    builder: (context, _) => switch ((
                      workspace.problem.value,
                      workspace.open.value,
                    )) {
                      (final problem?, null) => Padding(
                        padding: const EdgeInsets.only(top: WsMetrics.space3),
                        child: WarningNotice(problem),
                      ),
                      _ => const SizedBox.shrink(),
                    },
                  ),
                const SidebarHeading('Agent sessions'),
                if ((state, info) case (final state?, final info?))
                  ..._session(context, state, info, small),
              ],
            ),
          ),
          if (onSettings case final onSettings?)
            Tooltip(
              message: 'Settings',
              child: Shortcuts(
                shortcuts: activateKeys,
                child: InkWell(
                  onTap: onSettings,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  child: Container(
                    height: WsMetrics.rowSide,
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space3,
                    ),
                    child: Row(
                      children: [
                        Icon(
                          LucideIcons.settings,
                          size: 14,
                          color: ws.textMuted,
                        ),
                        const SizedBox(width: WsMetrics.space3),
                        Expanded(
                          child: Text(
                            'Settings',
                            style: TextStyle(color: ws.textMuted),
                          ),
                        ),
                        Hint(Keymap.of(context).label(Command.settings)),
                      ],
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Short status text and its color.
(String, Color) statusLabel(BuildContext context, RunStatus status) {
  final ws = context.ws;
  return switch (status) {
    RunStatus.idle => ('idle', ws.textFaint),
    RunStatus.running => ('running', ws.accent),
    RunStatus.waitingForYou => ('waiting', ws.warning),
    RunStatus.exited => ('exited', ws.textFaint),
    RunStatus.failed => ('failed', ws.danger),
  };
}
