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
import '../theme/machine_icon.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/workspace_switcher.dart';
import '../workspace/workspaces.dart';
import '../workspace/explorer.dart';
import '../workspace/workspace_state.dart';
import 'session_menus.dart';

@immutable
class SessionInfo {
  const SessionInfo({required this.project, this.path});

  final String project;

  final String? path;
}

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

final _noPage = ValueNotifier<OpenPage?>(null);

class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    this.state,
    this.info,
    this.workspace,
    this.workspaceProblem,
    this.workspaces,
    this.agentProblem,
    this.onSettings,
    this.onSearch,
    this.onNewSession,
    this.onShowSession,
    this.onMove,
  });

  final SessionState? state;
  final SessionInfo? info;

  final WorkspaceState? workspace;
  final String? workspaceProblem;

  final Workspaces? workspaces;

  final String? agentProblem;
  final VoidCallback? onSettings;

  final VoidCallback? onSearch;

  final VoidCallback? onNewSession;

  final VoidCallback? onShowSession;

  final void Function(HostThread listed)? onMove;

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
          if (workspace != null || workspaces != null)
            WorkspaceSwitcher(
              key: ValueKey((workspace, workspaces)),
              state: workspace,
              workspaces: workspaces,
            )
          else
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
          if (onSearch case final search?)
            InkWell(
              key: const Key('sidebar search'),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: search,
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                  vertical: WsMetrics.space2,
                ),
                child: Row(
                  children: [
                    Icon(LucideIcons.search, size: 14, color: ws.textFaint),
                    const SizedBox(width: WsMetrics.space2),
                    Expanded(
                      child: Text(
                        'Search or run a command',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.textMuted,
                        ),
                      ),
                    ),
                    Text(
                      Keymap.of(context).label(Command.powerBar),
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.textFaint,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          Expanded(
            child: ListView(
              children: [
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
                      if (state case final state?)
                        ListenableBuilder(
                          listenable: state.hostProblems,
                          builder: (context, _) => Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              for (final problem
                                  in state.hostProblems.value.values)
                                Padding(
                                  padding: const EdgeInsets.only(
                                    top: WsMetrics.space3,
                                  ),
                                  child: WarningNotice(problem),
                                ),
                            ],
                          ),
                        ),
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
                          onMove: onMove,
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

class _Sessions extends StatelessWidget {
  const _Sessions({
    required this.state,
    required this.workspace,
    required this.onShow,
    this.onMove,
  });

  final SessionState state;
  final WorkspaceState? workspace;
  final VoidCallback? onShow;
  final void Function(HostThread listed)? onMove;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.recent,
      state.sessionTitle,
      state.status,
      state.model,
      workspace?.open ?? _noPage,
    ]),
    builder: (context, _) {
      final open = workspace?.open.value == null;
      final current = state.session?.id;
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final listed in state.recent.value)
            _SessionRow(
              state: state,
              listed: listed,
              selected:
                  open &&
                  listed.thread.id == current &&
                  listed.address == state.sessionAddress,
              status:
                  listed.thread.id == current &&
                      listed.address == state.sessionAddress
                  ? state.status.value
                  : listed.thread.running
                  ? RunStatus.running
                  : RunStatus.idle,
              onShow: onShow,
              onMove: onMove,
            ),
        ],
      );
    },
  );
}

class _SessionRow extends StatelessWidget {
  const _SessionRow({
    required this.state,
    required this.listed,
    required this.selected,
    required this.status,
    required this.onShow,
    this.onMove,
  });

  final SessionState state;
  final HostThread listed;
  final bool selected;
  final RunStatus status;
  final VoidCallback? onShow;
  final void Function(HostThread listed)? onMove;

  BridgeThreadSummary get thread => listed.thread;

  (String, Color) _label(BuildContext context) {
    final ws = context.ws;
    if (thread.archived) return ('archived', ws.textFaint);
    if (thread.moving != null) return ('Moving', ws.textFaint);
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
          'Move to host…',
          () => onMove?.call(listed),
          enabled: listed.thread.movable,
          disabledReason: listed.thread.harnessKnown
              ? 'Only Claude Code and Codex sessions can move.'
              : 'This host needs a newer Meta-Harness',
        ),
        RowAction(
          thread.archived ? 'Unarchive' : 'Archive thread',
          () => unawaited(
            state.archive(
              thread.id,
              address: listed.address,
              archived: !thread.archived,
            ),
          ),
          enabled: !thread.running,
        ),
        null,
        RowAction(
          'Delete',
          () => unawaited(confirmDeleteSession(context, state, listed)),
          danger: true,
        ),
      ],
      builder: (context, menu, shown) => Semantics(
        selected: selected,
        child: Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            key: Key('session-${thread.id}'),
            onTap: () {
              if (state.session?.id != thread.id ||
                  state.sessionAddress != listed.address) {
                unawaited(state.open(thread.id, address: listed.address));
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
                        if (listed.machine case final kind?) ...[
                          MachineIcon(
                            kind,
                            name: state.hostNameOf(listed.address),
                          ),
                          const SizedBox(width: WsMetrics.space2),
                        ],
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

(String, Color) statusLabel(BuildContext context, RunStatus status) {
  final ws = context.ws;
  return switch (status) {
    RunStatus.idle => ('idle', ws.textFaint),
    RunStatus.running => ('running', ws.accent),
    RunStatus.waitingForYou => ('waiting', ws.warning),
  };
}
