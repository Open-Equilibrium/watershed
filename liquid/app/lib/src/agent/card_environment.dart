import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import 'session_card_rows.dart';

class EnvironmentRow extends StatelessWidget {
  const EnvironmentRow({super.key, required this.state, required this.host});

  final SessionState state;
  final String host;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.recent,
    builder: (context, threads, _) {
      final ws = context.ws;
      final path = threads
          .where((listed) => listed.thread.id == state.session?.id)
          .firstOrNull
          ?.thread
          .worktree;
      final row = CardRow(
        key: const Key('workspace-row'),
        children: [
          Icon(LucideIcons.server, size: 16, color: ws.textFaint),
          Flexible(
            child: Text(host, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          Tooltip(
            message: path ?? 'Local checkout',
            child: Icon(
              path == null ? LucideIcons.folder : LucideIcons.folderGit,
              size: 16,
              color: ws.textFaint,
            ),
          ),
        ],
      );
      if (path == null) return row;
      return GestureDetector(
        onSecondaryTapUp: (details) =>
            _copyMenu(context, details.globalPosition, path),
        child: row,
      );
    },
  );

  Future<void> _copyMenu(BuildContext context, Offset at, String path) async {
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final copy = await showMenu<bool>(
      context: context,
      position: RelativeRect.fromRect(
        at & Size.zero,
        Offset.zero & overlay.size,
      ),
      items: const [
        PopupMenuItem(value: true, height: 32, child: Text('Copy full path')),
      ],
    );
    if (copy != true) return;
    await Clipboard.setData(ClipboardData(text: path));
    state.notify(NoticeLevel.info, 'Path copied: $path');
  }
}
