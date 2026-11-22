import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart' show untitled;
import 'table_tabs.dart';
import 'workspace_state.dart';

typedef ExistingView = ({String table, String title, List<BridgeTab> tabs});

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
  if (!context.mounted) return null;
  final pages = {
    for (final area in state.areas.value)
      for (final page in state.pages(area.id).value) page.id: page.title,
  };
  final tabs = await showDialog<List<BridgeTab>>(
    context: context,
    builder: (_) => _AddView(
      name: table.name,
      views: [
        for (final view in shown)
          (
            title: '${_named(pages[view.page] ?? '')} › ${_named(view.title)}',
            detail: view.tabs.length == 1
                ? '1 tab'
                : '${view.tabs.length} tabs',
            tabs: view.tabs,
          ),
      ],
    ),
  );
  if (tabs == null) return null;
  return (table: table.id, title: table.name, tabs: tabs);
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

typedef _Offered = ({String title, String detail, List<BridgeTab> tabs});

class _AddView extends StatefulWidget {
  const _AddView({required this.name, required this.views});

  final String name;
  final List<_Offered> views;

  @override
  State<_AddView> createState() => _AddViewState();
}

class _AddViewState extends State<_AddView> {
  int? _copied;
  var _layout = BridgeLayout.table;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget option(int? index, Widget child) {
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
              Expanded(child: child),
            ],
          ),
        ),
      );
    }

    Text text(String text, {bool faint = false}) => Text(
      text,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: TextStyle(
        fontSize: faint ? WsMetrics.fsSmall : WsMetrics.fsCompact,
        color: faint ? ws.textFaint : ws.text,
      ),
    );

    final layoutChoice = Row(
      children: [
        for (final (layout, icon, name) in layouts)
          Padding(
            padding: const EdgeInsets.only(right: WsMetrics.space1),
            child: InkWell(
              key: Key('layout $name'),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: () => setState(() {
                _copied = null;
                _layout = layout;
              }),
              child: Container(
                height: 26,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                ),
                decoration: BoxDecoration(
                  color: _layout == layout ? ws.surface : null,
                  border: Border.all(
                    color: _layout == layout ? ws.borderStrong : ws.border,
                  ),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(icon, size: 13, color: ws.textMuted),
                    const SizedBox(width: WsMetrics.space1),
                    text(name),
                  ],
                ),
              ),
            ),
          ),
      ],
    );

    return ActionDialog(
      title: 'Add a View of “${_named(widget.name)}”',
      actions: [
        PrimaryButton(
          label: 'Add View',
          onPressed: () => Navigator.of(context).pop(switch (_copied) {
            final index? => widget.views[index].tabs,
            null => [tabOfLayout(_layout)],
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
              option(
                null,
                Padding(
                  padding: const EdgeInsets.symmetric(
                    vertical: WsMetrics.space1,
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      text('Start with one tab'),
                      const SizedBox(height: WsMetrics.space2),
                      layoutChoice,
                    ],
                  ),
                ),
              ),
              if (widget.views.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.fromLTRB(
                    WsMetrics.space2,
                    WsMetrics.space2,
                    WsMetrics.space2,
                    WsMetrics.space1,
                  ),
                  child: text('Or copy the tabs of', faint: true),
                ),
              for (final (index, view) in widget.views.indexed)
                option(
                  index,
                  Row(
                    children: [
                      Expanded(child: text(view.title)),
                      const SizedBox(width: WsMetrics.space3),
                      text(view.detail, faint: true),
                    ],
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }
}
