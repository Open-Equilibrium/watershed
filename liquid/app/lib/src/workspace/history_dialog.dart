import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/history.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'automations.dart' show DialogHeader;
import 'explorer.dart' show untitled;
import 'page_icon.dart';
import 'workspace_state.dart';

const historyWidth = 1060.0;
const _limit = 60;

String dayOf(DateTime at, DateTime now) {
  final today = DateUtils.dateOnly(now);
  final day = DateUtils.dateOnly(at);
  if (day == today) return 'Today';
  if (day == today.subtract(const Duration(days: 1))) return 'Yesterday';
  return DateFormat.yMMMd('en_US').format(at);
}

DateTime timeOf(BridgeEntry entry) =>
    DateTime.fromMillisecondsSinceEpoch(entry.at.toInt());

String actorName(WorkspaceState state, BridgeActor actor) =>
    switch (actor.kind) {
      BridgeActorKind.person => 'You',
      BridgeActorKind.automation =>
        state.automations.value
                .where((automation) => automation.id == actor.id)
                .firstOrNull
                ?.setup
                .name ??
            'An Automation',
    };

String originName(BridgeOrigin origin) => switch (origin) {
  BridgeOrigin.app => 'App',
  BridgeOrigin.api => 'API',
  BridgeOrigin.automation => 'Automation',
};

class HistoryDialog extends StatefulWidget {
  const HistoryDialog({super.key, required this.state, required this.onClose});

  final WorkspaceState state;
  final VoidCallback onClose;

  @override
  State<HistoryDialog> createState() => _HistoryDialogState();
}

class _HistoryDialogState extends State<HistoryDialog> {
  final _entries = <BridgeEntry>[];
  BridgeEntry? _chosen;
  String? _page;
  String? _automation;
  BridgeOrigin? _origin;
  bool _more = true;
  bool _reading = false;
  int _reads = 0;

  WorkspaceState get _state => widget.state;

  @override
  void initState() {
    super.initState();
    unawaited(_read(fresh: true));
  }

  Future<void> _read({bool fresh = false}) async {
    if (_reading && !fresh) return;
    final read = ++_reads;
    _reading = true;
    try {
      final page = await _state.workspace.history(
        asked: BridgeEntryQuery(
          page: _page,
          automation: _automation,
          origin: _origin,
          before: fresh ? null : _entries.lastOrNull?.actions.last,
          limit: _limit,
        ),
      );
      if (!mounted || read != _reads) return;
      setState(() {
        if (fresh) _entries.clear();
        _entries.addAll(page.entries);
        _more = page.more;
        _chosen = fresh
            ? _entries.firstOrNull
            : _chosen ?? _entries.firstOrNull;
      });
    } on Object catch (error) {
      _state.readFailed(error);
    } finally {
      if (read == _reads) _reading = false;
    }
  }

  Future<void> _revert(BridgeEntry entry) async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => ActionDialog(
        title: 'Revert “${entry.title}”?',
        bindings: {
          const SingleActivator(LogicalKeyboardKey.enter): () =>
              Navigator.of(context).pop(true),
        },
        actions: [
          PrimaryButton(
            label: 'Revert',
            onPressed: () => Navigator.of(context).pop(true),
          ),
        ],
        children: [
          Text(
            'This adds a new Action that undoes it. The original stays in '
            'History.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    final BridgeReverted reverted;
    try {
      reverted = await _state.workspace.revert(actions: entry.actions);
    } on Object catch (error) {
      _state.report('Liquid did not revert this: $error');
      return;
    }
    if (!mounted) return;
    if (reverted.purged) {
      _state.report(
        'Its content was deleted for good, so it cannot be reverted.',
      );
    } else if (reverted.conflict) {
      await _conflict(entry);
    }
    await _read(fresh: true);
  }

  Future<void> _conflict(BridgeEntry entry) async {
    final page = entry.page;
    final restore = await showDialog<bool>(
      context: context,
      builder: (_) => ActionDialog(
        title: 'Revert “${entry.title}”?',
        actions: [
          if (page != null)
            PrimaryButton(
              label: 'Restore version…',
              onPressed: () => Navigator.of(context).pop(true),
            ),
        ],
        children: [
          Text(
            'It cannot be reverted cleanly: a later change touched the same '
            'thing. Restore version returns the Page to how it was, and the '
            'later change stays in History.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (restore == true && page != null && mounted) {
      _state.pageHistory.value = page;
      widget.onClose();
    }
  }

  Future<void> _purgeTable(BridgeEntry entry) async {
    final table = entry.deletedTable;
    if (table == null) return;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => DangerDialog(
        title: 'Delete it for good?',
        action: 'Delete for good',
        children: [
          Text(
            'The deleted Table\'s rows and every earlier version of it leave '
            'this device for good. Its deletion can no longer be reverted, '
            'and History keeps its entries as “a deleted Table”.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    try {
      await _state.workspace.purge(table: table);
    } on Object catch (error) {
      _state.report('Liquid did not delete it for good: $error');
    }
    await _read(fresh: true);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): widget.onClose,
      },
      child: Stack(
        children: [
          Positioned.fill(
            child: GestureDetector(
              onTap: widget.onClose,
              child: ColoredBox(color: Colors.black.withValues(alpha: 0.18)),
            ),
          ),
          Center(
            child: Focus(
              autofocus: true,
              child: Container(
                key: const Key('history'),
                width: historyWidth,
                height: 640,
                decoration: BoxDecoration(
                  color: ws.surface,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
                  boxShadow: ws.shadow,
                ),
                clipBehavior: Clip.antiAlias,
                child: Material(
                  type: MaterialType.transparency,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      DialogHeader(
                        title: const Text('History'),
                        actions: const [],
                        onClose: widget.onClose,
                      ),
                      Expanded(
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            SizedBox(width: 430, child: _list(context)),
                            VerticalDivider(width: 1, color: ws.border),
                            Expanded(child: _detail(context)),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _filters(BuildContext context) {
    final pages = [
      for (final area in _state.areas.value)
        for (final page in _state.pages(area.id).value) page,
    ];
    Widget filter(
      String name,
      String label,
      List<(Widget, VoidCallback)> choices,
    ) => PopupMenuButton<VoidCallback>(
      key: Key('filter $name'),
      tooltip: '',
      onSelected: (choose) {
        choose();
        unawaited(_read(fresh: true));
      },
      itemBuilder: (_) => [
        for (final (child, choose) in choices)
          PopupMenuItem(value: choose, height: 32, child: child),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space2,
          vertical: WsMetrics.space1,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          spacing: WsMetrics.space1,
          children: [
            Text(label, style: const TextStyle(fontSize: WsMetrics.fsCompact)),
            const Icon(LucideIcons.chevronDown, size: 12),
          ],
        ),
      ),
    );
    String titled(String title) => title.isEmpty ? untitled : title;
    return Wrap(
      spacing: WsMetrics.space2,
      children: [
        filter(
          'page',
          switch (pages.where((page) => page.id == _page).firstOrNull) {
            final page? => titled(page.title),
            null => 'All Pages',
          },
          [
            (const Text('All Pages'), () => _page = null),
            for (final page in pages)
              (
                Row(
                  spacing: WsMetrics.space2,
                  children: [PageIcon(page.icon), Text(titled(page.title))],
                ),
                () => _page = page.id,
              ),
          ],
        ),
        filter(
          'actor',
          switch (_state.automations.value
              .where((automation) => automation.id == _automation)
              .firstOrNull) {
            final automation? => automation.setup.name,
            null => 'Anyone',
          },
          [
            (const Text('Anyone'), () => _automation = null),
            for (final automation in _state.automations.value)
              (Text(automation.setup.name), () => _automation = automation.id),
          ],
        ),
        filter(
          'origin',
          _origin == null ? 'Any origin' : originName(_origin!),
          [
            (const Text('Any origin'), () => _origin = null),
            for (final origin in BridgeOrigin.values)
              (Text(originName(origin)), () => _origin = origin),
          ],
        ),
      ],
    );
  }

  Widget _list(BuildContext context) {
    final ws = context.ws;
    final now = DateTime.now();
    final rows = <Widget>[];
    String? day;
    for (final entry in _entries) {
      final at = timeOf(entry);
      final shown = dayOf(at, now);
      if (shown != day) {
        day = shown;
        rows.add(
          Padding(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space4,
              WsMetrics.space4,
              WsMetrics.space4,
              WsMetrics.space1,
            ),
            child: Text(
              shown,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
        );
      }
      rows.add(
        _EntryRow(
          entry: entry,
          chosen: entry == _chosen,
          by: actorName(_state, entry.actor),
          onTap: () => setState(() => _chosen = entry),
        ),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(WsMetrics.space3),
          child: _filters(context),
        ),
        Divider(height: 1, color: ws.border),
        Expanded(
          child: NotificationListener<ScrollNotification>(
            onNotification: (scroll) {
              if (_more &&
                  scroll.metrics.pixels >=
                      scroll.metrics.maxScrollExtent - 200) {
                unawaited(_read());
              }
              return false;
            },
            child: ListView(children: rows),
          ),
        ),
      ],
    );
  }

  Widget _detail(BuildContext context) {
    final ws = context.ws;
    final entry = _chosen;
    if (entry == null) {
      return Center(
        child: Text(
          'Nothing changed yet.',
          style: TextStyle(color: ws.textFaint),
        ),
      );
    }
    final page = entry.page;
    final pageTitle = page == null
        ? null
        : [
            for (final area in _state.areas.value)
              for (final shown in _state.pages(area.id).value)
                if (shown.id == page) shown.title,
          ].firstOrNull;
    final at = timeOf(entry);
    final facts = [
      (
        'By',
        '${actorName(_state, entry.actor)}, ${switch (entry.origin) {
          BridgeOrigin.app => 'in the app',
          BridgeOrigin.api => 'through the API',
          BridgeOrigin.automation => 'as an Automation',
        }}',
      ),
      (
        'When',
        '${dayOf(at, DateTime.now())} ${DateFormat.Hm('en_US').format(at)}',
      ),
      ('Allowed by', entry.allowedBy),
      if (pageTitle != null) ('On', pageTitle.isEmpty ? untitled : pageTitle),
    ];
    return ListView(
      padding: const EdgeInsets.all(WsMetrics.space6),
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: WsMetrics.space3,
          children: [
            Expanded(
              child: Text(
                entry.title,
                style: TextStyle(
                  fontSize: WsMetrics.fsH3,
                  fontWeight: WsMetrics.fwSemibold,
                  color: ws.text,
                ),
              ),
            ),
            if (entry.deletedTable != null)
              SecondaryButton(
                label: 'Delete for good…',
                color: ws.danger,
                onPressed: () => unawaited(_purgeTable(entry)),
              ),
            SecondaryButton(
              label: 'Revert…',
              icon: LucideIcons.undo2,
              onPressed: entry.revertable
                  ? () => unawaited(_revert(entry))
                  : null,
            ),
          ],
        ),
        const SizedBox(height: WsMetrics.space5),
        for (final (label, value) in facts)
          Padding(
            padding: const EdgeInsets.only(bottom: WsMetrics.space2),
            child: Row(
              children: [
                SizedBox(
                  width: 120,
                  child: Text(label, style: TextStyle(color: ws.textMuted)),
                ),
                Expanded(child: Text(value)),
              ],
            ),
          ),
        if (entry.purged || (!entry.revertable && !entry.undone))
          Padding(
            padding: const EdgeInsets.only(top: WsMetrics.space4),
            child: Text(
              entry.purged
                  ? 'Its content was deleted for good, so it cannot be reverted.'
                  : 'This entry cannot be reverted.',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
        if (entry.undone)
          Padding(
            padding: const EdgeInsets.only(top: WsMetrics.space4),
            child: Text(
              'Undone by a later entry.',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
      ],
    );
  }
}

class _EntryRow extends StatelessWidget {
  const _EntryRow({
    required this.entry,
    required this.chosen,
    required this.by,
    required this.onTap,
  });

  final BridgeEntry entry;
  final bool chosen;
  final String by;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    return InkWell(
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.fromLTRB(
          WsMetrics.space4,
          WsMetrics.space2,
          WsMetrics.space3,
          WsMetrics.space2,
        ),
        decoration: BoxDecoration(
          color: chosen ? ws.selected : null,
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 48,
              child: Text(
                DateFormat.Hm('en_US').format(timeOf(entry)),
                style: faint,
              ),
            ),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 2,
                children: [
                  Text(
                    entry.title,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: entry.purged || entry.undone
                          ? ws.textFaint
                          : ws.text,
                    ),
                  ),
                  Row(
                    spacing: WsMetrics.space2,
                    children: [
                      Flexible(
                        child: Text(
                          '$by · ${originName(entry.origin)}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: faint,
                        ),
                      ),
                      if (entry.undone) Text('Undone', style: faint),
                    ],
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class UndoIntent extends Intent {
  const UndoIntent();
}

class RedoIntent extends Intent {
  const RedoIntent();
}

class LiveFocusNode extends FocusNode {
  LiveFocusNode({super.debugLabel, super.onKeyEvent});
}

class _HistoryAction<T extends Intent> extends Action<T> {
  _HistoryAction(this.run, this.active);

  final Future<void> Function() run;
  final bool Function() active;

  @override
  bool isEnabled(T intent) {
    if (!active()) return false;
    final focus = primaryFocus;
    final typing =
        focus?.context?.findAncestorWidgetOfExactType<EditableText>() != null;
    return !typing || focus is LiveFocusNode;
  }

  @override
  Object? invoke(T intent) {
    unawaited(run());
    return null;
  }
}

class UndoScope extends StatelessWidget {
  const UndoScope({
    super.key,
    required this.state,
    required this.active,
    required this.child,
  });

  final WorkspaceState? state;
  final bool Function() active;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final state = this.state;
    if (state == null) return child;
    bool paged() => active() && state.open.value != null;
    return Shortcuts(
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.keyZ, meta: true): UndoIntent(),
        SingleActivator(LogicalKeyboardKey.keyZ, meta: true, shift: true):
            RedoIntent(),
      },
      child: Actions(
        actions: {
          UndoIntent: _HistoryAction<UndoIntent>(state.undo, paged),
          RedoIntent: _HistoryAction<RedoIntent>(state.redo, paged),
        },
        child: child,
      ),
    );
  }
}
