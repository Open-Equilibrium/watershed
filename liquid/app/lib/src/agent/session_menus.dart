import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

class RowAction {
  const RowAction(
    this.label,
    this.onSelected, {
    this.danger = false,
    this.enabled = true,
    this.shortcut,
    this.disabledReason,
  });

  final String label;
  final VoidCallback onSelected;
  final bool danger;

  final bool enabled;
  final String? shortcut;

  final String? disabledReason;
}

class RowMenu extends StatefulWidget {
  const RowMenu({super.key, required this.actions, required this.builder});

  final List<RowAction?> actions;

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
              child: _withReason(
                action,
                Row(
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

Future<void> confirmDeleteSession(
  BuildContext context,
  SessionState state,
  HostThread listed,
) async {
  final thread = listed.thread;
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
    only =
        worktree != null &&
        await state.onlySessionInWorktree(thread.id, address: listed.address);
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
  await state.delete(
    thread.id,
    worktree: deleteWorktree,
    address: listed.address,
  );
}

Future<void> exportSessionRecord(SessionState state) async {
  final folder = await state.exportRecord(
    host: Platform.localHostname,
    day: _localDay(DateTime.now()),
  );
  if (folder != null) await openInFileManager(state, folder);
}

String _localDay(DateTime now) =>
    '${now.year.toString().padLeft(4, '0')}-'
    '${now.month.toString().padLeft(2, '0')}-'
    '${now.day.toString().padLeft(2, '0')}';

@visibleForTesting
Future<ProcessResult> Function(String program, List<String> arguments)
runOpener = Process.run;

Future<void> openInFileManager(SessionState state, String path) =>
    _openWithSystem(state, path);

Future<void> openInBrowser(SessionState state, String url) async {
  if (!url.startsWith('https://') && !_isLoopbackHttp(url)) {
    state.notify(NoticeLevel.error, 'Could not open $url: not an HTTPS page.');
    return;
  }
  await _openWithSystem(state, url);
}

bool _isLoopbackHttp(String url) {
  final uri = Uri.tryParse(url);
  return uri != null &&
      uri.scheme == 'http' &&
      const {'localhost', '127.0.0.1', '::1'}.contains(uri.host);
}

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

Widget _withReason(RowAction action, Widget child) {
  final reason = action.disabledReason;
  return reason == null || action.enabled
      ? child
      : Tooltip(message: reason, child: child);
}
