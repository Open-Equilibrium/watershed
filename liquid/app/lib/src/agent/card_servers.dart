import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/details.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';
import 'session_menus.dart';

/// The Servers section: the local servers the session's agent or terminals run. A click opens one
/// in the default browser; Stop ends the process that runs it, as T3 Code's row does.
class ServersSection extends StatelessWidget {
  const ServersSection({
    super.key,
    required this.state,
    required this.servers,
    required this.divided,
  });

  final SessionState state;
  final List<BridgeLocalServer> servers;
  final bool divided;

  @override
  Widget build(BuildContext context) => CardSection(
    divided: divided,
    children: [
      const CardHeading(title: 'Servers'),
      PagedRows(
        initial: 4,
        collapsible: true,
        rows: [
          for (final server in servers)
            _ServerRow(state: state, server: server),
        ],
      ),
    ],
  );
}

class _ServerRow extends StatelessWidget {
  const _ServerRow({required this.state, required this.server});

  final SessionState state;
  final BridgeLocalServer server;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final who = server.terminal ? 'Terminal' : 'Agent';
    final process = server.process;
    final pid = server.pid;
    return Tooltip(
      message: 'Open in your browser',
      child: CardRow(
        onTap: () => unawaited(openInBrowser(state, server.url)),
        children: [
          Icon(LucideIcons.globe, size: 16, color: ws.textFaint),
          Flexible(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '${server.host}:${server.port}',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(height: 1.2),
                ),
                Text(
                  process == null ? who : '$who · $process',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCaption,
                    fontWeight: WsMetrics.fwRegular,
                    height: 1.2,
                    color: ws.textMuted,
                  ),
                ),
              ],
            ),
          ),
          if (pid != null)
            CardIconButton(
              icon: LucideIcons.square,
              tooltip: 'Stop server',
              onPressed: () => unawaited(state.stopServer(pid, server.port)),
            ),
        ],
      ),
    );
  }
}
