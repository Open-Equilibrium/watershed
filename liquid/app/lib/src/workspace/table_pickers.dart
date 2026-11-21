import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'cell_pickers.dart' show shownDays;
import 'explorer.dart' show untitled;
import 'table_query.dart' show filterLabel;
import 'workspace_state.dart';

typedef ExistingView = ({
  String table,
  String title,
  BridgeSort? sort,
  BridgeFilter? filter,
});

typedef _Setup = ({BridgeSort? sort, BridgeFilter? filter});

String _named(String name) => name.isEmpty ? untitled : name;

Future<ExistingView?> pickExistingTable(
  BuildContext context,
  WorkspaceState state,
) async {
  final tables = state.tables.value;
  if (tables.isEmpty) {
    state.report('There is no Table yet; choose Table to make one.');
    return null;
  }
  final table = await showDialog<BridgeTable>(
    context: context,
    builder: (_) => _ChooseTable(tables: tables),
  );
  if (table == null || !context.mounted) return null;
  final shown = (await state.read(
    query(BridgeQueryKind.tableViews, block: table.id),
  )).shown;
  final plain = (table: table.id, title: table.name, sort: null, filter: null);
  final setups = [
    for (final view in shown)
      if (view.sort != null || view.filter != null) view,
  ];
  if (setups.isEmpty) return plain;
  final fields =
      (await state.read(query(BridgeQueryKind.database, block: table.id)))
          .database
          ?.fields ??
      const <BridgeField>[];
  if (!context.mounted) return null;
  final pages = {
    for (final area in state.areas.value)
      for (final page in state.pages(area.id).value) page.id: page.title,
  };
  final locale = Localizations.localeOf(context).toString();
  final setup = await showDialog<_Setup>(
    context: context,
    builder: (_) => _AddView(
      name: table.name,
      views: [
        for (final view in setups)
          (
            title: view.title,
            page: _named(pages[view.page] ?? ''),
            setup: _describe(view, fields, locale),
            chosen: (sort: view.sort, filter: view.filter),
          ),
      ],
    ),
  );
  if (setup == null) return null;
  return (
    table: table.id,
    title: table.name,
    sort: setup.sort,
    filter: setup.filter,
  );
}

String _describe(BridgeShown view, List<BridgeField> fields, String locale) {
  final numbers = NumberFormat.decimalPattern(
    NumberFormat.localeExists(locale) ? locale : 'en',
  );
  final days = shownDays(DateFormat.localeExists(locale) ? locale : 'en');
  return [
    if (view.sort case final sort?)
      if (fields.where((field) => field.id == sort.field).firstOrNull
          case final field?)
        '${field.name}, ${sort.descending ? 'descending' : 'ascending'}',
    if (view.filter case final filter?)
      ?filterLabel(filter, fields, numbers, days),
  ].join(' · ');
}

class _ChooseTable extends StatelessWidget {
  const _ChooseTable({required this.tables});

  final List<BridgeTable> tables;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ActionDialog(
      title: 'Add a View of a Table',
      actions: const [],
      dismiss: 'Cancel',
      children: [
        for (final table in tables)
          InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            onTap: () => Navigator.of(context).pop(table),
            child: Container(
              height: 32,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              child: Row(
                children: [
                  Icon(LucideIcons.table, size: 14, color: ws.textFaint),
                  const SizedBox(width: WsMetrics.space3),
                  Expanded(
                    child: Text(
                      _named(table.name),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsUi,
                        color: ws.text,
                      ),
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

typedef _Offered = ({String title, String page, String setup, _Setup chosen});

class _AddView extends StatefulWidget {
  const _AddView({required this.name, required this.views});

  final String name;
  final List<_Offered> views;

  @override
  State<_AddView> createState() => _AddViewState();
}

class _AddViewState extends State<_AddView> {
  int? _copied;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget option(int? index, String label, {String? detail, String? page}) {
      final on = _copied == index;
      return InkWell(
        key: Key('setup ${index ?? 'none'}'),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        onTap: () => setState(() => _copied = index),
        child: Container(
          constraints: const BoxConstraints(minHeight: 32),
          padding: const EdgeInsets.symmetric(
            horizontal: WsMetrics.space2,
            vertical: WsMetrics.space1,
          ),
          decoration: BoxDecoration(
            color: on ? ws.hover : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              Icon(
                on ? LucideIcons.circleDot : LucideIcons.circle,
                size: 14,
                color: on ? ws.accent : ws.textFaint,
              ),
              const SizedBox(width: WsMetrics.space3),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.text,
                      ),
                    ),
                    if (detail != null)
                      Text(
                        detail,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          fontSize: WsMetrics.fsSmall,
                          color: ws.textFaint,
                        ),
                      ),
                  ],
                ),
              ),
              if (page != null) ...[
                const SizedBox(width: WsMetrics.space3),
                Text(
                  page,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textFaint,
                  ),
                ),
              ],
            ],
          ),
        ),
      );
    }

    return ActionDialog(
      title: 'Add a View of “${_named(widget.name)}”',
      actions: [
        PrimaryButton(
          label: 'Add View',
          onPressed: () => Navigator.of(context).pop<_Setup>(switch (_copied) {
            final index? => widget.views[index].chosen,
            null => (sort: null, filter: null),
          }),
        ),
      ],
      children: [
        Container(
          padding: const EdgeInsets.all(WsMetrics.space1),
          decoration: BoxDecoration(
            color: ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              option(null, 'Start with no sort or filter'),
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  WsMetrics.space2,
                  WsMetrics.space2,
                  WsMetrics.space2,
                  WsMetrics.space1,
                ),
                child: Text(
                  'Or copy the setup of',
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textFaint,
                  ),
                ),
              ),
              for (final (index, view) in widget.views.indexed)
                option(
                  index,
                  _named(view.title),
                  detail: view.setup,
                  page: view.page,
                ),
            ],
          ),
        ),
      ],
    );
  }
}
