import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../appearance.dart';
import '../controls.dart';
import '../diff/diff_settings.dart';
import '../diff/diff_view.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/first_start.dart';
import '../shortcuts/keymap.dart';
import '../shortcuts/shortcut_reveal.dart';
import '../shortcuts/shortcut_store.dart';
import '../shortcuts/shortcuts_page.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../trajectory/trajectory_view.dart';
import 'action_area.dart';
import 'chat_view.dart';
import 'input_lane.dart';
import 'sidebar.dart';
import 'status_bar.dart';

enum AgentTab { chat, trajectory, diff }

/// The agent surface: sidebar, tabs, the active tab, the widget strip, the input area and the
/// status bar. Every part keeps its position in every tab (guidelines, principle 8).
class AgentShell extends StatefulWidget {
  const AgentShell({
    super.key,
    required this.state,
    required this.info,
    this.shortcuts,
    this.diffSettings,
    this.appearance,
  });

  final SessionState state;
  final SessionInfo info;

  /// Theme and text size, which Settings change; without it Settings show only shortcuts.
  final AppearanceController? appearance;

  /// The shortcut settings; Standard, unsaved, without them.
  final ShortcutController? shortcuts;

  /// How the Diff tab shows changes; the defaults, unsaved, without them.
  final DiffSettingsController? diffSettings;

  @override
  State<AgentShell> createState() => _AgentShellState();
}

class _AgentShellState extends State<AgentShell> {
  final _composer = TextEditingController();
  final _composerFocus = FocusNode(debugLabel: 'composer');

  /// Keeps the input area's state while it moves into the open tab.
  final _inputKey = GlobalKey(debugLabel: 'input area');
  final _tab = ValueNotifier(AgentTab.chat);
  final _comparing = ValueNotifier(false);
  final _settings = ValueNotifier(false);
  final _focus = FocusNode(debugLabel: 'shell');
  late final _shortcuts = widget.shortcuts ?? ShortcutController.memory();
  late final _diffSettings = widget.diffSettings ?? DiffSettingsController();

  /// Stops the agent before the app exits, so no pi process outlives Liquid.
  late final _lifecycle = AppLifecycleListener(
    onExitRequested: () async {
      await widget.state.dispose();
      return AppExitResponse.exit;
    },
  );

  @override
  void initState() {
    super.initState();
    _lifecycle;
    // The kept choice counts every change read, also before the Diff tab opens.
    _diffSettings.addListener(_hideWhitespace);
    _hideWhitespace();
    widget.state.draft.addListener(_applyDraft);
    widget.state.requests.addListener(_requestsChanged);
    unawaited(widget.state.loadStart());
    // A reopened Trajectory tab starts outside the comparison.
    _tab.addListener(() => _comparing.value = false);
    _shortcuts.report = (problem) =>
        widget.state.notify(BridgeLevel.error, problem);
    if (_shortcuts.firstStart) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _askProfile());
    }
  }

  Future<void> _askProfile() async {
    final profile = await askProfile(context);
    // An approval or a question closed it before a choice; it asks again once they are done.
    if (profile == null) return;
    await _shortcuts.update(_shortcuts.value.choose(profile));
  }

  /// Settings replace the main area; closing them gives the keys back to the shell.
  void _toggleSettings() {
    _settings.value = !_settings.value;
    if (!_settings.value) _focus.requestFocus();
  }

  /// An approval or a question closes Settings and any menu or dialog, which would cover it
  /// or take the keys meant for it; once none is left, a first start asks for the profile
  /// again.
  void _requestsChanged() {
    if (widget.state.asking) {
      _settings.value = false;
      Navigator.of(context).popUntil((route) => route.isFirst);
    } else if (_shortcuts.firstStart && ModalRoute.of(context)!.isCurrent) {
      unawaited(_askProfile());
    }
  }

  /// Text handed back to the input area goes before what the user typed meanwhile, which stays.
  void _applyDraft() {
    final draft = widget.state.draft.value;
    if (draft == null) return;
    final typed = _composer.text.trim();
    final text = typed.isEmpty || typed == draft ? draft : '$draft\n\n$typed';
    _composer.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
    // Taken, so the same text handed back again arrives again.
    widget.state.draft.value = null;
  }

  /// Reads the changes as the Diff settings say, which own the choice.
  void _hideWhitespace() =>
      widget.state.hideWhitespace.value = _diffSettings.value.hideWhitespace;

  @override
  void dispose() {
    _lifecycle.dispose();
    widget.state.draft.removeListener(_applyDraft);
    widget.state.requests.removeListener(_requestsChanged);
    _composer.dispose();
    _composerFocus.dispose();
    _tab.dispose();
    _comparing.dispose();
    _settings.dispose();
    _focus.dispose();
    _shortcuts.report = null;
    if (widget.shortcuts == null) _shortcuts.dispose();
    _diffSettings.removeListener(_hideWhitespace);
    if (widget.diffSettings == null) _diffSettings.dispose();
    super.dispose();
  }

  /// Starts a new session in the chat, ready to describe the task.
  void _newSession() {
    _tab.value = AgentTab.chat;
    _composerFocus.requestFocus();
    unawaited(widget.state.newSession());
  }

  /// What each app-wide command does, in the menu bar and from its keys alike.
  late final Map<Command, VoidCallback> _actions = {
    Command.chat: () => _tab.value = AgentTab.chat,
    Command.trajectory: () => _tab.value = AgentTab.trajectory,
    Command.diff: () => _tab.value = AgentTab.diff,
    Command.focusInput: _composerFocus.requestFocus,
    Command.newSession: _newSession,
    Command.dismissNotice: widget.state.dismissNotice,
    Command.settings: _toggleSettings,
  };

  /// App-wide shortcuts in the macOS menu bar, where they stay out of the GUI;
  /// the menu shows ⌘ keys, the bindings below handle every key on every platform.
  Widget _menus(ShortcutSettings keys, Widget child) {
    if (defaultTargetPlatform != TargetPlatform.macOS) return child;
    PlatformMenuItem item(Command command) => PlatformMenuItem(
      label: command.label,
      shortcut: keys.menu(command)?.menuShortcut,
      onSelected: _actions[command],
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
            PlatformMenuItemGroup(members: [item(Command.settings)]),
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
        PlatformMenu(label: 'Session', menus: [item(Command.newSession)]),
        PlatformMenu(
          label: 'View',
          menus: [
            PlatformMenuItemGroup(
              members: [
                item(Command.chat),
                item(Command.trajectory),
                item(Command.diff),
              ],
            ),
            PlatformMenuItemGroup(members: [item(Command.focusInput)]),
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
  Widget build(BuildContext context) => Keymap(
    notifier: _shortcuts,
    child: ValueListenableBuilder(
      valueListenable: _shortcuts,
      builder: (context, keys, _) => _build(context, keys),
    ),
  );

  Widget _build(BuildContext context, ShortcutSettings keys) {
    final ws = context.ws;
    return _menus(
      keys,
      ShortcutReveal(
        liquid: keys.liquidKey,
        child: CallbackShortcuts(
          bindings: keys.bind(_actions),
          child: Focus(
            focusNode: _focus,
            autofocus: true,
            child: Material(
              color: ws.canvas,
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Sidebar(
                    state: widget.state,
                    info: widget.info,
                    onSettings: _toggleSettings,
                  ),
                  Expanded(
                    child: ValueListenableBuilder(
                      valueListenable: _settings,
                      builder: (context, settings, main) => settings
                          ? ShortcutsPage(
                              shortcuts: _shortcuts,
                              appearance: widget.appearance,
                              onClose: _toggleSettings,
                            )
                          : main!,
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          _Header(state: widget.state),
                          _Tabs(tab: _tab, state: widget.state),
                          Expanded(
                            child: ValueListenableBuilder(
                              valueListenable: _tab,
                              builder: (context, tab, _) {
                                // The input area floats over the open tab's main column.
                                final input = KeyedSubtree(
                                  key: _inputKey,
                                  child: Column(
                                    mainAxisSize: MainAxisSize.min,
                                    crossAxisAlignment:
                                        CrossAxisAlignment.stretch,
                                    children: [
                                      WidgetStrip(state: widget.state),
                                      ActionArea(
                                        state: widget.state,
                                        project: widget.info.project,
                                        composer: _composer,
                                        composerFocus: _composerFocus,
                                      ),
                                    ],
                                  ),
                                );
                                return switch (tab) {
                                  AgentTab.chat => ChatView(
                                    state: widget.state,
                                    project: widget.info.project,
                                    input: input,
                                  ),
                                  AgentTab.diff => DiffView(
                                    state: widget.state,
                                    settings: _diffSettings,
                                    input: input,
                                  ),
                                  AgentTab.trajectory => TrajectoryView(
                                    state: widget.state,
                                    onBranched: _composerFocus.requestFocus,
                                    comparing: _comparing,
                                    input: input,
                                  ),
                                };
                              },
                            ),
                          ),
                          ListenableBuilder(
                            listenable: Listenable.merge([_tab, _comparing]),
                            builder: (context, _) => StatusBar(
                              state: widget.state,
                              keys: switch (_tab.value) {
                                AgentTab.chat => '',
                                AgentTab.diff => diffKeys(),
                                AgentTab.trajectory =>
                                  _comparing.value
                                      ? compareKeys(keys)
                                      : trajectoryKeys(keys),
                              },
                            ),
                          ),
                        ],
                      ),
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
  const _Header({required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => Container(
    height: 44,
    padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space7),
    alignment: Alignment.centerLeft,
    child: SessionTitle(
      state: state,
      style: Theme.of(context).textTheme.labelMedium
          ?.copyWith(fontWeight: WsMetrics.fwSemibold),
    ),
  );
}

/// How many files the latest run changed, beside the Diff tab's name (approved mockup).
class _ChangedFiles extends StatelessWidget {
  const _ChangedFiles({required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.changed,
    builder: (context, changed, _) {
      final count = changed?.length ?? 0;
      if (count == 0) return const SizedBox.shrink();
      return Container(
        key: const Key('changed-files'),
        margin: const EdgeInsets.only(left: WsMetrics.space2),
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
        decoration: BoxDecoration(
          color: context.ws.hover,
          borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        ),
        child: Text(
          '$count',
          style: TextStyle(
            color: context.ws.textMuted,
            fontSize: WsMetrics.fsSmall,
          ),
        ),
      );
    },
  );
}

class _Tabs extends StatelessWidget {
  const _Tabs({required this.tab, required this.state});

  final ValueNotifier<AgentTab> tab;
  final SessionState state;

  static const _commands = {
    AgentTab.chat: Command.chat,
    AgentTab.trajectory: Command.trajectory,
    AgentTab.diff: Command.diff,
  };

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final keys = Keymap.of(context);
    final shown = ShortcutReveal.shown(context);
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
            for (final entry in _commands.entries)
              // Which tab is open is told to assistive technology, not only drawn.
              Semantics(
                selected: entry.key == active,
                inMutuallyExclusiveGroup: true,
                child: InkWell(
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
                              ? ws.accent
                              : Colors.transparent,
                          width: 2,
                        ),
                      ),
                    ),
                    alignment: Alignment.center,
                    // The hint sits on the label's corner, so the tab keeps its size.
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Text(
                              entry.value.label,
                              style: TextStyle(
                                color: entry.key == active
                                    ? ws.text
                                    : ws.textMuted,
                                fontWeight: entry.key == active
                                    ? WsMetrics.fwMedium
                                    : WsMetrics.fwRegular,
                              ),
                            ),
                            if (entry.key == AgentTab.diff)
                              _ChangedFiles(state: state),
                          ],
                        ),
                        // Keys the menu bar shows stay out of the GUI; the Liquid key,
                        // which it cannot show, gets a hint.
                        if (shown &&
                            keys.label(entry.value).isNotEmpty &&
                            keys.label(entry.value) !=
                                keys.menu(entry.value)?.label)
                          Positioned(
                            top: -11,
                            right: -14,
                            child: KeyBadge(keys.label(entry.value)),
                          ),
                      ],
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

/// Extension widgets, directly above the input area.
class WidgetStrip extends StatelessWidget {
  const WidgetStrip({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder(
      valueListenable: state.widgets,
      builder: (context, widgets, _) => SizedBox(
        height: widgetStripHeight,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          child: Align(
            alignment: Alignment.centerLeft,
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
    );
  }
}
