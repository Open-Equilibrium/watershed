import 'dart:async';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../agent_hosts/agent_hosts.dart';
import '../agent_hosts/agent_hosts_pane.dart';
import '../appearance.dart';
import '../controls.dart';
import '../diff/diff_model.dart';
import '../diff/diff_panel.dart';
import '../diff/diff_settings.dart';
import '../diff/diff_view.dart';
import '../diff/turn_scope.dart';
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
import '../workspace/source_page.dart';
import '../workspace/automations.dart';
import '../workspace/history_dialog.dart';
import '../workspace/notices.dart';
import '../workspace/page_history.dart';
import '../workspace/power_bar.dart';
import '../workspace/roles.dart';
import '../workspace/page_view.dart';
import '../workspace/table_page.dart';
import '../workspace/workspaces.dart';
import '../workspace/workspace_state.dart';
import 'action_area.dart';
import 'chat_view.dart';
import 'move_dialog.dart';
import 'session_card_area.dart';
import 'session_card_layout.dart';
import 'session_card_settings.dart';
import 'sidebar.dart';
import 'start_page.dart';
import 'statistics_panel.dart';
import 'status_bar.dart';

class AgentShell extends StatefulWidget {
  const AgentShell({
    super.key,
    this.state,
    this.info,
    this.workspace,
    this.workspaceProblem,
    this.workspaces,
    this.agentProblem,
    this.shortcuts,
    this.diffSettings,
    this.cardSettings,
    this.appearance,
    this.agentHosts,
    this.settings,
    this.parseDiff = parsePatchInBackground,
  }) : assert((state == null) == (info == null));

  final SessionState? state;
  final SessionInfo? info;

  final WorkspaceState? workspace;
  final String? workspaceProblem;

  final Workspaces? workspaces;

  final String? agentProblem;

  final AppearanceController? appearance;

  final ShortcutController? shortcuts;

  final DiffSettingsController? diffSettings;

  final SessionCardSettingsController? cardSettings;

  final AgentHostsController? agentHosts;

  final ValueNotifier<SettingsSection?>? settings;

  final PatchParser parseDiff;

  @override
  State<AgentShell> createState() => _AgentShellState();
}

class _AgentShellState extends State<AgentShell> {
  var _composer = TextEditingController();
  bool _composerReplaced = false;
  final _composerFocus = FocusNode(debugLabel: 'composer');

  final _inputKey = GlobalKey(debugLabel: 'input area');
  final _diffOpen = ValueNotifier(false);
  final _card = CardVisibility();
  final _cardToggleFocus = FocusNode(debugLabel: 'session card');
  late final _settings =
      widget.settings ?? ValueNotifier<SettingsSection?>(null);
  final _focus = FocusNode(debugLabel: 'shell');
  late final _shortcuts = widget.shortcuts ?? ShortcutController.memory();
  late final _diffSettings = widget.diffSettings ?? DiffSettingsController();
  late final _cardSettings =
      widget.cardSettings ?? SessionCardSettingsController();

  late final _saved = <SavedSettings<Object?>?>[
    _shortcuts,
    _diffSettings,
    _cardSettings,
    widget.appearance,
  ];

  final _saveFailure = ValueNotifier<SaveFailure?>(null);

  final _hooks =
      <
        (
          SavedSettings<Object?>,
          void Function(String problem),
          void Function(String? failure),
        )
      >[];

  late final _lifecycle = AppLifecycleListener(
    onExitRequested: () async {
      await widget.state?.dispose();
      await widget.workspace?.drain();
      await widget.workspace?.dispose();
      return AppExitResponse.exit;
    },
  );

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_liquidTap);
    _lifecycle;
    _diffSettings.addListener(_hideWhitespace);
    _hideWhitespace();
    if (widget.state case final state?) {
      state.hostLabel = _hostName;
      state.draft.addListener(_applyDraft);
      state.mention.addListener(_applyMention);
      state.request.addListener(_requestChanged);
      unawaited(state.loadStart());
    }
    widget.workspace?.open.addListener(_pageOpened);
    for (final settings in _saved.nonNulls) {
      void report(String problem) => widget.state != null
          ? widget.state!.notify(NoticeLevel.error, problem)
          : widget.workspace?.report(problem);
      void outcome(String? failure) => _saveOutcome(settings, failure);
      settings.report = report;
      settings.saveOutcome = outcome;
      _hooks.add((settings, report, outcome));
    }
    if (_shortcuts.firstStart) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _askProfile());
    }
  }

  Future<void> _askProfile() async {
    if (widget.state?.request.value != null) return;
    final profile = await askProfile(context);
    if (profile == null) return;
    await _shortcuts.change(_shortcuts.value.choose(profile));
  }

  void _saveOutcome(Object settings, String? failure) {
    if (failure != null) {
      _saveFailure.value = (settings: settings, text: failure);
    } else if (_saveFailure.value?.settings == settings) {
      _saveFailure.value = null;
    }
  }

  void _toggleSettings() {
    _settings.value = _settings.value == null ? _firstSection : null;
    if (_settings.value == null) _focus.requestFocus();
  }

  SettingsSection get _firstSection => widget.appearance != null
      ? SettingsSection.appearance
      : SettingsSection.shortcuts;

  void _requestChanged() {
    if (widget.state!.request.value != null) {
      _settings.value = null;
      _powerBar.value = false;
      Navigator.of(context).popUntil((route) => route.isFirst);
    } else if (_shortcuts.firstStart && ModalRoute.of(context)!.isCurrent) {
      unawaited(_askProfile());
    }
  }

  void _writeComposer(TextEditingValue value) {
    if (writeText(_composer, value)) setState(() => _composerReplaced = true);
  }

  void _sent() => _writeComposer(TextEditingValue.empty);

  void _applyDraft() {
    final state = widget.state!;
    final draft = state.draft.value;
    if (draft == null) return;
    final typed = _composer.text;
    final bare = typed.trim();
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
    state.draft.value = null;
  }

  void _applyMention() {
    final state = widget.state!;
    final name = state.mention.value;
    if (name == null) return;
    final rest = _composer.text.replaceFirst(RegExp(r'^@\S*\s*'), '');
    final text = '@$name $rest';
    _writeComposer(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
    );
    _composerFocus.requestFocus();
    state.mention.value = null;
  }

  void _hideWhitespace() =>
      widget.state?.hideWhitespace.value = _diffSettings.value.hideWhitespace;

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_liquidTap);
    _liquidAlone?.cancel();
    _powerBar.dispose();
    _automations.dispose();
    _history.dispose();
    widget.workspace?.open.removeListener(_pageOpened);
    _lifecycle.dispose();
    widget.state?.draft.removeListener(_applyDraft);
    widget.state?.mention.removeListener(_applyMention);
    widget.state?.request.removeListener(_requestChanged);
    _composer.dispose();
    _composerFocus.dispose();
    _diffOpen.dispose();
    _card.dispose();
    _cardToggleFocus.dispose();
    if (widget.settings == null) _settings.dispose();
    _focus.dispose();
    for (final (settings, report, outcome) in _hooks) {
      if (settings.report == report) settings.report = null;
      if (settings.saveOutcome == outcome) settings.saveOutcome = null;
    }
    _saveFailure.dispose();
    if (widget.shortcuts == null) _shortcuts.dispose();
    _diffSettings.removeListener(_hideWhitespace);
    if (widget.diffSettings == null) _diffSettings.dispose();
    if (widget.cardSettings == null) _cardSettings.dispose();
    super.dispose();
  }

  void _pageOpened() {
    if (widget.workspace?.open.value != null) _settings.value = null;
  }

  String _hostName(String? address) =>
      widget.agentHosts?.value.hosts
          .where((host) => host.address == address)
          .firstOrNull
          ?.name ??
      widget.state?.hostNameOf(address) ??
      (address ?? thisMacName);

  void _showSession() {
    _settings.value = null;
    widget.workspace?.open.value = null;
  }

  void _openDiff(DiffScope scope) {
    unawaited(widget.state!.showTurns(scope));
    _showSession();
    _diffOpen.value = true;
  }

  void _toggleDiff() {
    _showSession();
    if (_diffOpen.value) {
      _diffOpen.value = false;
      _focus.requestFocus();
    } else {
      _diffOpen.value = true;
    }
  }

  void _toggleStatistics(SessionState state) {
    _showSession();
    state.statisticsOpen.value = !state.statisticsOpen.value;
  }

  void _toggleCard() {
    _showSession();
    _card.toggle();
    if (_card.shown) {
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => _card.focusFirstControl(),
      );
    }
  }

  void _closeCardPopover() {
    _card.closePopover();
    _cardToggleFocus.requestFocus();
  }

  void _newSession() {
    _showSession();
    _composerFocus.requestFocus();
    unawaited(widget.state!.newSession());
  }

  void _newPage() {
    final workspace = widget.workspace!;
    final area =
        workspace.open.value?.area ?? workspace.areas.value.firstOrNull?.id;
    if (area == null) return;
    _settings.value = null;
    unawaited(workspace.newPage(area).catchError((_) {}));
  }

  late final Map<Command, VoidCallback> _actions = {
    if (widget.state case final state?) ...{
      Command.chat: _showSession,
      Command.diff: _toggleDiff,
      Command.sessionDetails: _toggleCard,
      Command.statistics: () => _toggleStatistics(state),
      Command.focusInput: () {
        _showSession();
        unawaited(
          WidgetsBinding.instance.endOfFrame.then(
            (_) => _composerFocus.requestFocus(),
          ),
        );
      },
      Command.newSession: _newSession,
      Command.dismissNotice: state.dismissNotice,
    },
    if (widget.workspace != null) ...{
      Command.newPage: _newPage,
      Command.powerBar: () => _powerBar.value = !_powerBar.value,
    },
    Command.settings: _toggleSettings,
  };

  final _powerBar = ValueNotifier(false);

  final _automations = ValueNotifier(false);

  final _history = ValueNotifier(false);

  Timer? _liquidAlone;

  bool _liquidTap(KeyEvent event) {
    if (widget.workspace == null || !_shortcuts.value.liquidKey) return false;
    if (event.logicalKey != liquidKey) {
      if (event is KeyDownEvent) _liquidAlone?.cancel();
      return false;
    }
    if (event is KeyDownEvent) {
      _liquidAlone?.cancel();
      _liquidAlone = Timer(revealDelay, () {});
    } else if (event is KeyUpEvent && (_liquidAlone?.isActive ?? false)) {
      _liquidAlone?.cancel();
      if (ModalRoute.of(context)?.isCurrent ?? true) {
        _powerBar.value = !_powerBar.value;
      }
    }
    return false;
  }

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
    final project = widget.state?.project;
    if (_composerReplaced) {
      _composerReplaced = false;
      _composer = adoptController(_composer);
    }
    return _menus(
      keys,
      ShortcutReveal(
        liquid: keys.liquidKey,
        child: CallbackShortcuts(
          bindings: {
            ...keys.bind(_actions),
            if (widget.workspaces case final workspaces?)
              ...Fixed.switchWorkspace.bind([
                for (var index = 0; index < 9; index++)
                  () => unawaited(workspaces.switchTo(index)),
              ]),
          },
          child: UndoScope(
            state: widget.workspace,
            active: () => _settings.value == null,
            child: FocusScope(
              child: Focus(
                focusNode: _focus,
                autofocus: true,
                skipTraversal: true,
                child: _withPowerBar(
                  Material(
                    color: ws.canvas,
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Sidebar(
                          state: widget.state,
                          info: widget.info,
                          workspace: widget.workspace,
                          workspaceProblem: widget.workspaceProblem,
                          workspaces: widget.workspaces,
                          agentProblem: widget.agentProblem,
                          onSettings: _toggleSettings,
                          onAutomations: widget.workspace == null
                              ? null
                              : () => _automations.value = true,
                          onHistory: widget.workspace == null
                              ? null
                              : () => _history.value = true,
                          onSearch: widget.workspace == null
                              ? null
                              : () => _powerBar.value = true,
                          onNewSession: _newSession,
                          onShowSession: _showSession,
                          onMove: widget.state == null
                              ? null
                              : (listed) => unawaited(
                                  moveSession(
                                    context,
                                    widget.state!,
                                    listed,
                                    nameOf: _hostName,
                                  ),
                                ),
                        ),
                        Expanded(
                          child: ListenableBuilder(
                            listenable: Listenable.merge([
                              _settings,
                              ?widget.workspace?.open,
                            ]),
                            builder: (context, agent) {
                              if (_settings.value case final section?) {
                                return SettingsPage(
                                  shortcuts: _shortcuts,
                                  section: section,
                                  onSection: (next) => _settings.value = next,
                                  appearance: widget.appearance,
                                  cardSettings: _cardSettings,
                                  onClose: _toggleSettings,
                                  saveFailure: _saveFailure,
                                  onDismissSaveFailure: () =>
                                      _saveFailure.value = null,
                                  writer: project == null
                                      ? null
                                      : WriterModelPane(project: project),
                                  roles: switch (widget.workspace) {
                                    final workspace? => RolesPane(
                                      state: workspace,
                                    ),
                                    null => null,
                                  },
                                  agentHosts: widget.agentHosts == null
                                      ? null
                                      : AgentHostsPane(
                                          hosts: widget.agentHosts!,
                                          saveOutcome: (failure) =>
                                              _saveOutcome(
                                                widget.agentHosts!,
                                                failure,
                                              ),
                                        ),
                                );
                              }
                              final open = widget.workspace?.open.value;
                              return IndexedStack(
                                index: open == null ? 0 : 1,
                                sizing: StackFit.expand,
                                children: [
                                  agent ?? const SizedBox.shrink(),
                                  if (open?.table case final table?)
                                    TablePage(
                                      key: ValueKey(table),
                                      state: widget.workspace!,
                                      table: table,
                                    )
                                  else if (open?.source case final source?)
                                    SourcePage(
                                      key: ValueKey((source, open!.inside)),
                                      state: widget.workspace!,
                                      open: open,
                                    )
                                  else if (open != null)
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
          ),
        ),
      ),
    );
  }

  Widget _withPowerBar(Widget child) {
    final workspace = widget.workspace;
    if (workspace == null) return child;
    return ListenableBuilder(
      listenable: Listenable.merge([
        _powerBar,
        _automations,
        _history,
        workspace.pageHistory,
      ]),
      builder: (context, _) => Stack(
        children: [
          child,
          if (_history.value)
            Positioned.fill(
              child: HistoryDialog(
                state: workspace,
                onClose: () {
                  _history.value = false;
                  _focus.requestFocus();
                },
              ),
            ),
          if (workspace.pageHistory.value case final page?)
            Positioned.fill(
              child: PageHistoryDialog(
                key: ValueKey(page),
                state: workspace,
                page: page,
                onClose: () {
                  workspace.pageHistory.value = null;
                  _focus.requestFocus();
                },
              ),
            ),
          if (_automations.value)
            Positioned.fill(
              child: AutomationsDialog(
                state: workspace,
                onClose: () {
                  _automations.value = false;
                  _focus.requestFocus();
                },
              ),
            ),
          if (_powerBar.value)
            Positioned.fill(
              child: PowerBar(
                state: workspace,
                onClose: () {
                  _powerBar.value = false;
                  _focus.requestFocus();
                },
                onNewPage: _newPage,
                onSettings: _toggleSettings,
              ),
            ),
          Positioned.fill(
            child: IgnorePointer(child: Notices(state: workspace)),
          ),
        ],
      ),
    );
  }

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
          card: _card,
          toggleFocus: _cardToggleFocus,
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
                    final input = KeyedSubtree(
                      key: _inputKey,
                      child: ActionArea(
                        state: state,
                        composer: _composer,
                        composerFocus: _composerFocus,
                        onSent: _sent,
                      ),
                    );
                    return SessionCardArea(
                      state: state,
                      settings: _cardSettings,
                      visibility: _card,
                      diffSettings: _diffSettings,
                      viewportWidth: MediaQuery.sizeOf(context).width,
                      onChanges: _openDiff,
                      onDismiss: _closeCardPopover,
                      diff: diffOpen
                          ? DiffPanel(
                              state: state,
                              settings: _diffSettings,
                              parse: widget.parseDiff,
                              onClose: _toggleDiff,
                            )
                          : null,
                      child: ChatView(
                        state: state,
                        info: info,
                        input: input,
                        onOpenDiff: (turn) => _openDiff(OneTurn(turn)),
                      ),
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

class _Header extends StatelessWidget {
  const _Header({
    required this.state,
    required this.project,
    required this.card,
    required this.toggleFocus,
    required this.onDetails,
  });

  final SessionState state;
  final String project;
  final CardVisibility card;
  final FocusNode toggleFocus;
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
                  child: ListenableBuilder(
                    listenable: Listenable.merge([
                      state.model,
                      state.providers,
                    ]),
                    builder: (context, _) => switch (state.model.value) {
                      final model? => chip(
                        modelName(state.providers.value, model),
                      ),
                      null => const SizedBox.shrink(),
                    },
                  ),
                ),
              ],
            ),
          ),
          _DetailsButton(card: card, focus: toggleFocus, onPressed: onDetails),
        ],
      ),
    );
  }
}

class _DetailsButton extends StatelessWidget {
  const _DetailsButton({
    required this.card,
    required this.focus,
    required this.onPressed,
  });

  final CardVisibility card;
  final FocusNode focus;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ListenableBuilder(
      listenable: card,
      builder: (context, _) {
        final shown = card.shown;
        return TapRegion(
          groupId: cardTapGroup,
          child: CommandHint(
            command: Command.sessionDetails,
            child: Tooltip(
              message: 'Toggle thread details',
              child: Semantics(
                button: true,
                selected: shown,
                child: InkWell(
                  key: const Key('session-card-toggle'),
                  focusNode: focus,
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
      },
    );
  }
}

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
    return Stack(
      clipBehavior: Clip.none,
      children: [
        child,
        Positioned(top: -11, right: -14, child: KeyBadge(label)),
      ],
    );
  }
}
