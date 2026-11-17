import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/logo.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/explorer.dart';
import '../workspace/workspace_state.dart';
import 'session_menus.dart';

/// What the sidebar shows about the project.
@immutable
class SessionInfo {
  const SessionInfo({required this.project, this.path});

  final String project;

  /// The project's folder, when known.
  final String? path;
}

/// The open session's title, or "New thread" before it has one.
class SessionTitle extends StatelessWidget {
  const SessionTitle({super.key, required this.state, this.style});

  final SessionState state;
  final TextStyle? style;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.sessionTitle,
    builder: (context, title, _) => Text(
      title ?? 'New thread',
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: style,
    ),
  );
}

/// No Page is open without a Workspace.
final _noPage = ValueNotifier<OpenPage?>(null);

/// The Explorer above the project's agent sessions, with Settings in its footer (approved Pages
/// mockups).
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.state,
    this.info,
    this.workspace,
    this.workspaceProblem,
    this.agentProblem,
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

  /// Why no agent session started, such as an agent host that does not answer.
  final String? agentProblem;
  final VoidCallback? onSettings;

  /// Starts a new session as ⌘N does: in the chat, ready to describe the task.
  final VoidCallback? onNewSession;

  /// Shows the session in place of the open Page.
  final VoidCallback? onShowSession;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
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
                // Hover and focus ink paints on a Material above the sidebar's background,
                // inside the list, whose edges clip it.
                Material(
                  type: MaterialType.transparency,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      if (workspace case final workspace?)
                        Explorer(state: workspace),
                      if (workspaceProblem case final problem?)
                        Padding(
                          padding: const EdgeInsets.only(top: WsMetrics.space3),
                          child: WarningNotice(problem),
                        ),
                      if (agentProblem case final problem?)
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
                              padding: const EdgeInsets.only(
                                top: WsMetrics.space3,
                              ),
                              child: WarningNotice(problem, danger: true),
                            ),
                            _ => const SizedBox.shrink(),
                          },
                        ),
                      const SidebarHeading('Threads'),
                      if ((state, info) case (final state?, final info?)) ...[
                        _ProjectRow(
                          state: state,
                          info: info,
                          onNewSession: onNewSession,
                        ),
                        _Sessions(
                          state: state,
                          workspace: workspace,
                          onShow: onShowSession,
                        ),
                      ],
                    ],
                  ),
                ),
              ],
            ),
          ),
          if (onSettings case final onSettings?)
            Material(
              type: MaterialType.transparency,
              child: Tooltip(
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
            ),
        ],
      ),
    );
  }
}

/// The project's row, whose menu opens its folder or starts a session.
class _ProjectRow extends StatelessWidget {
  const _ProjectRow({
    required this.state,
    required this.info,
    required this.onNewSession,
  });

  final SessionState state;
  final SessionInfo info;
  final VoidCallback? onNewSession;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return RowMenu(
      actions: [
        if (info.path case final path?)
          RowAction(
            'Copy path',
            () => unawaited(Clipboard.setData(ClipboardData(text: path))),
          ),
        RowAction(
          'New thread',
          onNewSession ?? () => unawaited(state.newSession()),
          shortcut: Keymap.of(context).label(Command.newSession),
        ),
      ],
      builder: (context, menu, _) => Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space3,
          vertical: WsMetrics.space2,
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                info.project,
                style: TextStyle(
                  color: ws.textMuted,
                  fontWeight: WsMetrics.fwMedium,
                ),
              ),
            ),
            menu,
          ],
        ),
      ),
    );
  }
}

/// The project's sessions, newest first, as the state lists them. The open one is marked while no
/// Page is open.
class _Sessions extends StatelessWidget {
  const _Sessions({
    required this.state,
    required this.workspace,
    required this.onShow,
  });

  final SessionState state;
  final WorkspaceState? workspace;
  final VoidCallback? onShow;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.recent,
      state.sessionTitle,
      state.status,
      workspace?.open ?? _noPage,
    ]),
    builder: (context, _) {
      final open = workspace?.open.value == null;
      final current = state.session?.id;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final thread in state.recent.value)
            _SessionRow(
              state: state,
              thread: thread,
              selected: open && thread.id == current,
              status: thread.id == current
                  ? state.status.value
                  : thread.running
                  ? RunStatus.running
                  : RunStatus.idle,
              onShow: onShow,
            ),
        ],
      );
    },
  );
}

/// One session of the project: its status or last change, and the menu for export, archive and
/// delete.
class _SessionRow extends StatelessWidget {
  const _SessionRow({
    required this.state,
    required this.thread,
    required this.selected,
    required this.status,
    required this.onShow,
  });

  final SessionState state;
  final BridgeThreadSummary thread;
  final bool selected;
  final RunStatus status;
  final VoidCallback? onShow;

  /// The status word and its colour: the status while one runs, else when the session last changed.
  (String, Color) _label(BuildContext context) {
    final ws = context.ws;
    if (thread.archived) return ('archived', ws.textFaint);
    if (status != RunStatus.idle) return statusLabel(context, status);
    final updated = thread.updatedMs;
    return (
      updated == null ? '' : formatAgo(updated, DateTime.now()),
      ws.textFaint,
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (text, color) = _label(context);
    return RowMenu(
      actions: [
        if (selected)
          RowAction('Export record', () => exportSessionRecord(state)),
        RowAction(
          thread.archived ? 'Unarchive' : 'Archive thread',
          () => unawaited(state.archive(thread.id, archived: !thread.archived)),
          // T3 does not archive a running thread.
          enabled: !thread.running,
        ),
        null,
        RowAction(
          'Delete',
          () => unawaited(confirmDeleteSession(context, state, thread)),
          danger: true,
        ),
      ],
      // Marked while it shows rather than a Page.
      builder: (context, menu, shown) => Semantics(
        selected: selected,
        child: Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            key: Key('session-${thread.id}'),
            onTap: () {
              if (state.session?.id != thread.id) {
                unawaited(state.open(thread.id));
              }
              onShow?.call();
            },
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            child: Ink(
              decoration: BoxDecoration(
                color: selected ? ws.hover : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(
                  WsMetrics.space6,
                  WsMetrics.space2,
                  WsMetrics.space3,
                  WsMetrics.space2,
                ),
                child: Column(
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
                        Expanded(
                          child: Text(
                            thread.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        // The "···" takes the status word's place while it shows; the dot keeps
                        // the status.
                        Stack(
                          alignment: Alignment.centerRight,
                          children: [
                            Opacity(
                              opacity: shown ? 0 : 1,
                              child: Text(
                                text,
                                style: TextStyle(
                                  color: color,
                                  fontSize: WsMetrics.fsSmall,
                                ),
                              ),
                            ),
                            menu,
                          ],
                        ),
                      ],
                    ),
                    if (thread.branch case final branch?)
                      Padding(
                        padding: const EdgeInsets.only(left: 15),
                        child: Text(
                          branch,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            color: ws.textFaint,
                            fontSize: WsMetrics.fsSmall,
                            height: WsMetrics.lhSmall / WsMetrics.fsSmall,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
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
  };
}
