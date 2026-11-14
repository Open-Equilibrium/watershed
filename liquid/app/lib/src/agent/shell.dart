import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../appearance.dart';
import '../controls.dart';
import '../diff/diff_model.dart';
import '../diff/diff_panel.dart';
import '../diff/diff_settings.dart';
import '../diff/diff_view.dart';
import '../diff/turn_scope.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../settings_file.dart';
import '../settings_page.dart';
import '../shortcuts/first_start.dart';
import '../shortcuts/keymap.dart';
import '../shortcuts/shortcut_reveal.dart';
import '../shortcuts/shortcut_store.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../workspace/page_view.dart';
import '../workspace/workspace_state.dart';
import 'action_area.dart';
import 'chat_view.dart';
import 'input_lane.dart';
import 'pull_request_status.dart';
import 'session_card_area.dart';
import 'session_card_settings.dart';
import 'sidebar.dart';
import 'statistics_panel.dart';
import 'status_bar.dart';
import 'worktrees_pane.dart';

/// The window: the sidebar with the Explorer and the agent session, and beside it the open
/// Page or the agent surface with its tabs, the Diff panel beside the chat, the widget strip, the input area
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
    this.cardSettings,
    this.appearance,
    this.parseDiff = parsePatchInBackground,
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

  /// How the Diff panel shows changes; the defaults, unsaved, without them.
  final DiffSettingsController? diffSettings;

  /// Which rows the session card shows; all of them, unsaved, without it.
  final SessionCardSettingsController? cardSettings;

  /// How the Diff panel reads a patch; tests pass the synchronous parser.
  final PatchParser parseDiff;

  @override
  State<AgentShell> createState() => _AgentShellState();
}

class _AgentShellState extends State<AgentShell> {
  /// The composer's controller, which Liquid's writes to the composer replace with a fresh one
  /// when the input area builds (see [_writeComposer]).
  var _composer = TextEditingController();
  bool _composerReplaced = false;
  final _composerFocus = FocusNode(debugLabel: 'composer');

  /// Keeps the input area's state while it moves into the open tab.
  final _inputKey = GlobalKey(debugLabel: 'input area');
  final _diffOpen = ValueNotifier(false);
  final _cardOpen = ValueNotifier(true);
  final _settings = ValueNotifier(false);
  final _focus = FocusNode(debugLabel: 'shell');
  late final _shortcuts = widget.shortcuts ?? ShortcutController.memory();
  late final _diffSettings = widget.diffSettings ?? DiffSettingsController();
  late final _cardSettings =
      widget.cardSettings ?? SessionCardSettingsController();

  /// The settings whose failed saves the status bar reports.
  late final _saved = <SavedSettings<Object?>?>[
    _shortcuts,
    _diffSettings,
    _cardSettings,
    widget.appearance,
  ];

  /// The latest failed save, which Settings show at their top until ✕ or esc, or until the same
  /// settings save again.
  final _saveFailure = ValueNotifier<SaveFailure?>(null);

  /// Stops the agent before the app exits, so no pi process outlives Liquid.
  late final _lifecycle = AppLifecycleListener(
    // The pull request summary is read again when the window regains focus.
    onResume: () => unawaited(widget.state?.pulls.refresh()),
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
    // The kept choice counts every change read, also before the Diff panel opens.
    _diffSettings.addListener(_hideWhitespace);
    _hideWhitespace();
    if (widget.state case final state?) {
      state.draft.addListener(_applyDraft);
      state.question.addListener(_questionChanged);
      unawaited(state.loadStart());
      state.git.watch();
    }
    widget.workspace?.open.addListener(_pageOpened);
    for (final settings in _saved.nonNulls) {
      settings.report = (problem) => widget.state != null
          ? widget.state!.notify(BridgeLevel.error, problem)
          : widget.workspace?.problem.value = problem;
      settings.saveOutcome = (failure) => _saveOutcome(settings, failure);
    }
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
    // A question closed it before a choice; it asks again once it is answered.
    if (profile == null) return;
    await _shortcuts.change(_shortcuts.value.choose(profile));
  }

  /// Shows a failed save at the top of Settings, or clears the notice once the same settings save.
  void _saveOutcome(SavedSettings<Object?> settings, String? failure) {
    if (failure != null) {
      _saveFailure.value = (settings: settings, text: failure);
    } else if (_saveFailure.value?.settings == settings) {
      _saveFailure.value = null;
    }
  }

  /// Settings replace the main area; closing them gives the keys back to the shell.
  void _toggleSettings() {
    _settings.value = !_settings.value;
    if (!_settings.value) _focus.requestFocus();
  }

  /// A question closes Settings and any menu or dialog, which would cover it or take the keys
  /// meant for it; once none is left, a first start asks for the profile again.
  void _questionChanged() {
    if (widget.state!.asking) {
      _settings.value = false;
      Navigator.of(context).popUntil((route) => route.isFirst);
    } else if (_shortcuts.firstStart && ModalRoute.of(context)!.isCurrent) {
      unawaited(_askProfile());
    }
  }

  /// Shows `value` in the composer as Liquid's text; its field takes a fresh controller when the
  /// input area builds, so ⌘Z cannot bring back what it replaced.
  void _writeComposer(TextEditingValue value) {
    if (writeText(_composer, value)) setState(() => _composerReplaced = true);
  }

  /// Clears the composer after a send, so ⌘Z cannot bring the message back.
  void _sent() => _writeComposer(TextEditingValue.empty);

  /// Text handed back to the input area goes before what the user typed meanwhile, which stays.
  void _applyDraft() {
    final state = widget.state!;
    final draft = state.draft.value;
    if (draft == null) return;
    final typed = _composer.text;
    final bare = typed.trim();
    // The typed text comes back as it was typed; only the emptiness and equality checks trim
    // it, and the same message typed again keeps its own form.
    final text = bare.isEmpty
        ? draft
        : bare == draft.trim()
        ? typed
        : '$draft\n\n$typed';
    _writeComposer(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
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
    widget.state?.question.removeListener(_questionChanged);
    _composer.dispose();
    _composerFocus.dispose();
    _diffOpen.dispose();
    _cardOpen.dispose();
    _settings.dispose();
    _focus.dispose();
    for (final settings in _saved.nonNulls) {
      settings.report = null;
      settings.saveOutcome = null;
    }
    _saveFailure.dispose();
    if (widget.shortcuts == null) _shortcuts.dispose();
    _diffSettings.removeListener(_hideWhitespace);
    if (widget.diffSettings == null) _diffSettings.dispose();
    if (widget.cardSettings == null) _cardSettings.dispose();
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

  /// Opens the Diff panel beside the Chat, showing `scope`; the Changes row and a turn's Open diff
  /// do.
  void _openDiff(DiffScope scope) {
    widget.state!.diffScope.value = scope;
    _showSession();
    _diffOpen.value = true;
  }

  /// Opens or closes the Diff panel, the keys and its close button alike; closing gives the keys
  /// back to the shell.
  void _toggleDiff() {
    _showSession();
    if (_diffOpen.value) {
      _diffOpen.value = false;
      _focus.requestFocus();
    } else {
      _diffOpen.value = true;
    }
  }

  /// Shows or hides the session card, whether it sits beside the Chat or over it.
  void _toggleStatistics(SessionState state) {
    _showSession();
    state.statisticsOpen.value = !state.statisticsOpen.value;
  }

  void _toggleCard() {
    _showSession();
    _cardOpen.value = !_cardOpen.value;
  }

  /// Starts a new session in the chat, ready to describe the task.
  void _newSession() {
    _showSession();
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
      Command.chat: _showSession,
      Command.diff: _toggleDiff,
      Command.sessionDetails: _toggleCard,
      Command.statistics: () => _toggleStatistics(state),
      Command.focusInput: () {
        _showSession();
        // A tab that Settings just gave way to takes the keys when it opens, after this action.
        unawaited(
          WidgetsBinding.instance.endOfFrame.then(
            (_) => _composerFocus.requestFocus(),
          ),
        );
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
                item(Command.diff),
                item(Command.sessionDetails),
                item(Command.statistics),
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
    if (_composerReplaced) {
      _composerReplaced = false;
      _composer = adoptController(_composer);
    }
    return _menus(
      keys,
      ShortcutReveal(
        liquid: keys.liquidKey,
        child: CallbackShortcuts(
          bindings: keys.bind(_actions),
          child: Focus(
            focusNode: _focus,
            autofocus: true,
            // The whole window shows no focus, so Tab passes over it.
            skipTraversal: true,
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
                            saveFailure: _saveFailure,
                            onDismissSaveFailure: () =>
                                _saveFailure.value = null,
                            worktrees: widget.state == null
                                ? null
                                : WorktreesPane(
                                    state: widget.state!,
                                    device: widget.info!.device,
                                  ),
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

  /// The agent surface: header, tabs, the Chat beside the Diff panel with the input area and
  /// the session card, and the status bar.
  Widget _agentSurface(
    BuildContext context,
    SessionState state,
    SessionInfo info,
  ) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Header(
          state: state,
          project: info.project,
          cardOpen: _cardOpen,
          onDetails: _toggleCard,
        ),
        const _Tabs(),
        Expanded(
          child: Stack(
            children: [
              Positioned.fill(
                child: ValueListenableBuilder(
                  valueListenable: _diffOpen,
                  builder: (context, diffOpen, _) {
                    // The input area stays with the Chat, beside the Diff panel.
                    final input = KeyedSubtree(
                      key: _inputKey,
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          WidgetStrip(state: state),
                          PushedOffer(state: state, device: info.device),
                          ActionArea(
                            state: state,
                            project: info.project,
                            composer: _composer,
                            composerFocus: _composerFocus,
                            onSent: _sent,
                          ),
                        ],
                      ),
                    );
                    return Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(
                          child: SessionCardArea(
                            state: state,
                            device: info.device,
                            settings: _cardSettings,
                            open: _cardOpen,
                            diffOpen: diffOpen,
                            onChanges: () => _openDiff(const LatestTurn()),
                            child: ChatView(
                              state: state,
                              info: info,
                              input: input,
                              onOpenDiff: (turn) => _openDiff(OneTurn(turn)),
                            ),
                          ),
                        ),
                        if (diffOpen)
                          DiffPanel(
                            state: state,
                            settings: _diffSettings,
                            parse: widget.parseDiff,
                            onClose: _toggleDiff,
                          ),
                      ],
                    );
                  },
                ),
              ),
              Positioned(
                top: WsMetrics.space4,
                right: WsMetrics.space4,
                bottom: WsMetrics.space4,
                left: WsMetrics.space4,
                child: ListenableBuilder(
                  listenable: state.statisticsOpen,
                  builder: (context, _) => state.statisticsOpen.value
                      ? Align(
                          alignment: Alignment.topRight,
                          child: ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 660),
                            child: StatisticsPanel(
                              state: state,
                              onClose: () => state.statisticsOpen.value = false,
                              onAsk: () {
                                state.statisticsOpen.value = false;
                                _composerFocus.requestFocus();
                              },
                            ),
                          ),
                        )
                      : const SizedBox.shrink(),
                ),
              ),
            ],
          ),
        ),
        ListenableBuilder(
          listenable: _diffOpen,
          builder: (context, _) =>
              StatusBar(state: state, keys: _diffOpen.value ? diffKeys() : ''),
        ),
      ],
    );
  }
}

/// The session's title with its project and model as chips, and at the right the button that
/// shows or hides the session card.
class _Header extends StatelessWidget {
  const _Header({
    required this.state,
    required this.project,
    required this.cardOpen,
    required this.onDetails,
  });

  final SessionState state;
  final String project;
  final ValueNotifier<bool> cardOpen;
  final VoidCallback onDetails;

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
      height: WsMetrics.rowHeader,
      padding: const EdgeInsets.only(
        left: WsMetrics.space7,
        right: WsMetrics.space5,
      ),
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
          _DetailsButton(open: cardOpen, onPressed: onDetails),
        ],
      ),
    );
  }
}

/// The header's button that shows or hides the session card, as T3 Code's sliders button does.
class _DetailsButton extends StatelessWidget {
  const _DetailsButton({required this.open, required this.onPressed});

  final ValueListenable<bool> open;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: open,
      builder: (context, shown, _) => CommandHint(
        command: Command.sessionDetails,
        child: Tooltip(
          message: 'Session details',
          child: Semantics(
            button: true,
            selected: shown,
            child: InkWell(
              key: const Key('session-details'),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: onPressed,
              child: Container(
                width: 28,
                height: 28,
                decoration: BoxDecoration(
                  color: shown ? ws.hover : null,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Icon(
                  LucideIcons.slidersHorizontal,
                  size: 16,
                  color: shown ? ws.text : ws.textMuted,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The tab strip: the Chat, the session's only tab.
class _Tabs extends StatelessWidget {
  const _Tabs();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 38,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space6),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      // Which tab is open is told to assistive technology, not only drawn.
      child: Semantics(
        selected: true,
        inMutuallyExclusiveGroup: true,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          alignment: Alignment.center,
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: ws.accent, width: 2)),
          ),
          child: CommandHint(
            command: Command.chat,
            child: Text(
              'Chat',
              style: TextStyle(color: ws.text, fontWeight: WsMetrics.fwMedium),
            ),
          ),
        ),
      ),
    );
  }
}

/// A command's key, shown on the corner of its control while the reveal key is held. The menu
/// bar shows the ⌘ keys itself, so they stay out of the GUI.
class CommandHint extends StatelessWidget {
  const CommandHint({super.key, required this.command, required this.child});

  final Command command;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final keys = Keymap.of(context);
    final label = keys.label(command);
    if (!ShortcutReveal.shown(context) ||
        label.isEmpty ||
        label == keys.menu(command)?.label) {
      return child;
    }
    // The hint sits on the control's corner, so the control keeps its size.
    return Stack(
      clipBehavior: Clip.none,
      children: [
        child,
        Positioned(top: -11, right: -14, child: KeyBadge(label)),
      ],
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
