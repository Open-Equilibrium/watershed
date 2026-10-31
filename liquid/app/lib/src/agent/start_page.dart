import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'input_lane.dart';
import 'sidebar.dart';

/// The device a local session runs on, as the start page's device chip names it.
final localDevice = Platform.isMacOS ? 'This Mac' : 'This computer';

/// The main area of a session without a conversation yet (approved start page mockup):
/// the project's recent sessions to continue; the input area below starts a new one.
class StartPage extends StatelessWidget {
  const StartPage({super.key, required this.state, required this.info});

  final SessionState state;
  final SessionInfo info;

  /// What each approval mode means, as the greeting says it.
  static const _modes = {
    BridgeApprovalMode.ask: 'Ask before changes',
    BridgeApprovalMode.allowEdits: 'Edits without asking',
    BridgeApprovalMode.fullAccess: 'Full access',
  };

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Column(
      children: [
        const SizedBox(height: WsMetrics.space8),
        // A new session greets with its project, as in T3 Code (approved look mockup).
        Text(
          'What should we build in ${info.project}?',
          textAlign: TextAlign.center,
          style: Theme.of(context).textTheme.titleLarge
              ?.copyWith(fontWeight: WsMetrics.fwSemibold),
        ),
        const SizedBox(height: WsMetrics.space2),
        ValueListenableBuilder(
          valueListenable: state.mode,
          builder: (context, mode, _) => Text(
            '${info.harness} on ${info.device} · ${_modes[mode]}',
            style: faint,
          ),
        ),
        // The list sits above the input area and scrolls when the window is short.
        Expanded(
          child: Align(
            alignment: Alignment.bottomCenter,
            child: ValueListenableBuilder(
              valueListenable: state.recent,
              builder: (context, recent, _) => recent.isEmpty
                  ? const SizedBox.shrink()
                  : SingleChildScrollView(
                      child: SizedBox(
                        width: readingWidth,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: WsMetrics.space3,
                                vertical: WsMetrics.space2,
                              ),
                              child: Text(
                                'Recent in ${info.project}',
                                style: faint,
                              ),
                            ),
                            for (final session in recent)
                              _RecentRow(session: session, state: state),
                          ],
                        ),
                      ),
                    ),
            ),
          ),
        ),
        const SizedBox(height: inputClearance),
      ],
    );
  }
}

class _RecentRow extends StatelessWidget {
  const _RecentRow({required this.session, required this.state});

  final BridgeStoredSession session;
  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      onTap: () => state.continueSession(session.path),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      child: Container(
        height: 30,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        child: Row(
          children: [
            Icon(LucideIcons.clock, size: 14, color: ws.textFaint),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Text(
                session.title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            Text(
              'continue · ${ago(session.modifiedMs)}',
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// How long ago a Unix millisecond time was, as the start page shows it.
String ago(int millis) {
  final elapsed = DateTime.now().difference(
    DateTime.fromMillisecondsSinceEpoch(millis),
  );
  if (elapsed.inMinutes < 1) return 'now';
  if (elapsed.inHours < 1) return '${elapsed.inMinutes} min';
  if (elapsed.inDays < 1) return '${elapsed.inHours} h';
  return '${elapsed.inDays} d';
}

/// The chips that set up a new session in the input area: project, device and model.
class StartChips extends StatelessWidget {
  const StartChips({super.key, required this.state, required this.project});

  final SessionState state;
  final String project;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Flexible(
        child: _Chip(icon: LucideIcons.hexagon, label: project),
      ),
      const SizedBox(width: WsMetrics.space2),
      Flexible(
        child: _Chip(icon: LucideIcons.laptop, label: localDevice),
      ),
      const SizedBox(width: WsMetrics.space2),
      Flexible(
        child: ListenableBuilder(
          listenable: Listenable.merge([state.models, state.model]),
          builder: (context, _) {
            final models = state.models.value;
            final current = models
                .where(
                  (model) =>
                      '${model.provider}/${model.id}' == state.model.value,
                )
                .firstOrNull;
            final label = current?.name ?? state.model.value ?? 'Model';
            return PopupMenuButton<BridgeModel>(
              tooltip: 'Model',
              onSelected: state.setModel,
              itemBuilder: (context) => [
                for (final model in models)
                  PopupMenuItem(value: model, child: Text(model.name)),
              ],
              child: _Chip(
                icon: LucideIcons.sparkles,
                label: label,
                menu: true,
              ),
            );
          },
        ),
      ),
    ],
  );
}

class _Chip extends StatelessWidget {
  const _Chip({required this.icon, required this.label, this.menu = false});

  final IconData icon;
  final String label;
  final bool menu;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 28,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        border: Border.all(color: ws.borderStrong),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 13, color: ws.textMuted),
          const SizedBox(width: WsMetrics.space1),
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsCompact,
              ),
            ),
          ),
          if (menu) ...[
            const SizedBox(width: WsMetrics.space1),
            Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
          ],
        ],
      ),
    );
  }
}
