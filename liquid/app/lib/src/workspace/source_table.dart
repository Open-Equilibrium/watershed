import 'package:flutter/material.dart';

import '../rust/api/source.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../controls.dart';
import 'page_icon.dart';
import 'workspace_state.dart';

class SourceTable extends StatelessWidget {
  const SourceTable({
    super.key,
    required this.state,
    required this.file,
    required this.head,
    required this.users,
    required this.root,
    required this.onPage,
  });

  final WorkspaceState state;
  final BridgeSourceFile file;
  final List<Widget> head;
  final List<(String, String, String)> users;
  final bool root;
  final ValueChanged<String> onPage;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textFaint);
    final caption = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    Widget line(List<String> cells, {bool header = false}) => Container(
      height: header ? 30 : WsMetrics.rowTable,
      decoration: BoxDecoration(
        color: header ? ws.subtle : null,
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        children: [
          for (final (at, cell) in cells.indexed)
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                child: Text(
                  cell,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: file.columns[at].number
                      ? TextAlign.right
                      : TextAlign.left,
                  style: header ? faint : null,
                ),
              ),
            ),
        ],
      ),
    );
    final tail = <Widget>[
      const SizedBox(height: WsMetrics.space4),
      if (root) ...[
        Row(
          spacing: WsMetrics.space3,
          children: [
            _PagePicker(state: state, onPage: onPage),
            Flexible(
              child: Text(
                'Liquid reads this file and never writes it.',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: caption,
              ),
            ),
          ],
        ),
        const SizedBox(height: WsMetrics.space4),
        Text('Used by', style: caption),
        const SizedBox(height: WsMetrics.space1),
        if (users.isEmpty) Text('Nothing yet.', style: caption),
        for (final (page, _, title) in users)
          InkWell(
            key: ValueKey('source user $page'),
            onTap: () {
              final area = state.areas.value
                  .where(
                    (area) => state
                        .pages(area.id)
                        .value
                        .any((known) => known.id == page),
                  )
                  .firstOrNull;
              if (area != null) state.open.value = OpenPage(area.id, page);
            },
            child: SizedBox(
              height: 32,
              child: Row(
                children: [
                  Icon(LucideIcons.barChart3, size: 14, color: ws.textFaint),
                  const SizedBox(width: WsMetrics.space2),
                  Expanded(child: Text(title)),
                  Text('Chart · ${_pageTitle(page)}', style: caption),
                ],
              ),
            ),
          ),
      ],
    ];
    return CustomScrollView(
      slivers: [
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(
            WsMetrics.space7,
            28,
            WsMetrics.space7,
            WsMetrics.space8,
          ),
          sliver: SliverCrossAxisConstrained(
            maxCrossAxisExtent: WsMetrics.widthReading,
            child: SliverList.list(
              children: [
                ...head,
                line([
                  for (final column in file.columns) column.name,
                ], header: true),
                for (final row in file.rows) line(row),
                ...tail,
              ],
            ),
          ),
        ),
      ],
    );
  }

  String _pageTitle(String page) {
    for (final area in state.areas.value) {
      for (final known in state.pages(area.id).value) {
        if (known.id == page) {
          return known.title.isEmpty ? 'Untitled' : known.title;
        }
      }
    }
    return 'Untitled';
  }
}

class SliverCrossAxisConstrained extends StatelessWidget {
  const SliverCrossAxisConstrained({
    super.key,
    required this.maxCrossAxisExtent,
    required this.child,
  });

  final double maxCrossAxisExtent;
  final Widget child;

  @override
  Widget build(BuildContext context) => SliverLayoutBuilder(
    builder: (context, constraints) {
      final extra = constraints.crossAxisExtent - maxCrossAxisExtent;
      return SliverPadding(
        padding: EdgeInsets.symmetric(horizontal: extra > 0 ? extra / 2 : 0),
        sliver: child,
      );
    },
  );
}

class _PagePicker extends StatelessWidget {
  const _PagePicker({required this.state, required this.onPage});

  final WorkspaceState state;
  final ValueChanged<String> onPage;

  @override
  Widget build(BuildContext context) => MenuAnchor(
    menuChildren: [
      for (final area in state.areas.value)
        for (final page in state.pages(area.id).value)
          MenuItemButton(
            key: ValueKey('use in ${page.id}'),
            onPressed: () => onPage(page.id),
            leadingIcon: PageIcon(page.icon),
            child: Text(page.title.isEmpty ? 'Untitled' : page.title),
          ),
    ],
    builder: (context, controller, _) => SecondaryButton(
      label: 'Use in a Block…',
      icon: LucideIcons.link,
      onPressed: () =>
          controller.isOpen ? controller.close() : controller.open(),
    ),
  );
}
