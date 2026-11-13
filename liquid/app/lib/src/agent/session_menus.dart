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

/// A row whose menu opens on right-click or from the "···" that shows inside the row while the
/// pointer is over it or the keyboard is on the button, as T3 Code's thread list and Notion's
/// pages do.
class RowMenu extends StatefulWidget {
  const RowMenu({super.key, required this.actions, required this.builder});

  /// The menu's items, groups split by `null`.
  final List<RowAction?> actions;

  /// Lays out the row. `menu` is the "···", which the row places at its right end inside its
  /// highlight; `shown` says whether it is seen now, for a row that gives it another's place.
  final Widget Function(BuildContext context, Widget menu, bool shown) builder;

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
  Widget build(BuildContext context) {
    final shown = _hovered || _focused;
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        onSecondaryTapUp: (details) => _open(details.globalPosition),
        child: widget.builder(context, _menu(shown), shown),
      ),
    );
  }

  /// The "···" button: always there for the keyboard and screen readers, seen while `shown`.
  Widget _menu(bool shown) => Opacity(
    opacity: shown ? 1 : 0,
    alwaysIncludeSemantics: true,
    child: Semantics(
      label: 'More actions',
      button: true,
      child: Tooltip(
        message: 'More actions',
        excludeFromSemantics: true,
        child: Builder(
          builder: (context) => InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
            onFocusChange: (focused) => setState(() => _focused = focused),
            onTap: () {
              final box = context.findRenderObject()! as RenderBox;
              _open(box.localToGlobal(box.size.bottomLeft(Offset.zero)));
            },
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
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
  );
}

/// Asks before deleting the session stored at `path`, named `title`, with its checkpoints,
/// then deletes it.
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
  // A question that arrived meanwhile takes the keys, so no dialog opens over it.
  if (!context.mounted || state.asking) return;
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
/// then removes them.
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
  // A question that arrived meanwhile takes the keys, so no dialog opens over it.
  if (!context.mounted || state.asking) return;
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
              '1 session keeps its chat, but no longer shows turn changes '
                  'or reverts files. Project files, commits and branches stay. Git frees the '
                  'space at its next cleanup.',
            final count =>
              '$count sessions keep their chats, but no longer show turn '
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

/// Writes the record of the session stored at `path`, named `title`, into Downloads and shows the
/// new folder in the system's file manager; when nothing was written, nothing is shown.
Future<void> exportSessionRecord(
  SessionState state, {
  required String path,
  required String title,
}) async {
  final folder = await state.exportRecord(
    path: path,
    title: title,
    host: Platform.localHostname,
    day: _localDay(DateTime.now()),
  );
  if (folder != null) await openInFileManager(state, folder);
}

/// The local date as `YYYY-MM-DD`, which names a record's folder.
String _localDay(DateTime now) =>
    '${now.year.toString().padLeft(4, '0')}-'
    '${now.month.toString().padLeft(2, '0')}-'
    '${now.day.toString().padLeft(2, '0')}';

/// Starts the system's opener, `open` on macOS and `xdg-open` elsewhere. Tests replace it.
@visibleForTesting
Future<ProcessResult> Function(String program, List<String> arguments)
runOpener = Process.run;

/// Shows the folder at `path` in the system's file manager; a failure is reported.
Future<void> openInFileManager(SessionState state, String path) =>
    _openWithSystem(state, path);

/// Opens a GitHub page in the system's browser; only HTTPS pages, and a failure is reported.
Future<void> openInBrowser(SessionState state, String url) async {
  if (!url.startsWith('https://')) {
    state.notify(BridgeLevel.error, 'Could not open $url: not an HTTPS page.');
    return;
  }
  await _openWithSystem(state, url);
}

/// Opens `target` with the system's default application; a failure is reported.
Future<void> _openWithSystem(SessionState state, String target) async {
  try {
    final result = await runOpener(Platform.isMacOS ? 'open' : 'xdg-open', [
      target,
    ]);
    if (result.exitCode != 0) {
      state.notify(
        BridgeLevel.error,
        'Could not open $target: ${result.stderr}',
      );
    }
  } on ProcessException catch (error) {
    state.notify(BridgeLevel.error, 'Could not open $target: ${error.message}');
  }
}
