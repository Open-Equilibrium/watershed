import 'dart:async';

import 'package:flutter/material.dart';

import '../diff/turn_scope.dart';
import '../rust/api/git.dart';
import '../rust/api/thread.dart';
import '../session/session_statistics.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'git_actions.dart';
import 'pull_request_rows.dart';
import 'session_card_layout.dart';
import 'session_card_rows.dart';
import 'session_card_settings.dart';
import 'session_card_statistics.dart';

/// The session card's rows, in T3 Code's order, shrunk to the space they have: the workspace, the
/// Version Control rows, then the context window and the session statistics below a divider. A row
/// is left out when Liquid has nothing for it, and the Git action row leaves a compact card as T3
/// Code's does.
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

  /// Opens the Diff panel with the scope the Changes row names.
  final ValueChanged<DiffScope> onChanges;

  /// Whether the card sits beside the chat rather than over it as a popover.
  final bool beside;

  /// Closes the card when it is a popover and a press lands outside it.
  final VoidCallback onOutsidePress;

  @override
  State<SessionCard> createState() => _SessionCardState();
}

class _SessionCardState extends State<SessionCard> {
  final _contentKey = GlobalKey(debugLabel: 'session card content');

  /// The rows menu, which a right-click or the choose row opens.
  final _menu = MenuController();

  /// The menu's first row, which takes focus when the menu opens, so Escape and the arrow keys
  /// work in it.
  final _menuFirst = FocusNode(debugLabel: 'rows menu');

  @override
  void dispose() {
    _menuFirst.dispose();
    super.dispose();
  }

  /// The content's height at each density, measured for the rows named by `_measuredFor`.
  Map<CardDensity, double> _heights = const {};
  String? _measuredFor;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final route = ModalRoute.of(context);
    // A popover closes when a press or the focus leaves it, as T3 Code's does. The rows menu and
    // a dialog the card opened take their own presses and focus.
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
    final hasWorkspace =
        shown.shows(CardItem.workspace) && state.session != null;
    final hasBranch =
        shown.shows(CardItem.branch) && (status != null || problem != null);
    final hasPull = shown.shows(CardItem.branch) && status != null;
    final hasGit = shown.shows(CardItem.gitActions) && status != null;
    // The Changes row names the uncommitted changes only; it is left out while there are none.
    final uncommitted = status?.changes ?? const [];
    final hasChanges = shown.shows(CardItem.changes) && uncommitted.isNotEmpty;
    final window = shown.shows(CardItem.context)
        ? state.contextWindow.value
        : null;
    final hasStatistics = shown.shows(CardItem.statistics);
    final hasUsage = hasStatistics && state.statistics.value != null;
    // Measurements hold only while the same rows show.
    final signature =
        '$hasWorkspace $hasBranch $hasPull $hasGit $hasChanges '
        '${window != null} $hasUsage';
    final measured = _measuredFor == signature
        ? _heights
        : const <CardDensity, double>{};
    final density = resolveCardDensity(
      maxHeight,
      full: measured[CardDensity.full] ?? 0,
      compact: measured[CardDensity.compact] ?? 0,
    );
    _measure(density, signature);
    // T3 Code keeps the Git action row out of a compact card unless an action runs.
    final showGit =
        hasGit && (density == CardDensity.full || git.running.value != null);
    // A card with no row left to show keeps one row that opens the rows menu, to refill it.
    final anyRow =
        hasWorkspace ||
        hasBranch ||
        hasPull ||
        hasGit ||
        hasChanges ||
        hasStatistics ||
        window != null;
    return MenuAnchor(
      controller: _menu,
      style: MenuStyle(
        backgroundColor: WidgetStatePropertyAll(
          Theme.of(context).colorScheme.surface,
        ),
      ),
      menuChildren: _menuChildren(),
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
              children: [
                _Section(rows: [if (hasWorkspace) _WorkspaceRow(state: state)]),
                _Section(
                  divided: hasWorkspace,
                  rows: [
                    if (hasBranch) BranchRow(state: state),
                    if (hasPull) PullRequestRows(state: state),
                    if (showGit) GitActionsControl(state: state),
                    if (status case final status? when hasChanges)
                      _ChangesRow(status: status, onTap: widget.onChanges),
                    if (!anyRow) _ChooseRow(onOpen: _openMenuFrom),
                  ],
                ),
                if (window != null) _ContextWindow(window: window),
                if (hasStatistics)
                  _StatisticsSection(state: state, divided: window == null),
              ],
            ),
          ],
        ),
      ),
    );
  }

  /// Measures the content at `density` after the frame, so the next build knows whether it fits.
  /// The essential rows are not measured: nothing is left to drop below them.
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

  /// The rows menu: a checkbox per row, which a click shows or hides and leaves the menu open,
  /// then Reset to default, which closes it.
  List<Widget> _menuChildren() {
    final settings = widget.settings;
    return [
      for (final item in CardItem.values)
        CheckboxMenuButton(
          focusNode: item == CardItem.values.first ? _menuFirst : null,
          value: settings.value.shows(item),
          closeOnActivate: false,
          onChanged: (shown) => unawaited(
            settings.change(settings.value.withItem(item, shown ?? true)),
          ),
          child: Text(item.label),
        ),
      const Divider(),
      MenuItemButton(
        onPressed: settings.value.isDefault
            ? null
            : () => unawaited(settings.change(const SessionCardSettings())),
        child: const Text('Reset to default'),
      ),
    ];
  }

  /// Opens the rows menu at `position`, in the card's coordinates, and moves focus into it.
  void _openMenu(Offset position) {
    _menu.open(position: position);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_menu.isOpen) _menuFirst.requestFocus();
    });
  }

  /// Opens the rows menu below the row `row` is in.
  void _openMenuFrom(BuildContext row) {
    final box = row.findRenderObject()! as RenderBox;
    final card = context.findRenderObject()! as RenderBox;
    _openMenu(
      card.globalToLocal(box.localToGlobal(Offset(0, box.size.height))),
    );
  }
}

/// The row that opens the rows menu when no row is left to show, so the card can be refilled.
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

/// Fades the edge the card's rows scroll past, as T3 Code's card does: the top once the rows are
/// scrolled down, the bottom while more lie below. Rows that fit draw without a fade.
class _ScrollFade extends StatefulWidget {
  const _ScrollFade({required this.child});

  final Widget child;

  @override
  State<_ScrollFade> createState() => _ScrollFadeState();
}

class _ScrollFadeState extends State<_ScrollFade> {
  /// The fade's depth, as T3 Code's card has it: 1.5 rem.
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

/// A section of the card: its rows in a padded block, below a hairline when `divided`. A section
/// without rows draws nothing.
class _Section extends StatelessWidget {
  const _Section({this.divided = false, required this.rows});

  final bool divided;
  final List<Widget> rows;

  @override
  Widget build(BuildContext context) {
    if (rows.isEmpty) return const SizedBox.shrink();
    return DecoratedBox(
      decoration: BoxDecoration(
        border: divided
            ? Border(top: BorderSide(color: context.ws.border))
            : null,
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: rows,
        ),
      ),
    );
  }
}

/// The workspace row: the checkout the open session works in, as T3 Code's workspace section names
/// it.
class _WorkspaceRow extends StatelessWidget {
  const _WorkspaceRow({required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: state.recent,
      builder: (context, threads, _) {
        final worktree = threads
            .where((thread) => thread.id == state.session?.id)
            .firstOrNull
            ?.worktree;
        // T3 names a worktree by its folder, and a checkout without one as the local checkout.
        final folder = worktree
            ?.split('/')
            .where((part) => part.isNotEmpty)
            .lastOrNull;
        return CardRow(
          key: const Key('workspace-row'),
          children: [
            Icon(
              folder == null ? LucideIcons.folder : LucideIcons.gitBranch,
              size: 16,
              color: ws.textFaint,
            ),
            Flexible(
              child: Text(
                folder ?? 'Local checkout',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        );
      },
    );
  }
}

/// The session statistics rows, with the Statistics row that opens the panel. The rows wait for the
/// first read of the statistics; `divided` is false when the context window above draws the
/// divider.
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
      return _Section(
        divided: divided,
        rows: [
          if (stored != null)
            ...sessionStatisticsRows(
              context,
              SessionStatistics(stored, running: turnRuns(state)),
            ),
          // The Statistics row opens the panel, and names the key that opens it.
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

/// The context window, below a divider.
class _ContextWindow extends StatelessWidget {
  const _ContextWindow({required this.window});

  final BridgeContext window;

  @override
  Widget build(BuildContext context) =>
      _Section(divided: true, rows: [ContextRow(window: window)]);
}

/// The Changes row: the uncommitted changes and their lines. Its tap opens the Diff panel at them.
class _ChangesRow extends StatelessWidget {
  const _ChangesRow({required this.status, required this.onTap});

  final BridgeGitStatus status;
  final ValueChanged<DiffScope> onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final added = status.changes.fold(0, (sum, file) => sum + file.added);
    final removed = status.changes.fold(0, (sum, file) => sum + file.removed);
    final numbers = monoStyle(context)
        .copyWith(fontSize: WsMetrics.fsSmall, fontWeight: WsMetrics.fwRegular);
    return CardRow(
      key: const Key('changes-row'),
      onTap: () => onTap(const UncommittedChanges()),
      children: [
        Icon(LucideIcons.fileDiff, size: 16, color: ws.textFaint),
        const Text('Changes'),
        Flexible(
          child: Text(
            'Uncommitted',
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwRegular,
              color: ws.textMuted,
            ),
          ),
        ),
        Text('+$added', style: numbers.copyWith(color: ws.success)),
        Text('−$removed', style: numbers.copyWith(color: ws.danger)),
      ],
    );
  }
}
