import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'action_area.dart';
import 'chat_view.dart';

/// What the sidebar shows about a session.
@immutable
class SessionInfo {
  const SessionInfo({
    required this.title,
    required this.project,
    required this.harness,
    required this.device,
    this.branch,
  });

  final String title;
  final String project;
  final String harness;
  final String device;
  final String? branch;
}

enum AgentTab { chat, trajectory, diff }

/// The agent surface: sidebar, tabs, the active tab, the widget strip, the input area and the
/// status bar. Every part keeps its position in every tab (guidelines, principle 8).
class AgentShell extends StatefulWidget {
  const AgentShell({super.key, required this.state, required this.info});

  final SessionState state;
  final SessionInfo info;

  @override
  State<AgentShell> createState() => _AgentShellState();
}

class _AgentShellState extends State<AgentShell> {
  final _composer = TextEditingController();
  final _composerFocus = FocusNode(debugLabel: 'composer');
  final _tab = ValueNotifier(AgentTab.chat);

  @override
  void initState() {
    super.initState();
    widget.state.draft.addListener(_applyDraft);
  }

  void _applyDraft() {
    final draft = widget.state.draft.value;
    if (draft != null) _composer.text = draft;
  }

  @override
  void dispose() {
    widget.state.draft.removeListener(_applyDraft);
    _composer.dispose();
    _composerFocus.dispose();
    _tab.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.digit1, meta: true): () =>
            _tab.value = AgentTab.chat,
        const SingleActivator(LogicalKeyboardKey.digit2, meta: true): () =>
            _tab.value = AgentTab.trajectory,
        const SingleActivator(LogicalKeyboardKey.digit3, meta: true): () =>
            _tab.value = AgentTab.diff,
      },
      child: Focus(
        autofocus: true,
        child: Material(
          color: ws.canvas,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Sidebar(state: widget.state, info: widget.info),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _Header(title: widget.info.title),
                    _Tabs(tab: _tab),
                    Expanded(
                      child: ValueListenableBuilder(
                        valueListenable: _tab,
                        builder: (context, tab, _) => switch (tab) {
                          AgentTab.chat => ChatView(state: widget.state),
                          AgentTab.trajectory ||
                          AgentTab.diff => const _NotBuiltYet(),
                        },
                      ),
                    ),
                    WidgetStrip(state: widget.state),
                    Padding(
                      padding: const EdgeInsets.only(bottom: WsMetrics.space4),
                      child: Center(
                        child: SizedBox(
                          width: readingWidth,
                          child: ActionArea(
                            state: widget.state,
                            composer: _composer,
                          ),
                        ),
                      ),
                    ),
                    StatusBar(state: widget.state),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _NotBuiltYet extends StatelessWidget {
  const _NotBuiltYet();

  @override
  Widget build(BuildContext context) => Center(
    child: Text(
      'Coming in this slice',
      style: Theme.of(context).textTheme.bodySmall,
    ),
  );
}

class _Header extends StatelessWidget {
  const _Header({required this.title});

  final String title;

  @override
  Widget build(BuildContext context) => Container(
    height: 44,
    padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space7),
    alignment: Alignment.centerLeft,
    child: Text(
      title,
      style: Theme.of(context).textTheme.labelMedium
          ?.copyWith(fontWeight: WsMetrics.fwSemibold),
    ),
  );
}

class _Tabs extends StatelessWidget {
  const _Tabs({required this.tab});

  final ValueNotifier<AgentTab> tab;

  static const _labels = {
    AgentTab.chat: ('Chat', '⌘1'),
    AgentTab.trajectory: ('Trajectory', '⌘2'),
    AgentTab.diff: ('Diff', '⌘3'),
  };

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 38,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space6),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: ValueListenableBuilder(
        valueListenable: tab,
        builder: (context, active, _) => Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final entry in _labels.entries)
              InkWell(
                canRequestFocus: false,
                onTap: () => tab.value = entry.key,
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                  ),
                  decoration: BoxDecoration(
                    border: Border(
                      bottom: BorderSide(
                        color: entry.key == active
                            ? ws.text
                            : Colors.transparent,
                        width: 2,
                      ),
                    ),
                  ),
                  child: Row(
                    children: [
                      Text(
                        entry.value.$1,
                        style: TextStyle(
                          color: entry.key == active ? ws.text : ws.textMuted,
                          fontWeight: entry.key == active
                              ? WsMetrics.fwMedium
                              : WsMetrics.fwRegular,
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space2),
                      Hint(entry.value.$2),
                    ],
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// Session navigation with project, harness, branch and device per session.
class Sidebar extends StatelessWidget {
  const Sidebar({super.key, required this.state, required this.info});

  final SessionState state;
  final SessionInfo info;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      color: ws.textFaint,
      fontSize: WsMetrics.fsSmall,
      height: WsMetrics.lhSmall / WsMetrics.fsSmall,
    );
    return Container(
      width: 264,
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
                Text('Agent sessions', style: small),
                const Spacer(),
                Hint('⌘N'),
              ],
            ),
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
                        Expanded(
                          child: Text(
                            info.title,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
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
                        [
                          info.harness,
                          if (info.branch != null) info.branch!,
                          info.device,
                        ].join(' · '),
                        style: small,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ],
                );
              },
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

/// Extension widgets, directly above the input area.
class WidgetStrip extends StatelessWidget {
  const WidgetStrip({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder(
      valueListenable: state.widgets,
      builder: (context, widgets, _) => SizedBox(
        height: 36,
        child: Center(
          child: SizedBox(
            width: readingWidth,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
              child: Text(
                [
                  for (final entry in widgets.entries)
                    '${entry.key}  ${entry.value.join('  ')}',
                ].join('    '),
                key: const Key('widgets'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: monoStyle(
                  context,
                  color: context.ws.textMuted,
                ).copyWith(fontSize: WsMetrics.fsSmall),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Run status, tokens, cost and extension status entries.
class StatusBar extends StatelessWidget {
  const StatusBar({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final text = TextStyle(
      color: ws.textMuted,
      fontSize: WsMetrics.fsSmall,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    return Container(
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: ws.border)),
      ),
      child: ListenableBuilder(
        listenable: Listenable.merge([
          state.status,
          state.totals,
          state.statuses,
          state.failure,
        ]),
        builder: (context, _) {
          final (label, color) = statusLabel(context, state.status.value);
          final totals = state.totals.value;
          final failure = state.failure.value;
          return Row(
            children: [
              Text(
                '● ${failure ?? label}',
                key: const Key('status'),
                style: text.copyWith(color: color),
              ),
              const SizedBox(width: WsMetrics.space6),
              Text(
                '${_tokens(totals.input)} in · ${_tokens(totals.output)} out',
                style: text,
              ),
              const SizedBox(width: WsMetrics.space6),
              Text(
                '≈ \$${(totals.costMicroUsd / 1e6).toStringAsFixed(2)}',
                style: text,
              ),
              for (final entry in state.statuses.value.entries) ...[
                const SizedBox(width: WsMetrics.space6),
                Text('${entry.key} ${entry.value}', style: text),
              ],
              const Spacer(),
              Text(
                '⌘1–3 tabs · ⌘L input',
                style: text.copyWith(color: ws.textFaint),
              ),
            ],
          );
        },
      ),
    );
  }

  static String _tokens(int count) =>
      count >= 1000 ? '${(count / 1000).toStringAsFixed(1)}k' : '$count';
}
