import 'dart:async';

import 'package:flutter/material.dart';

import '../agent/session_menus.dart';
import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'workspace_state.dart';

/// What the Explorer and the Page header call a Page without a title, as Notion does.
const untitled = 'Untitled';

/// The Workspace's areas, each under its own heading with its nested Pages, as Notion shows
/// teamspaces (approved Pages mockups).
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
      ],
    ),
  );
}

/// A heading as the sidebar's sections have it: small and faint.
class SidebarHeading extends StatelessWidget {
  const SidebarHeading(this.text, {super.key, this.trailing});

  final String text;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(
      WsMetrics.space3,
      WsMetrics.space5,
      WsMetrics.space2,
      WsMetrics.space2,
    ),
    child: Row(
      children: [
        Expanded(
          child: Text(
            text,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: context.ws.textFaint,
              fontSize: WsMetrics.fsSmall,
              height: WsMetrics.lhSmall / WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwMedium,
            ),
          ),
        ),
        ?trailing,
      ],
    ),
  );
}

class _Area extends StatefulWidget {
  const _Area({super.key, required this.state, required this.area});

  final WorkspaceState state;
  final BridgeArea area;

  @override
  State<_Area> createState() => _AreaState();
}

class _AreaState extends State<_Area> {
  /// Pages shown open; the rest show collapsed, as in Notion.
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

  /// The open Page's ancestors expand, so the Page shows in the tree.
  void _revealOpen() {
    final open = widget.state.open.value;
    if (open == null || open.area != widget.area.id) return;
    final tree = widget.state.pages(widget.area.id).value;
    final parents = {for (final page in tree) page.id: page.parent};
    final added = <String>[];
    for (var at = parents[open.page]; at != null; at = parents[at]) {
      if (_expanded.add(at)) added.add(at);
    }
    if (added.isNotEmpty) setState(() {});
  }

  void _newPage({String? parent}) {
    if (parent != null) setState(() => _expanded.add(parent));
    unawaited(
      widget.state.newPage(widget.area.id, parent: parent).catchError((_) {}),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        MouseRegion(
          onEnter: (_) => setState(() => _hovered = true),
          onExit: (_) => setState(() => _hovered = false),
          child: SidebarHeading(
            widget.area.name,
            // Always there for the keyboard and screen readers; seen on hover or focus.
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
              // Shown only below expanded ancestors.
              var shown = true;
              for (var at = page.parent; at != null; at = parents[at]) {
                if (!_expanded.contains(at)) shown = false;
              }
              if (!shown) continue;
              final nests =
                  index + 1 < tree.length && tree[index + 1].parent == page.id;
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

/// One Page in the tree, indented by its depth, with a disclosure when it nests others.
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
        child: Shortcuts(
          shortcuts: activateKeys,
          child: InkWell(
            onTap: onOpen,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            // Ink, so the hover and focus highlights paint above the selected row too.
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
                            // Reachable with Tab and opened with ⏎ or space too.
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
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
