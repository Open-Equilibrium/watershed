import 'package:flutter/material.dart';

import '../controls.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/logo.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// What the sidebar shows about a session.
@immutable
class SessionInfo {
  const SessionInfo({
    required this.project,
    required this.harness,
    required this.device,
  });

  final String project;
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

/// Width of the sidebar at the default text size (approved look mockup).
const sidebarWidth = 280.0;

/// Session navigation with project, harness and device per session, and Settings in its
/// footer (approved look mockup).
class Sidebar extends StatelessWidget {
  const Sidebar({
    super.key,
    required this.state,
    required this.info,
    this.onSettings,
  });

  final SessionState state;
  final SessionInfo info;
  final VoidCallback? onSettings;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      color: ws.textFaint,
      fontSize: WsMetrics.fsSmall,
      height: WsMetrics.lhSmall / WsMetrics.fsSmall,
    );
    return Container(
      width: sidebarWidth,
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
          const SizedBox(height: WsMetrics.space5),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            child: Row(children: [Text('Agent sessions', style: small)]),
          ),
          const SizedBox(height: WsMetrics.space2),
          Padding(
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
          Container(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space6,
              WsMetrics.space2,
              WsMetrics.space3,
              WsMetrics.space2,
            ),
            decoration: BoxDecoration(
              color: ws.hover,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
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
                      child: Text(
                        [info.harness, info.device].join(' · '),
                        style: small,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ],
                );
              },
            ),
          ),
          const Spacer(),
          if (onSettings case final onSettings?)
            Tooltip(
              message: 'Settings',
              child: Shortcuts(
                shortcuts: activateKeys,
                child: InkWell(
                  onTap: onSettings,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  child: Container(
                    height: WsMetrics.rowNav,
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
