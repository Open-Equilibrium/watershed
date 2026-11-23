import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'workspace_state.dart';

class Notices extends StatelessWidget {
  const Notices({super.key, required this.state});

  final WorkspaceState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: state.notices,
      builder: (context, notices, _) => Material(
        type: MaterialType.transparency,
        child: Align(
          alignment: Alignment.bottomCenter,
          child: Padding(
            padding: const EdgeInsets.only(bottom: WsMetrics.space6),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              spacing: WsMetrics.space2,
              children: [
                for (final notice in notices)
                  Container(
                    key: ValueKey(notice.id),
                    constraints: const BoxConstraints(maxWidth: 560),
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space5,
                      vertical: WsMetrics.space3,
                    ),
                    decoration: BoxDecoration(
                      color: ws.text,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                      boxShadow: ws.shadow,
                    ),
                    child: Semantics(
                      liveRegion: true,
                      child: Text(
                        notice.text,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.canvas,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
