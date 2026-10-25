import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'action_area.dart';
import 'chat_view.dart';
import 'shortcut_reveal.dart';
import '../diff/diff_view.dart';
import '../trajectory/trajectory_view.dart';

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
  final _trajectoryKeys = ValueNotifier(trajectoryKeys);

  @override
  void initState() {
    super.initState();
    widget.state.draft.addListener(_applyDraft);
    // A reopened Trajectory tab starts outside the comparison.
    _tab.addListener(() => _trajectoryKeys.value = trajectoryKeys);
  }

  void _applyDraft() {
    final draft = widget.state.draft.value;
    if (draft == null) return;
    _composer.value = TextEditingValue(
      text: draft,
      selection: TextSelection.collapsed(offset: draft.length),
    );
    // Taken, so the same text handed back again arrives again.
    widget.state.draft.value = null;
  }

  @override
  void dispose() {
    widget.state.draft.removeListener(_applyDraft);
    _composer.dispose();
    _composerFocus.dispose();
    _tab.dispose();
    _trajectoryKeys.dispose();
    super.dispose();
  }

  /// App-wide shortcuts in the macOS menu bar, where they stay out of the GUI (ADR-0204);
  /// the bindings below handle the same keys on every platform.
  Widget _menus(Widget child) {
    if (defaultTargetPlatform != TargetPlatform.macOS) return child;
    PlatformMenuItem item(
      String label,
      LogicalKeyboardKey key,
      VoidCallback action,
    ) => PlatformMenuItem(
      label: label,
      shortcut: SingleActivator(key, meta: true),
      onSelected: action,
    );
    PlatformMenuItem provided(PlatformProvidedMenuItemType type) =>
        PlatformProvidedMenuItem(type: type);
    return PlatformMenuBar(
      menus: [
        PlatformMenu(
          label: 'Liquid',
          menus: [
            PlatformMenuItemGroup(
              members: [provided(PlatformProvidedMenuItemType.about)],
            ),
            PlatformMenuItemGroup(
              members: [
                provided(PlatformProvidedMenuItemType.hide),
                provided(PlatformProvidedMenuItemType.hideOtherApplications),
                provided(PlatformProvidedMenuItemType.showAllApplications),
              ],
            ),
            provided(PlatformProvidedMenuItemType.quit),
          ],
        ),
        PlatformMenu(
          label: 'View',
          menus: [
            PlatformMenuItemGroup(
              members: [
                item(
                  'Chat',
                  LogicalKeyboardKey.digit1,
                  () => _tab.value = AgentTab.chat,
                ),
                item(
                  'Trajectory',
                  LogicalKeyboardKey.digit2,
                  () => _tab.value = AgentTab.trajectory,
                ),
                item(
                  'Diff',
                  LogicalKeyboardKey.digit3,
                  () => _tab.value = AgentTab.diff,
                ),
              ],
            ),
            PlatformMenuItemGroup(
              members: [
                item(
                  'Focus Input',
                  LogicalKeyboardKey.keyL,
                  _composerFocus.requestFocus,
                ),
              ],
            ),
            provided(PlatformProvidedMenuItemType.toggleFullScreen),
          ],
        ),
        PlatformMenu(
          label: 'Window',
          menus: [
            provided(PlatformProvidedMenuItemType.minimizeWindow),
            provided(PlatformProvidedMenuItemType.zoomWindow),
          ],
        ),
      ],
      child: child,
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return _menus(
      ShortcutReveal(
        child: CallbackShortcuts(
          bindings: {
            const SingleActivator(LogicalKeyboardKey.digit1, meta: true): () =>
                _tab.value = AgentTab.chat,
            const SingleActivator(LogicalKeyboardKey.digit2, meta: true): () =>
                _tab.value = AgentTab.trajectory,
            const SingleActivator(LogicalKeyboardKey.digit3, meta: true): () =>
                _tab.value = AgentTab.diff,
            const SingleActivator(LogicalKeyboardKey.keyL, meta: true):
                _composerFocus.requestFocus,
            const SingleActivator(LogicalKeyboardKey.escape):
                widget.state.dismissNotice,
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
                              AgentTab.diff => DiffView(state: widget.state),
                              AgentTab.trajectory => TrajectoryView(
                                state: widget.state,
                                onBranched: _composerFocus.requestFocus,
                                keys: _trajectoryKeys,
                              ),
                            },
                          ),
                        ),
                        WidgetStrip(state: widget.state),
                        Padding(
                          padding: const EdgeInsets.only(
                            bottom: WsMetrics.space4,
                          ),
                          child: Center(
                            child: SizedBox(
                              width: readingWidth,
                              child: ActionArea(
                                state: widget.state,
                                composer: _composer,
                                composerFocus: _composerFocus,
                              ),
                            ),
                          ),
                        ),
                        ListenableBuilder(
                          listenable: Listenable.merge([_tab, _trajectoryKeys]),
                          builder: (context, _) => StatusBar(
                            state: widget.state,
                            keys: _tab.value == AgentTab.trajectory
                                ? _trajectoryKeys.value
                                : '',
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
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
    AgentTab.chat: 'Chat',
    AgentTab.trajectory: 'Trajectory',
    AgentTab.diff: 'Diff',
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
                  alignment: Alignment.center,
                  child: Text(
                    entry.value,
                    style: TextStyle(
                      color: entry.key == active ? ws.text : ws.textMuted,
                      fontWeight: entry.key == active
                          ? WsMetrics.fwMedium
                          : WsMetrics.fwRegular,
                    ),
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

/// Run status, tokens, cost and extension status entries. A notification takes the status
/// slot (approved session-flow mockup); a click lists the latest ones.
class StatusBar extends StatefulWidget {
  const StatusBar({super.key, required this.state, required this.keys});

  final SessionState state;

  /// The keyboard keys of the active view.
  final String keys;

  @override
  State<StatusBar> createState() => _StatusBarState();
}

class _StatusBarState extends State<StatusBar> {
  final _list = OverlayPortalController();
  final _anchor = LayerLink();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
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
          state.notice,
          state.notices,
        ]),
        builder: (context, _) {
          final (label, color) = statusLabel(context, state.status.value);
          final totals = state.totals.value;
          final failure = state.failure.value;
          final notice = failure == null ? state.notice.value : null;
          final more = state.notices.value.length - 1;
          return Row(
            children: [
              Flexible(
                child: CompositedTransformTarget(
                  link: _anchor,
                  child: OverlayPortal(
                    controller: _list,
                    overlayChildBuilder: (context) => _NoticeList(
                      state: state,
                      link: _anchor,
                      onClose: _list.hide,
                    ),
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: _list.toggle,
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          if (notice != null) ...[
                            Icon(
                              _mark(notice.level),
                              size: 13,
                              color: _tone(context, notice.level),
                            ),
                            const SizedBox(width: WsMetrics.space2),
                          ],
                          Flexible(
                            child: Text(
                              notice?.text ?? '● ${failure ?? label}',
                              key: const Key('status'),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: text.copyWith(
                                color: notice == null
                                    ? color
                                    : _tone(context, notice.level),
                              ),
                            ),
                          ),
                          if (notice != null && more > 0) ...[
                            const SizedBox(width: WsMetrics.space2),
                            Text(
                              '· $more more',
                              style: text.copyWith(color: ws.textFaint),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
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
              // The view's keys show only while ⌘ is held (ADR-0204).
              if (ShortcutReveal.shown(context))
                Text(widget.keys, style: text.copyWith(color: ws.textFaint)),
            ],
          );
        },
      ),
    );
  }

  static String _tokens(int count) =>
      count >= 1000 ? '${(count / 1000).toStringAsFixed(1)}k' : '$count';
}

/// The symbol that tells a notification's level apart without color.
IconData _mark(BridgeLevel level) => switch (level) {
  BridgeLevel.info => LucideIcons.info,
  BridgeLevel.warning || BridgeLevel.error => LucideIcons.triangleAlert,
};

Color _tone(BuildContext context, BridgeLevel level) => switch (level) {
  BridgeLevel.info => context.ws.textMuted,
  BridgeLevel.warning => context.ws.warning,
  BridgeLevel.error => context.ws.danger,
};

/// The latest notifications above the status bar.
class _NoticeList extends StatelessWidget {
  const _NoticeList({
    required this.state,
    required this.link,
    required this.onClose,
  });

  final SessionState state;
  final LayerLink link;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return CompositedTransformFollower(
      link: link,
      targetAnchor: Alignment.topLeft,
      followerAnchor: Alignment.bottomLeft,
      offset: const Offset(-WsMetrics.space2, -WsMetrics.space3),
      child: Align(
        alignment: Alignment.bottomLeft,
        child: Material(
          color: ws.canvas,
          elevation: 0,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          child: Container(
            width: 320,
            constraints: const BoxConstraints(maxHeight: 320),
            padding: const EdgeInsets.all(WsMetrics.space2),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              boxShadow: ws.shadow,
            ),
            child: ValueListenableBuilder(
              valueListenable: state.notices,
              builder: (context, notices, _) => Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    height: WsMetrics.rowNav,
                    child: Row(
                      children: [
                        const SizedBox(width: WsMetrics.space2),
                        Text('Notifications', style: small),
                        const Spacer(),
                        GestureDetector(
                          onTap: () {
                            state.clearNotices();
                            onClose();
                          },
                          child: Text('Clear all', style: small),
                        ),
                        const SizedBox(width: WsMetrics.space2),
                      ],
                    ),
                  ),
                  Flexible(
                    child: ListView(
                      shrinkWrap: true,
                      children: [
                        for (final notice in notices)
                          SizedBox(
                            height: WsMetrics.rowNav,
                            child: Row(
                              children: [
                                const SizedBox(width: WsMetrics.space2),
                                Icon(
                                  _mark(notice.level),
                                  size: 13,
                                  color: _tone(context, notice.level),
                                ),
                                const SizedBox(width: WsMetrics.space3),
                                Expanded(
                                  child: Text(
                                    notice.text,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: const TextStyle(fontSize: 13),
                                  ),
                                ),
                                Text(
                                  '${notice.at.hour.toString().padLeft(2, '0')}:'
                                  '${notice.at.minute.toString().padLeft(2, '0')}',
                                  style: small,
                                ),
                                const SizedBox(width: WsMetrics.space2),
                              ],
                            ),
                          ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
