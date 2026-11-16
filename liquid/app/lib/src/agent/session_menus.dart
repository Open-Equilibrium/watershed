import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
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
    this.enabled = true,
    this.shortcut,
  });

  final String label;
  final VoidCallback onSelected;
  final bool danger;

  /// Whether the item can be chosen; a disabled item stays listed.
  final bool enabled;
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
              enabled: action.enabled,
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

/// Asks before deleting `thread`, as T3 Code's delete does. When it is the only thread in its
/// worktree, T3 then asks whether the worktree goes too; Cancel keeps the worktree and still
/// deletes the thread.
Future<void> confirmDeleteSession(
  BuildContext context,
  SessionState state,
  BridgeThreadSummary thread,
) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => DangerDialog(
      title: 'Delete thread "${thread.title}"?',
      action: 'Confirm',
      children: [
        Text(
          'This permanently clears conversation history for this thread.',
          style: DangerDialog.text(context),
        ),
      ],
    ),
  );
  if (confirmed != true || !context.mounted) return;
  final worktree = thread.worktree;
  var deleteWorktree = false;
  var only = false;
  try {
    only = worktree != null && await state.onlySessionInWorktree(thread.id);
  } on Object catch (error) {
    state.notify(NoticeLevel.error, 'Could not delete the session: $error');
    return;
  }
  if (only) {
    if (!context.mounted) return;
    deleteWorktree =
        await showDialog<bool>(
          context: context,
          builder: (context) => DangerDialog(
            title: 'Delete the worktree too?',
            action: 'Confirm',
            children: [
              Text(
                'This thread is the only one linked to this worktree:\n$worktree',
                style: DangerDialog.text(context),
              ),
            ],
          ),
        ) ??
        false;
  }
  await state.delete(thread.id, worktree: deleteWorktree);
}

/// Writes the open session's record into Downloads and shows the new folder in the system's file
/// manager; when nothing was written, nothing is shown.
Future<void> exportSessionRecord(SessionState state) async {
  final folder = await state.exportRecord(
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

/// Opens a page in the system's browser: an HTTPS page, or a server on this computer, which the
/// session card's servers are; a failure is reported.
Future<void> openInBrowser(SessionState state, String url) async {
  if (!url.startsWith('https://') && !_isLoopbackHttp(url)) {
    state.notify(NoticeLevel.error, 'Could not open $url: not an HTTPS page.');
    return;
  }
  await _openWithSystem(state, url);
}

/// Whether `url` is plain HTTP to this computer.
bool _isLoopbackHttp(String url) {
  final uri = Uri.tryParse(url);
  return uri != null &&
      uri.scheme == 'http' &&
      const {'localhost', '127.0.0.1', '::1'}.contains(uri.host);
}

/// Opens `target` with the system's default application; a failure is reported.
Future<void> _openWithSystem(SessionState state, String target) async {
  try {
    final result = await runOpener(Platform.isMacOS ? 'open' : 'xdg-open', [
      target,
    ]);
    if (result.exitCode != 0) {
      state.notify(
        NoticeLevel.error,
        'Could not open $target: ${result.stderr}',
      );
    }
  } on ProcessException catch (error) {
    state.notify(NoticeLevel.error, 'Could not open $target: ${error.message}');
  }
}
