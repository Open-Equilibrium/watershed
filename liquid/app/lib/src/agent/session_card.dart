import 'dart:async';

import 'package:flutter/material.dart';

import '../diff/turn_scope.dart';
import '../rust/api/history.dart';
import '../session/session_figures.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'git_actions.dart';
import 'pull_request_status.dart';
import 'session_card_figures.dart';
import 'session_card_layout.dart';
import 'session_card_rows.dart';
import 'session_card_settings.dart';

/// The session card's rows, in T3 Code's order, shrunk to the space they have: the Version
/// Control rows, then the session figures and the context window below a divider. A row is left
/// out when Liquid has nothing for it, and the Git action row leaves a compact card as T3 Code's
/// does.
class SessionCard extends StatefulWidget {
  const SessionCard({
    super.key,
    required this.state,
    required this.device,
    required this.settings,
    required this.onChanges,
    required this.beside,
    required this.onOutsidePress,
  });

  final SessionState state;
  final String device;
  final SessionCardSettingsController settings;
  final VoidCallback onChanges;

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

  /// What changes the rows shown; one listenable, so a rebuild does not subscribe again.
  late Listenable _changes = _changesOf(widget);

  static Listenable _changesOf(SessionCard widget) => Listenable.merge([
    widget.settings,
    widget.state.git.status,
    widget.state.git.problem,
    widget.state.git.loaded,
    widget.state.git.progress,
    widget.state.pulls.current,
    widget.state.statistics,
    widget.state.contextWindow,
  ]);

  @override
  void didUpdateWidget(SessionCard old) {
    super.didUpdateWidget(old);
    if (old.state != widget.state || old.settings != widget.settings) {
      _changes = _changesOf(widget);
    }
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
          key: const Key('session-details-card'),
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
                listenable: _changes,
                builder: (context, _) => _content(context, box.maxHeight),
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
    final status = state.git.status.value;
    final problem = state.git.problem.value;
    final hasBranch =
        shown.shows(CardItem.branch) && (status != null || problem != null);
    final hasPull =
        shown.shows(CardItem.branch) && state.pulls.current.value != null;
    final hasGit =
        shown.shows(CardItem.gitActions) &&
        state.git.loaded.value &&
        problem == null;
    final hasChanges = shown.shows(CardItem.changes) && status != null;
    final window = shown.shows(CardItem.context)
        ? state.contextWindow.value
        : null;
    final hasFigures = shown.shows(CardItem.figures);
    final hasUsage = hasFigures && state.statistics.value != null;
    // Measurements hold only while the same rows show.
    final signature =
        '$hasBranch $hasPull $hasGit $hasChanges ${window != null} $hasUsage';
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
        hasGit &&
        (density == CardDensity.full || state.git.progress.value != null);
    // A card with no row left to show keeps one row that opens the rows menu, to refill it.
    final anyRow =
        hasBranch ||
        hasPull ||
        hasGit ||
        hasChanges ||
        hasFigures ||
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
                _Section(
                  rows: [
                    if (hasBranch) BranchRow(state: state),
                    if (hasPull)
                      PullRequestRow(state: state, device: widget.device),
                    if (showGit)
                      GitActions(state: state, device: widget.device),
                    if (hasChanges)
                      _ChangesRow(state: state, onTap: widget.onChanges),
                    if (!anyRow) _ChooseRow(onOpen: _openMenuFrom),
                  ],
                ),
                if (window != null) _ContextWindow(window: window),
                if (hasFigures)
                  _SessionFigures(state: state, divided: window == null),
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

/// The session figures, with the Statistics row that opens the panel. The figures wait for the
/// first read of the statistics; `divided` is false when the context window above draws the
/// divider.
class _SessionFigures extends StatelessWidget {
  const _SessionFigures({required this.state, required this.divided});

  final SessionState state;
  final bool divided;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([state.statistics, state.status]),
    builder: (context, _) {
      final statistics = state.statistics.value;
      final running = state.status.value == RunStatus.running;
      return _Section(
        divided: divided,
        rows: [
          if (statistics != null)
            ...sessionFigureRows(
              context,
              SessionFigures(statistics, running: running),
            ),
          StatisticsRow(onTap: () => state.statisticsOpen.value = true),
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
      _Section(divided: true, rows: [ContextFigure(window: window)]);
}

/// The Changes row: the latest finished turn's lines, which opens the Diff panel.
class _ChangesRow extends StatelessWidget {
  const _ChangesRow({required this.state, required this.onTap});

  final SessionState state;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      fontWeight: WsMetrics.fwRegular,
    );
    return ValueListenableBuilder(
      valueListenable: state.turns,
      builder: (context, turns, _) {
        final latest = finished(turns).lastOrNull;
        final numbers = monoStyle(context).copyWith(
          fontSize: WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwRegular,
        );
        return CardRow(
          key: const Key('changes-row'),
          onTap: onTap,
          children: [
            Icon(LucideIcons.fileDiff, size: 16, color: ws.textFaint),
            const Text('Changes'),
            Flexible(
              child: Text(
                'latest turn',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: small.copyWith(color: ws.textMuted),
              ),
            ),
            Text(
              '+${latest?.added ?? 0}',
              style: numbers.copyWith(color: ws.success),
            ),
            Text(
              '−${latest?.removed ?? 0}',
              style: numbers.copyWith(color: ws.danger),
            ),
          ],
        );
      },
    );
  }
}
