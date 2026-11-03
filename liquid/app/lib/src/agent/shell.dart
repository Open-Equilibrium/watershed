import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../appearance.dart';
import '../controls.dart';
import '../diff/diff_settings.dart';
import '../diff/diff_view.dart';
import '../diff/turn_scope.dart';
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
import '../workspace/page_view.dart';
import '../workspace/workspace_state.dart';
import 'action_area.dart';
import 'branch_menu.dart';
import 'chat_view.dart';
import 'git_actions.dart';
import 'input_lane.dart';
import 'sidebar.dart';
import 'status_bar.dart';

enum AgentTab { chat, trajectory, diff }

/// The window: the sidebar with the Explorer and the agent session, and beside it the open
/// Page or the agent surface with its tabs, the active tab, the widget strip, the input area
/// and the status bar. Every part keeps its position in every tab (guidelines, principle 8).
class AgentShell extends StatefulWidget {
  const AgentShell({
    super.key,
    this.state,
    this.info,
    this.workspace,
    this.workspaceProblem,
    this.shortcuts,
    this.diffSettings,
    this.appearance,
  }) : assert((state == null) == (info == null));

  /// The agent session, if one started.
  final SessionState? state;
  final SessionInfo? info;

  /// The Workspace, or why it did not open.
  final WorkspaceState? workspace;
  final String? workspaceProblem;

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
      await widget.state?.dispose();
      // Every edit asked for reaches the replica before Liquid exits.
      await widget.workspace?.drain();
      await widget.workspace?.dispose();
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
    if (widget.state case final state?) {
      state.draft.addListener(_applyDraft);
      state.requests.addListener(_requestsChanged);
      unawaited(state.loadStart());
      state.git.watch();
    }
    widget.workspace?.open.addListener(_pageOpened);
    // A reopened Trajectory tab starts outside the comparison.
    _tab.addListener(() => _comparing.value = false);
    _shortcuts.report = (problem) => widget.state != null
        ? widget.state!.notify(BridgeLevel.error, problem)
        : widget.workspace?.problem.value = problem;
    if (_shortcuts.firstStart) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _askProfile());
    }
  }

  @override
  void didUpdateWidget(AgentShell oldWidget) {
    super.didUpdateWidget(oldWidget);
    // Only the shown session's checkout is fetched.
    if (oldWidget.state != widget.state) {
      oldWidget.state?.git.stop();
      widget.state?.git.watch();
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
    if (widget.state!.asking) {
      _settings.value = false;
      Navigator.of(context).popUntil((route) => route.isFirst);
    } else if (_shortcuts.firstStart && ModalRoute.of(context)!.isCurrent) {
      unawaited(_askProfile());
    }
  }

  /// Text handed back to the input area goes before what the user typed meanwhile, which stays.
  void _applyDraft() {
    final state = widget.state!;
    final draft = state.draft.value;
    if (draft == null) return;
    final typed = _composer.text.trim();
    final text = typed.isEmpty || typed == draft ? draft : '$draft\n\n$typed';
    _composer.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
    // Taken, so the same text handed back again arrives again.
    state.draft.value = null;
  }

  /// Reads the changes as the Diff settings say, which own the choice.
  void _hideWhitespace() =>
      widget.state?.hideWhitespace.value = _diffSettings.value.hideWhitespace;

  @override
  void dispose() {
    widget.state?.git.stop();
    widget.workspace?.open.removeListener(_pageOpened);
    _lifecycle.dispose();
    widget.state?.draft.removeListener(_applyDraft);
    widget.state?.requests.removeListener(_requestsChanged);
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

  /// A Page opened from anywhere shows in place of Settings.
  void _pageOpened() {
    if (widget.workspace?.open.value != null) _settings.value = false;
  }

  /// Shows the agent session in place of the open Page.
  void _showSession() {
    _settings.value = false;
    widget.workspace?.open.value = null;
  }

  /// Shows the agent session at `tab`.
  void _showTab(AgentTab tab) {
    _showSession();
    _tab.value = tab;
  }

  /// Starts a new session in the chat, ready to describe the task.
  void _newSession() {
    _showTab(AgentTab.chat);
    _composerFocus.requestFocus();
    unawaited(widget.state!.newSession());
  }

  /// A new Page in the open Page's area, or else in the first one (approved Pages mockups).
  void _newPage() {
    final workspace = widget.workspace!;
    final area =
        workspace.open.value?.area ?? workspace.areas.value.firstOrNull?.id;
    if (area == null) return;
    _settings.value = false;
    unawaited(workspace.newPage(area).catchError((_) {}));
  }

  /// What each app-wide command does, in the menu bar and from its keys alike; the agent's
  /// commands only with a session, the Workspace's only with a Workspace.
  late final Map<Command, VoidCallback> _actions = {
    if (widget.state case final state?) ...{
      Command.chat: () => _showTab(AgentTab.chat),
      Command.trajectory: () => _showTab(AgentTab.trajectory),
      Command.diff: () => _showTab(AgentTab.diff),
      Command.focusInput: () {
        _showSession();
        _composerFocus.requestFocus();
      },
      Command.newSession: _newSession,
      Command.dismissNotice: state.dismissNotice,
    },
    if (widget.workspace != null) Command.newPage: _newPage,
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
        PlatformMenu(label: 'Page', menus: [item(Command.newPage)]),
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
                    workspace: widget.workspace,
                    workspaceProblem: widget.workspaceProblem,
                    onSettings: _toggleSettings,
                    onNewSession: _newSession,
                    onShowSession: _showSession,
                  ),
                  Expanded(
                    child: ListenableBuilder(
                      listenable: Listenable.merge([
                        _settings,
                        ?widget.workspace?.open,
                      ]),
                      builder: (context, agent) {
                        if (_settings.value) {
                          return SettingsPage(
                            shortcuts: _shortcuts,
                            appearance: widget.appearance,
                            onClose: _toggleSettings,
                          );
                        }
                        final open = widget.workspace?.open.value;
                        // The agent surface stays built behind a Page, so it keeps its place.
                        return IndexedStack(
                          index: open == null ? 0 : 1,
                          sizing: StackFit.expand,
                          children: [
                            agent ?? const SizedBox.shrink(),
                            if (open != null)
                              WorkspacePage(
                                key: ValueKey(open.page),
                                state: widget.workspace!,
                                open: open,
                              )
                            else
                              const SizedBox.shrink(),
                          ],
                        );
                      },
                      child: switch ((widget.state, widget.info)) {
                        (final state?, final info?) => _agentSurface(
                          context,
                          keys,
                          state,
                          info,
                        ),
                        _ => null,
                      },
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

  /// The agent surface: header, tabs, the active tab with the input area, and the status bar.
  Widget _agentSurface(
    BuildContext context,
    ShortcutSettings keys,
    SessionState state,
    SessionInfo info,
  ) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Header(state: state, project: info.project, device: info.device),
        _Tabs(tab: _tab, state: state),
        Expanded(
          child: ValueListenableBuilder(
            valueListenable: _tab,
            builder: (context, tab, _) {
              // The input area floats over the open tab's main column.
              final input = KeyedSubtree(
                key: _inputKey,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    WidgetStrip(state: state),
                    ActionArea(
                      state: state,
                      project: info.project,
                      composer: _composer,
                      composerFocus: _composerFocus,
                    ),
                  ],
                ),
              );
              return switch (tab) {
                AgentTab.chat => ChatView(
                  state: state,
                  info: info,
                  input: input,
                  onOpenDiff: (turn) {
                    state.diffScope.value = OneTurn(turn);
                    _tab.value = AgentTab.diff;
                  },
                ),
                AgentTab.diff => DiffView(
                  state: state,
                  settings: _diffSettings,
                  input: input,
                ),
                AgentTab.trajectory => TrajectoryView(
                  state: state,
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
            state: state,
            keys: switch (_tab.value) {
              AgentTab.chat => '',
              AgentTab.diff => diffKeys(),
              AgentTab.trajectory =>
                _comparing.value ? compareKeys(keys) : trajectoryKeys(keys),
            },
          ),
        ),
      ],
    );
  }
}

/// The session's title with its project and model as chips (approved look mockup), and at the
/// right the checkout's branch (approved Git mockups).
class _Header extends StatelessWidget {
  const _Header({
    required this.state,
    required this.project,
    required this.device,
  });

  final SessionState state;
  final String project;
  final String device;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget chip(String text) => Container(
      margin: const EdgeInsets.only(left: WsMetrics.space3),
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        border: Border.all(color: ws.borderStrong),
      ),
      child: Text(
        text,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsSmall),
      ),
    );
    return Container(
      key: const Key('session-header'),
      height: 44,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space7),
      child: Row(
        children: [
          Expanded(
            child: Row(
              children: [
                Flexible(
                  child: SessionTitle(
                    state: state,
                    style: Theme.of(context).textTheme.labelMedium
                        ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                  ),
                ),
                Flexible(child: chip(project)),
                Flexible(
                  child: ValueListenableBuilder(
                    valueListenable: state.model,
                    builder: (context, model, _) => model == null
                        ? const SizedBox.shrink()
                        : chip(model.substring(model.indexOf('/') + 1)),
                  ),
                ),
              ],
            ),
          ),
          Flexible(child: BranchChip(state: state)),
          const SizedBox(width: WsMetrics.space2),
          GitActions(state: state, device: device),
        ],
      ),
    );
  }
}

/// How many files the Diff tab shows, beside its name (approved mockup): the shown turns',
/// the latest by default, counted as the whitespace choice says.
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
