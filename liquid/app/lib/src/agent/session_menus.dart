import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// One item of a row's menu.
class RowAction {
  const RowAction(
    this.label,
    this.onSelected, {
    this.danger = false,
    this.shortcut,
  });

  final String label;
  final VoidCallback onSelected;
  final bool danger;
  final String? shortcut;
}

/// A row whose menu opens on right-click or from the "···" that shows while the pointer is
/// over it, as T3 Code's thread list and Notion's pages do (approved checkpoint mockups).
class RowMenu extends StatefulWidget {
  const RowMenu({super.key, required this.actions, required this.child});

  /// The menu's items, groups split by `null`.
  final List<RowAction?> actions;
  final Widget child;

  @override
  State<RowMenu> createState() => _RowMenuState();
}

class _RowMenuState extends State<RowMenu> {
  bool _hovered = false;
  bool _focused = false;

  Future<void> _open(Offset at) async {
    final ws = context.ws;
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final chosen = await showMenu<RowAction>(
      context: context,
      position: RelativeRect.fromRect(
        at & Size.zero,
        Offset.zero & overlay.size,
      ),
      items: [
        for (final action in widget.actions)
          if (action == null)
            const PopupMenuDivider()
          else
            PopupMenuItem(
              value: action,
              height: 32,
              child: Row(
                children: [
                  Flexible(
                    child: Text(
                      action.label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: action.danger ? ws.danger : ws.text,
                      ),
                    ),
                  ),
                  if (action.shortcut case final keys?
                      when keys.isNotEmpty) ...[
                    const SizedBox(width: WsMetrics.space5),
                    Text(
                      keys,
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ],
                ],
              ),
            ),
      ],
    );
    chosen?.onSelected();
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
    onEnter: (_) => setState(() => _hovered = true),
    onExit: (_) => setState(() => _hovered = false),
    child: GestureDetector(
      onSecondaryTapUp: (details) => _open(details.globalPosition),
      child: Row(
        children: [
          Expanded(child: widget.child),
          // Always there for the keyboard and screen readers; seen on hover or focus.
          Opacity(
            opacity: _hovered || _focused ? 1 : 0,
            child: Tooltip(
              message: 'More actions',
              child: Builder(
                builder: (context) => InkWell(
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                  onFocusChange: (focused) =>
                      setState(() => _focused = focused),
                  onTap: () {
                    final box = context.findRenderObject()! as RenderBox;
                    _open(box.localToGlobal(box.size.bottomLeft(Offset.zero)));
                  },
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space2,
                    ),
                    child: Icon(
                      LucideIcons.ellipsis,
                      size: 16,
                      color: context.ws.textMuted,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

/// Asks before deleting the session stored at `path`, named `title`, with its checkpoints,
/// then deletes it (approved checkpoint mockups).
Future<void> confirmDeleteSession(
  BuildContext context,
  SessionState state, {
  required String path,
  required String title,
}) async {
  final int turns;
  try {
    turns = await state.storedTurns(path);
  } on Object catch (error) {
    state.notify(BridgeLevel.error, 'Could not read the session: $error');
    return;
  }
  if (!context.mounted) return;
  final checkpoints = switch (turns) {
    0 => 'Its pi session file is deleted',
    1 => 'Its pi session file and its turn’s checkpoints are deleted',
    final count =>
      'Its pi session file and its $count turns’ checkpoints are deleted',
  };
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => DangerDialog(
      title: 'Delete “$title”?',
      action: 'Delete session',
      children: [
        Text(
          '$checkpoints and cannot be restored. Project files, commits and branches stay.',
          style: DangerDialog.text(context),
        ),
      ],
    ),
  );
  if (confirmed == true) await state.deleteSession(path);
}

/// Asks before removing every checkpoint of `project`, listing the sessions that lose them,
/// then removes them (approved checkpoint mockups).
Future<void> confirmRemoveCheckpoints(
  BuildContext context,
  SessionState state,
  String project,
) async {
  final List<BridgeCheckpointSession> sessions;
  try {
    sessions = await state.checkpointSessions();
  } on Object catch (error) {
    state.notify(BridgeLevel.error, 'Could not read the checkpoints: $error');
    return;
  }
  if (!context.mounted) return;
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) {
      final ws = context.ws;
      final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
      return DangerDialog(
        title: 'Remove all checkpoints of $project?',
        action: sessions.isEmpty ? null : 'Remove checkpoints',
        children: [
          Text(switch (sessions.length) {
            0 => 'No session has checkpoints in this project.',
            1 =>
              '1 session keeps its chat and Trajectory, but no longer shows turn changes '
                  'or reverts files. Project files, commits and branches stay. Git frees the '
                  'space at its next cleanup.',
            final count =>
              '$count sessions keep their chats and Trajectories, but no longer show turn '
                  'changes or revert files. Project files, commits and branches stay. Git '
                  'frees the space at its next cleanup.',
          }, style: DangerDialog.text(context)),
          if (sessions.isNotEmpty)
            Container(
              constraints: const BoxConstraints(maxHeight: 240),
              padding: const EdgeInsets.all(WsMetrics.space2),
              decoration: BoxDecoration(
                color: ws.subtle,
                borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              ),
              child: ListView(
                shrinkWrap: true,
                children: [
                  for (final session in sessions)
                    SizedBox(
                      height: 26,
                      child: Padding(
                        padding: const EdgeInsets.symmetric(
                          horizontal: WsMetrics.space2,
                        ),
                        child: Row(
                          children: [
                            Expanded(
                              child: Text(
                                session.title,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                              ),
                            ),
                            Text(
                              session.turns == 1
                                  ? '1 turn'
                                  : '${session.turns} turns',
                              style: faint,
                            ),
                          ],
                        ),
                      ),
                    ),
                ],
              ),
            ),
        ],
      );
    },
  );
  if (confirmed == true) await state.removeAllCheckpoints();
}

/// Shows the folder at `path` in the system's file manager; a failure is reported.
Future<void> openInFileManager(SessionState state, String path) async {
  try {
    final result = await Process.run(Platform.isMacOS ? 'open' : 'xdg-open', [
      path,
    ]);
    if (result.exitCode != 0) {
      state.notify(BridgeLevel.error, 'Could not open $path: ${result.stderr}');
    }
  } on ProcessException catch (error) {
    state.notify(BridgeLevel.error, 'Could not open $path: ${error.message}');
  }
}
