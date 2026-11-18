import 'package:flutter/material.dart';

import '../diff/turn_scope.dart';
import '../rust/api/details.dart';
import '../session/session_statistics.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'changes_row.dart';
import 'card_automations.dart';
import 'card_environment.dart';
import 'card_lineage.dart';
import 'card_servers.dart';
import 'card_usage.dart';
import 'git_actions.dart';
import 'pull_request_rows.dart';
import 'session_card_layout.dart';
import 'session_card_rows.dart';
import 'session_card_settings.dart';
import 'session_card_statistics.dart';

class SessionCard extends StatefulWidget {
  const SessionCard({
    super.key,
    required this.state,
    required this.settings,
    required this.onChanges,
    required this.beside,
    required this.onOutsidePress,
  });

  final SessionState state;
  final SessionCardSettingsController settings;

  final ValueChanged<DiffScope> onChanges;

  final bool beside;

  final VoidCallback onOutsidePress;

  @override
  State<SessionCard> createState() => _SessionCardState();
}

class _SessionCardState extends State<SessionCard> {
  final _contentKey = GlobalKey(debugLabel: 'session card content');

  final _menu = MenuController();

  final _menuFirst = FocusNode(debugLabel: 'rows menu');

  @override
  void dispose() {
    _menuFirst.dispose();
    super.dispose();
  }

  Map<CardDensity, double> _heights = const {};
  String? _measuredFor;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final route = ModalRoute.of(context);
    void leave() {
      final coveredByRoute = route != null && !route.isCurrent;
      if (!widget.beside && !_menu.isOpen && !coveredByRoute) {
        widget.onOutsidePress();
      }
    }

    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onFocusChange: (hasFocus) {
        if (!hasFocus) leave();
      },
      child: TapRegion(
        groupId: cardTapGroup,
        onTapOutside: (_) => leave(),
        child: DecoratedBox(
          key: const Key('session-card-frame'),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
            boxShadow: ws.shadow,
          ),
          child: Material(
            color: ws.surface,
            borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
            clipBehavior: Clip.antiAlias,
            child: LayoutBuilder(
              builder: (context, box) => ListenableBuilder(
                listenable: widget.state.git,
                builder: (context, _) {
                  final git = widget.state.git.value;
                  return ListenableBuilder(
                    listenable: Listenable.merge([
                      widget.settings,
                      widget.state.recent,
                      widget.state.turns,
                      widget.state.statistics,
                      widget.state.status,
                      widget.state.contextWindow,
                      widget.state.servers,
                      widget.state.automations,
                      widget.state.lineage,
                      widget.state.limits,
                      widget.state.hostName,
                      git.status,
                      git.problem,
                      git.running,
                    ]),
                    builder: (context, _) => _content(context, box.maxHeight),
                  );
                },
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _content(BuildContext context, double maxHeight) {
    final state = widget.state;
    final shown = widget.settings.value;
    final git = state.git.value;
    final status = git.status.value;
    final problem = git.problem.value;
    final host = state.hostName.value;
    final hasWorkspace =
        shown.shows(CardItem.workspace) &&
        state.session != null &&
        host != null &&
        host.isNotEmpty;
    final hasBranch =
        shown.shows(CardItem.branch) && (status != null || problem != null);
    final hasPull = shown.shows(CardItem.branch) && status != null;
    final hasGit = shown.shows(CardItem.gitActions) && status != null;
    final hasChanges =
        shown.shows(CardItem.changes) && (status?.repository ?? false);
    final servers = shown.shows(CardItem.servers)
        ? state.servers.value
        : const <BridgeLocalServer>[];
    final automations = shown.shows(CardItem.automations)
        ? state.automations.value
        : const <BridgeAutomation>[];
    final lineage = state.lineage.value;
    final hasLineage =
        shown.shows(CardItem.lineage) &&
        lineage != null &&
        showsLineage(lineage);
    final hasSideChats =
        shown.shows(CardItem.sideChats) &&
        lineage != null &&
        showsSideChats(lineage);
    final hasStatistics = shown.shows(CardItem.statistics);
    final limits = shown.shows(CardItem.usage) ? state.limits.value : null;
    final hasUsage = limits != null && showsLimits(limits);
    final window = shown.shows(CardItem.context)
        ? state.contextWindow.value
        : null;
    final signature = [
      hasWorkspace,
      hasBranch,
      hasPull,
      hasGit,
      hasChanges,
      servers.isNotEmpty,
      automations.isNotEmpty,
      hasLineage,
      hasSideChats,
      hasStatistics,
      hasUsage,
      window != null,
    ].join(' ');
    final measured = _measuredFor == signature
        ? _heights
        : const <CardDensity, double>{};
    final density = resolveCardDensity(
      maxHeight,
      full: measured[CardDensity.full] ?? 0,
      compact: measured[CardDensity.compact] ?? 0,
    );
    _measure(density, signature);
    final full = density == CardDensity.full;
    final showGit = hasGit && (full || git.running.value != null);
    final versionControl = [
      if (hasBranch) BranchRow(state: state),
      if (hasPull) PullRequestRows(state: state),
      if (showGit) GitActionsControl(state: state),
      if (hasChanges) ChangesRow(state: state, onOpen: widget.onChanges),
    ];
    final anyRow =
        hasWorkspace ||
        hasBranch ||
        hasPull ||
        hasGit ||
        hasChanges ||
        servers.isNotEmpty ||
        automations.isNotEmpty ||
        hasLineage ||
        hasSideChats ||
        hasStatistics ||
        hasUsage ||
        window != null;

    final blocks = <Widget>[];
    void add(Widget Function(bool divided) section) =>
        blocks.add(section(blocks.isNotEmpty));
    if (full && host != null && hasWorkspace) {
      add(
        (divided) => CardSection(
          divided: divided,
          children: [EnvironmentRow(state: state, host: host)],
        ),
      );
    }
    if (versionControl.isNotEmpty) {
      add((divided) => CardSection(divided: divided, children: versionControl));
    }
    if (full && servers.isNotEmpty) {
      add(
        (divided) =>
            ServersSection(state: state, servers: servers, divided: divided),
      );
    }
    if (full && automations.isNotEmpty) {
      add(
        (divided) => AutomationsSection(
          state: state,
          tasks: automations,
          divided: divided,
        ),
      );
    }
    if (full && lineage != null && hasLineage) {
      add(
        (divided) =>
            LineageSection(state: state, lineage: lineage, divided: divided),
      );
    }
    if (full && lineage != null && hasSideChats) {
      add(
        (divided) =>
            SideChatsSection(state: state, lineage: lineage, divided: divided),
      );
    }
    if (hasStatistics) {
      add((divided) => _StatisticsSection(state: state, divided: divided));
    }
    if (limits != null && hasUsage) {
      add((divided) => UsageSection(limits: limits, divided: divided));
    }
    if (window != null) {
      add(
        (divided) => CardSection(
          divided: divided,
          children: [ContextRow(window: window)],
        ),
      );
    }
    if (!anyRow) {
      add(
        (divided) => CardSection(
          divided: divided,
          children: [_ChooseRow(onOpen: _openMenuFrom)],
        ),
      );
    }

    return MenuAnchor(
      controller: _menu,
      style: MenuStyle(
        backgroundColor: WidgetStatePropertyAll(
          Theme.of(context).colorScheme.surface,
        ),
      ),
      menuChildren: threadDetailsMenuItems(
        context,
        widget.settings,
        first: _menuFirst,
      ),
      builder: (context, controller, child) => GestureDetector(
        onSecondaryTapUp: (details) => _openMenu(details.localPosition),
        child: child,
      ),
      child: _ScrollFade(
        child: ListView(
          shrinkWrap: true,
          padding: EdgeInsets.zero,
          children: [
            Column(
              key: _contentKey,
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: blocks,
            ),
          ],
        ),
      ),
    );
  }

  void _measure(CardDensity density, String signature) {
    if (density == CardDensity.essential) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final height = _contentKey.currentContext?.size?.height;
      if (!mounted || height == null) return;
      final current = _measuredFor == signature
          ? _heights
          : const <CardDensity, double>{};
      if (_measuredFor == signature && current[density] == height) return;
      setState(() {
        _measuredFor = signature;
        _heights = {...current, density: height};
      });
    });
  }

  void _openMenu(Offset position) {
    _menu.open(position: position);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_menu.isOpen) _menuFirst.requestFocus();
    });
  }

  void _openMenuFrom(BuildContext row) {
    final box = row.findRenderObject()! as RenderBox;
    final card = context.findRenderObject()! as RenderBox;
    _openMenu(
      card.globalToLocal(box.localToGlobal(Offset(0, box.size.height))),
    );
  }
}

class _ChooseRow extends StatelessWidget {
  const _ChooseRow({required this.onOpen});

  final void Function(BuildContext row) onOpen;

  @override
  Widget build(BuildContext context) => CardRow(
    key: const Key('choose-rows'),
    onTap: () => onOpen(context),
    children: [
      Icon(
        LucideIcons.slidersHorizontal,
        size: 16,
        color: context.ws.textFaint,
      ),
      const Flexible(
        child: Text('Choose what to show', overflow: TextOverflow.ellipsis),
      ),
    ],
  );
}

class _ScrollFade extends StatefulWidget {
  const _ScrollFade({required this.child});

  final Widget child;

  @override
  State<_ScrollFade> createState() => _ScrollFadeState();
}

class _ScrollFadeState extends State<_ScrollFade> {
  static const _depth = 24.0;

  bool _above = false;
  bool _below = false;

  @override
  Widget build(BuildContext context) {
    final child = NotificationListener<Notification>(
      onNotification: (notification) {
        final metrics = switch (notification) {
          ScrollMetricsNotification(:final metrics) => metrics,
          ScrollNotification(:final metrics) => metrics,
          _ => null,
        };
        if (metrics != null) {
          final above = metrics.extentBefore > 0;
          final below = metrics.extentAfter > 0;
          if (above != _above || below != _below) {
            setState(() {
              _above = above;
              _below = below;
            });
          }
        }
        return false;
      },
      child: widget.child,
    );
    if (!_above && !_below) return child;
    return ShaderMask(
      blendMode: BlendMode.dstIn,
      shaderCallback: (bounds) {
        final depth = (_depth / bounds.height).clamp(0.0, 0.5);
        return LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [
            _above ? Colors.transparent : Colors.black,
            Colors.black,
            Colors.black,
            _below ? Colors.transparent : Colors.black,
          ],
          stops: [0, depth, 1 - depth, 1],
        ).createShader(bounds);
      },
      child: child,
    );
  }
}

class _StatisticsSection extends StatelessWidget {
  const _StatisticsSection({required this.state, required this.divided});

  final SessionState state;
  final bool divided;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([state.statistics, state.status, state.turns]),
    builder: (context, _) {
      final ws = context.ws;
      final stored = state.statistics.value;
      return CardSection(
        divided: divided,
        children: [
          if (stored != null)
            ...sessionStatisticsRows(
              context,
              SessionStatistics(stored, running: turnRuns(state)),
            ),
          CardRow(
            onTap: () => state.statisticsOpen.value = true,
            children: [
              Icon(LucideIcons.barChart3, size: 16, color: ws.textFaint),
              const Text('Statistics'),
              const Spacer(),
              Text(
                Keymap.of(context).label(Command.statistics),
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  fontWeight: WsMetrics.fwRegular,
                  color: ws.textFaint,
                ),
              ),
            ],
          ),
        ],
      );
    },
  );
}
