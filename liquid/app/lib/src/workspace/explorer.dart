import 'dart:async';

import 'package:flutter/material.dart';

import '../agent/session_menus.dart';
import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'workspace_state.dart';

const untitled = 'Untitled';

class Explorer extends StatelessWidget {
  const Explorer({super.key, required this.state});

  final WorkspaceState state;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.areas,
    builder: (context, areas, _) => Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final area in areas)
          _Area(key: ValueKey(area.id), state: state, area: area),
        _Tables(state: state),
      ],
    ),
  );
}

class SidebarHeading extends StatelessWidget {
  const SidebarHeading(
    this.text, {
    super.key,
    this.trailing,
    this.folded,
    this.onToggle,
  });

  final String text;
  final Widget? trailing;

  final bool? folded;
  final VoidCallback? onToggle;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final folded = this.folded;
    final label = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Flexible(
          child: Text(
            text,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: ws.textFaint,
              fontSize: WsMetrics.fsSmall,
              height: WsMetrics.lhSmall / WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwMedium,
            ),
          ),
        ),
        if (folded != null) ...[
          const SizedBox(width: WsMetrics.space1),
          Icon(
            folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
            size: 11,
            color: ws.textFaint,
          ),
        ],
      ],
    );
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space3,
        WsMetrics.space5,
        WsMetrics.space2,
        WsMetrics.space2,
      ),
      child: Row(
        children: [
          Expanded(
            child: Align(
              alignment: Alignment.centerLeft,
              child: onToggle == null
                  ? label
                  : Semantics(
                      button: true,
                      label: '${folded! ? 'Unfold' : 'Fold'} $text',
                      child: GestureDetector(
                        key: Key('fold $text'),
                        behavior: HitTestBehavior.opaque,
                        onTap: onToggle,
                        child: ExcludeSemantics(child: label),
                      ),
                    ),
            ),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

class _Tables extends StatefulWidget {
  const _Tables({required this.state});

  final WorkspaceState state;

  @override
  State<_Tables> createState() => _TablesState();
}

class _TablesState extends State<_Tables> {
  static const _section = 'tables';
  bool _hovered = false;
  bool _focused = false;

  void _newTable() {
    final state = widget.state;
    final area =
        state.open.value?.area.nonEmpty ?? state.areas.value.firstOrNull?.id;
    if (area == null) return;
    unawaited(state.newTable(area).catchError((_) {}));
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
    return ListenableBuilder(
      listenable: Listenable.merge([state.tables, state.folded, state.open]),
      builder: (context, _) {
        final folded = state.folded.value.contains(_section);
        final open = state.open.value?.table;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MouseRegion(
              onEnter: (_) => setState(() => _hovered = true),
              onExit: (_) => setState(() => _hovered = false),
              child: SidebarHeading(
                'Tables',
                folded: folded,
                onToggle: () => state.toggleFold(_section),
                trailing: Opacity(
                  opacity: _hovered || _focused ? 1 : 0,
                  alwaysIncludeSemantics: true,
                  child: Tooltip(
                    message: 'New Table',
                    child: InkWell(
                      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                      onFocusChange: (focused) =>
                          setState(() => _focused = focused),
                      onTap: _newTable,
                      child: Padding(
                        padding: const EdgeInsets.all(WsMetrics.space1),
                        child: Icon(
                          LucideIcons.plus,
                          size: 14,
                          color: ws.textMuted,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
            if (!folded)
              for (final table in state.tables.value)
                _TableRow(
                  key: ValueKey(table.id),
                  table: table,
                  selected: table.id == open,
                  onOpen: () => state.openTable(table.id),
                ),
          ],
        );
      },
    );
  }
}

extension on String {
  String? get nonEmpty => isEmpty ? null : this;
}

class _TableRow extends StatelessWidget {
  const _TableRow({
    super.key,
    required this.table,
    required this.selected,
    required this.onOpen,
  });

  final BridgeTable table;
  final bool selected;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      selected: selected,
      child: InkWell(
        onTap: onOpen,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        child: Ink(
          height: WsMetrics.rowSide,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          decoration: BoxDecoration(
            color: selected ? ws.selected : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              const SizedBox(width: 12 + WsMetrics.space2),
              SizedBox(
                width: 18,
                child: Icon(LucideIcons.table, size: 14, color: ws.textFaint),
              ),
              const SizedBox(width: WsMetrics.space2),
              Expanded(
                child: Text(
                  table.name.isEmpty ? untitled : table.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: selected ? ws.text : ws.textMuted,
                    fontSize: WsMetrics.fsSide,
                    height: WsMetrics.lhSide / WsMetrics.fsSide,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Area extends StatefulWidget {
  const _Area({super.key, required this.state, required this.area});

  final WorkspaceState state;
  final BridgeArea area;

  @override
  State<_Area> createState() => _AreaState();
}

class _AreaState extends State<_Area> {
  final _expanded = <String>{};
  bool _hovered = false;
  bool _focused = false;

  @override
  void initState() {
    super.initState();
    widget.state.open.addListener(_revealOpen);
  }

  @override
  void dispose() {
    widget.state.open.removeListener(_revealOpen);
    super.dispose();
  }

  void _revealOpen() {
    final open = widget.state.open.value;
    if (open == null || open.area != widget.area.id) return;
    if (widget.state.folded.value.contains(widget.area.id)) {
      widget.state.toggleFold(widget.area.id);
    }
    final tree = widget.state.pages(widget.area.id).value;
    final parents = {for (final page in tree) page.id: page.parent};
    final added = <String>[];
    for (var at = parents[open.page]; at != null; at = parents[at]) {
      if (_expanded.add(at)) added.add(at);
    }
    if (added.isNotEmpty) setState(() {});
  }

  void _newPage({String? parent}) {
    if (widget.state.folded.value.contains(widget.area.id)) {
      widget.state.toggleFold(widget.area.id);
    }
    if (parent != null) setState(() => _expanded.add(parent));
    unawaited(
      widget.state.newPage(widget.area.id, parent: parent).catchError((_) {}),
    );
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: widget.state.folded,
    builder: (context, folded, _) =>
        _build(context, folded.contains(widget.area.id)),
  );

  Widget _build(BuildContext context, bool folded) {
    final ws = context.ws;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        MouseRegion(
          onEnter: (_) => setState(() => _hovered = true),
          onExit: (_) => setState(() => _hovered = false),
          child: SidebarHeading(
            widget.area.name,
            folded: folded,
            onToggle: () => widget.state.toggleFold(widget.area.id),
            trailing: Opacity(
              opacity: _hovered || _focused ? 1 : 0,
              alwaysIncludeSemantics: true,
              child: Tooltip(
                message: 'New Page',
                child: InkWell(
                  borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
                  onFocusChange: (focused) =>
                      setState(() => _focused = focused),
                  onTap: _newPage,
                  child: Padding(
                    padding: const EdgeInsets.all(WsMetrics.space1),
                    child: Icon(
                      LucideIcons.plus,
                      size: 14,
                      color: ws.textMuted,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
        if (!folded)
          ListenableBuilder(
            listenable: Listenable.merge([
              widget.state.pages(widget.area.id),
              widget.state.open,
            ]),
            builder: (context, _) {
              final tree = widget.state.pages(widget.area.id).value;
              final open = widget.state.open.value?.page;
              final parents = {for (final page in tree) page.id: page.parent};
              final depths = <String, int>{};
              final rows = <Widget>[];
              for (final (index, page) in tree.indexed) {
                final depth = depths[page.id] = page.parent == null
                    ? 0
                    : (depths[page.parent] ?? 0) + 1;
                var shown = true;
                for (var at = page.parent; at != null; at = parents[at]) {
                  if (!_expanded.contains(at)) shown = false;
                }
                if (!shown) continue;
                final nests =
                    index + 1 < tree.length &&
                    tree[index + 1].parent == page.id;
                rows.add(
                  _PageRow(
                    key: ValueKey(page.id),
                    page: page,
                    depth: depth,
                    nests: nests,
                    expanded: _expanded.contains(page.id),
                    selected: page.id == open,
                    onOpen: () => widget.state.open.value = OpenPage(
                      widget.area.id,
                      page.id,
                    ),
                    onToggle: () => setState(() {
                      if (!_expanded.remove(page.id)) _expanded.add(page.id);
                    }),
                    onAddInside: () => _newPage(parent: page.id),
                  ),
                );
              }
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: rows,
              );
            },
          ),
      ],
    );
  }
}

class _PageRow extends StatelessWidget {
  const _PageRow({
    super.key,
    required this.page,
    required this.depth,
    required this.nests,
    required this.expanded,
    required this.selected,
    required this.onOpen,
    required this.onToggle,
    required this.onAddInside,
  });

  final BridgePage page;
  final int depth;
  final bool nests;
  final bool expanded;
  final bool selected;
  final VoidCallback onOpen;
  final VoidCallback onToggle;
  final VoidCallback onAddInside;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      selected: selected,
      child: RowMenu(
        actions: [RowAction('Add a page inside', onAddInside)],
        builder: (context, menu, _) => Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            onTap: onOpen,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            child: Ink(
              height: WsMetrics.rowSide,
              padding: EdgeInsets.only(
                left: WsMetrics.space3 + depth * 18,
                right: WsMetrics.space3,
              ),
              decoration: BoxDecoration(
                color: selected ? ws.selected : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Row(
                children: [
                  SizedBox(
                    width: 12,
                    child: nests
                        ? Semantics(
                            label: expanded ? 'Collapse' : 'Expand',
                            button: true,
                            child: InkWell(
                              key: const Key('disclosure'),
                              onTap: onToggle,
                              borderRadius: BorderRadius.circular(
                                WsMetrics.radiusSm,
                              ),
                              child: Icon(
                                expanded
                                    ? LucideIcons.chevronDown
                                    : LucideIcons.chevronRight,
                                size: 11,
                                color: ws.textFaint,
                              ),
                            ),
                          )
                        : null,
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  SizedBox(
                    width: 18,
                    child: Icon(
                      LucideIcons.file,
                      size: 14,
                      color: ws.textFaint,
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  Expanded(
                    child: Text(
                      page.title.isEmpty ? untitled : page.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: selected ? ws.text : ws.textMuted,
                        fontSize: WsMetrics.fsSide,
                        height: WsMetrics.lhSide / WsMetrics.fsSide,
                      ),
                    ),
                  ),
                  menu,
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
