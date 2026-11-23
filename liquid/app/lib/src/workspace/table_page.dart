import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart' show untitled;
import 'follow.dart';
import 'page_views.dart';
import 'stored_name.dart';
import 'table_block.dart';
import 'table_tabs.dart';
import 'roles.dart' show ProtectedMark, protectTable;
import 'table_view_parts.dart';
import 'workspace_state.dart';

class TablePage extends StatefulWidget {
  const TablePage({super.key, required this.state, required this.table});

  final WorkspaceState state;
  final String table;

  @override
  State<TablePage> createState() => _TablePageState();
}

typedef _Shown = ({String area, String page, String pageTitle, String title});

class _TablePageState extends State<TablePage> {
  late final _views = PageViews(widget.state, '');
  late final _name = StoredName(
    _table?.name ?? '',
    rename: (name) => widget.state.apply(
      mutation(
        BridgeMutationKind.renameDatabase,
        block: widget.table,
        text: name,
      ),
    ),
    restore: () async => mounted ? _table?.name ?? '' : null,
    quitting: () => widget.state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  final _nameFocus = FocusNode(debugLabel: 'Table name');
  final _nameHint = ValueNotifier(false);

  List<_Shown> _shown = const [];
  StreamSubscription<BridgeChange>? _followed;
  int _reads = 0;

  BridgeTable? get _table => widget.state.tables.value
      .where((table) => table.id == widget.table)
      .firstOrNull;

  @override
  void initState() {
    super.initState();
    widget.state.tables.addListener(_tablesChanged);
    widget.state.finishing.add(_name.typed);
    _nameFocus.addListener(_focusChanged);
    _followed = follow(
      widget.state.workspace,
      BridgeResource(kind: BridgeResourceKind.table, block: widget.table),
    ).listen((_) => unawaited(_readShown()));
  }

  void _tablesChanged() {
    if (!mounted) return;
    if (!_nameFocus.hasFocus) _name.show(_table?.name ?? '');
    setState(() {});
  }

  void _focusChanged() {
    _nameHint.value = _nameFocus.hasFocus;
    if (_nameFocus.hasFocus) return;
    _name.write();
  }

  Future<void> _readShown() async {
    final read = ++_reads;
    try {
      final answer = await widget.state.read(
        query(BridgeQueryKind.tableViews, block: widget.table),
      );
      final shown = <_Shown>[];
      for (final at in answer.shown) {
        final (area, title) = _pageOf(at.page);
        if (area == null) continue;
        shown.add((
          area: area,
          page: at.page,
          pageTitle: title,
          title: at.title,
        ));
      }
      if (mounted && read == _reads) setState(() => _shown = shown);
    } on Object catch (error) {
      if (mounted) widget.state.readFailed(error);
    }
  }

  (String?, String) _pageOf(String page) {
    for (final area in widget.state.areas.value) {
      final found = widget.state
          .pages(area.id)
          .value
          .where((shown) => shown.id == page)
          .firstOrNull;
      if (found != null) return (area.id, found.title);
    }
    return (null, '');
  }

  Future<void> _purge() async {
    final name = _named(_table?.name ?? '');
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => DangerDialog(
        title: 'Delete the earlier versions of “$name”?',
        action: 'Delete earlier versions',
        children: [
          Text(
            'The Table stays as it is now. Its earlier versions, and the '
            'rows and values only they still hold, leave this device for '
            'good. History keeps its entries without their content, and none '
            'of them can be reverted afterwards.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    try {
      await widget.state.workspace.purge(table: widget.table);
    } on Object catch (error) {
      widget.state.report('Liquid did not delete the earlier versions: $error');
    }
  }

  Future<void> _delete() async {
    final name = _table?.name ?? '';
    final pages = {for (final shown in _shown) _named(shown.pageTitle)};
    final rows = (await widget.state.read(
      query(BridgeQueryKind.rows, block: widget.table, limit: 0),
    )).count;
    if (!mounted) return;
    final views = _shown.length;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (_) => DangerDialog(
        title: 'Delete the Table “${_named(name)}”?',
        action: 'Delete Table',
        children: [
          Text(
            views == 0
                ? 'Its ${rowCount(rows)} go with it.'
                : 'Its ${rowCount(rows)} go with it, and so do its '
                      '$views ${views == 1 ? 'View' : 'Views'} on '
                      '${_list(pages.toList())}.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    await widget.state.deleteTable(widget.table);
  }

  @override
  void dispose() {
    widget.state.tables.removeListener(_tablesChanged);
    widget.state.finishing.remove(_name.typed);
    _name.write();
    unawaited(_followed?.cancel());
    _name.dispose();
    _nameFocus.dispose();
    _nameHint.dispose();
    _views.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final table = _table;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ValueListenableBuilder(
          valueListenable: widget.state.problem,
          builder: (context, problem, _) => problem == null
              ? const SizedBox.shrink()
              : Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space7,
                  ),
                  child: WarningNotice(problem, danger: true),
                ),
        ),
        Expanded(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space7,
              28,
              WsMetrics.space7,
              WsMetrics.space8,
            ),
            child: table == null
                ? const TablePlaceholder('This Table was deleted.')
                : Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                TextField(
                                  key: const Key('table page name'),
                                  controller: _name.take(),
                                  focusNode: _nameFocus,
                                  style: TextStyle(
                                    fontSize: WsMetrics.fsH1,
                                    height: WsMetrics.lhH1 / WsMetrics.fsH1,
                                    fontWeight: WsMetrics.fwBold,
                                    color: ws.text,
                                  ),
                                  decoration: InputDecoration.collapsed(
                                    hintText: untitled,
                                    hintStyle: TextStyle(color: ws.textFaint),
                                  ),
                                ),
                                ValueListenableBuilder(
                                  valueListenable: _nameHint,
                                  builder: (context, shown, _) =>
                                      shown && _shown.isNotEmpty
                                      ? Padding(
                                          padding: const EdgeInsets.only(
                                            top: WsMetrics.space2,
                                          ),
                                          child: Text(
                                            _shown.length == 1
                                                ? 'Renames the Table in its View'
                                                : 'Renames the Table in all ${_shown.length} Views',
                                            style: TextStyle(
                                              fontSize: WsMetrics.fsSmall,
                                              color: ws.textMuted,
                                            ),
                                          ),
                                        )
                                      : const SizedBox.shrink(),
                                ),
                              ],
                            ),
                          ),
                          if (table.protected)
                            const Padding(
                              padding: EdgeInsets.only(
                                top: WsMetrics.space3,
                                right: WsMetrics.space3,
                              ),
                              child: ProtectedMark(),
                            ),
                          MoreMenu(
                            label: 'Table actions',
                            plain: [
                              if (table.protected)
                                (
                                  LucideIcons.lockOpen,
                                  'Stop protecting',
                                  () => unawaited(
                                    widget.state
                                        .apply(
                                          mutation(
                                            BridgeMutationKind.unprotectTable,
                                            block: widget.table,
                                          ),
                                        )
                                        .then<void>((_) {}, onError: (_) {}),
                                  ),
                                )
                              else
                                (
                                  LucideIcons.lock,
                                  'Protect…',
                                  () => unawaited(
                                    protectTable(
                                      context,
                                      widget.state,
                                      widget.table,
                                      table.name,
                                    ),
                                  ),
                                ),
                            ],
                            items: [
                              (
                                LucideIcons.trash,
                                'Delete earlier versions…',
                                () => unawaited(_purge()),
                              ),
                              (LucideIcons.trash, 'Delete Table…', _delete),
                            ],
                          ),
                        ],
                      ),
                      const SizedBox(height: WsMetrics.space4),
                      if (_shown.isNotEmpty) ...[
                        _ShownIn(
                          shown: _shown,
                          onOpen: (shown) => widget.state.open.value = OpenPage(
                            shown.area,
                            shown.page,
                          ),
                        ),
                        const SizedBox(height: WsMetrics.space4),
                      ],
                      TableBlock(
                        views: _views,
                        fullPage: true,
                        view: BridgeView(
                          id: 'table ${widget.table}',
                          block: widget.table,
                          title: table.name,
                          boundary: false,
                          kind: BridgeBlockKind.database,
                          tabs: [tabOfLayout(BridgeLayout.table)],
                        ),
                      ),
                    ],
                  ),
          ),
        ),
      ],
    );
  }
}

String _named(String name) => name.isEmpty ? untitled : name;

String _list(List<String> names) => switch (names) {
  [] => '',
  [final one] => one,
  [...final rest, final last] => '${rest.join(', ')} and $last',
};

class _ShownIn extends StatelessWidget {
  const _ShownIn({required this.shown, required this.onOpen});

  final List<_Shown> shown;
  final void Function(_Shown shown) onOpen;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      key: const Key('shown in'),
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: WsMetrics.space2,
              vertical: WsMetrics.space1,
            ),
            child: Text(
              'Shown in',
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
          for (final at in shown)
            InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: () => onOpen(at),
              child: Container(
                height: 32,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        _named(at.title),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.text,
                        ),
                      ),
                    ),
                    const SizedBox(width: WsMetrics.space3),
                    Flexible(
                      child: Text(
                        _named(at.pageTitle),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: WsMetrics.fsSmall,
                          color: ws.textFaint,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}
