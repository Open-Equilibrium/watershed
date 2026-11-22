import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'table_query.dart';

const layouts = [
  (BridgeLayout.table, LucideIcons.table, 'Table'),
  (BridgeLayout.board, LucideIcons.kanban, 'Board'),
  (BridgeLayout.calendar, LucideIcons.calendar, 'Calendar'),
];

IconData layoutIcon(BridgeLayout layout) =>
    layouts.firstWhere((shown) => shown.$1 == layout).$2;

String layoutName(BridgeLayout layout) =>
    layouts.firstWhere((shown) => shown.$1 == layout).$3;

BridgeTab tabOfLayout(BridgeLayout layout) => BridgeTab(
  id: '',
  name: layoutName(layout),
  layout: layout,
  folded: const [],
);

BridgeTab copiedTab(BridgeTab tab, {List<BridgeCell>? folded}) => BridgeTab(
  id: tab.id,
  name: tab.name,
  layout: tab.layout,
  group: tab.group,
  date: tab.date,
  sort: tab.sort,
  filter: tab.filter,
  folded: folded ?? tab.folded,
);

bool sameGroup(BridgeCell a, BridgeCell b) =>
    a.kind == b.kind && a.text == b.text && a.checked == b.checked;

bool sameSetup(BridgeTab a, BridgeTab b) =>
    a.id == b.id &&
    a.layout == b.layout &&
    a.group == b.group &&
    a.date == b.date &&
    a.sort == b.sort &&
    sameFilter(a.filter, b.filter) &&
    a.folded.length == b.folded.length &&
    [for (final (at, cell) in a.folded.indexed) sameGroup(cell, b.folded[at])]
        .every((same) => same);

class TabStrip extends StatefulWidget {
  const TabStrip({
    super.key,
    required this.tabs,
    required this.active,
    required this.onSelect,
    required this.onAdd,
    required this.onRename,
    required this.onLayout,
    required this.onDuplicate,
    required this.onDelete,
    required this.onMove,
  });

  final List<BridgeTab> tabs;
  final String active;
  final ValueChanged<String> onSelect;
  final ValueChanged<BridgeLayout> onAdd;
  final void Function(String tab, String name) onRename;
  final void Function(String tab, BridgeLayout layout) onLayout;
  final ValueChanged<String> onDuplicate;
  final ValueChanged<String> onDelete;
  final void Function(String tab, String? after) onMove;

  @override
  State<TabStrip> createState() => _TabStripState();
}

class _TabStripState extends State<TabStrip> {
  String? _renaming;
  final _name = TextEditingController();
  final _nameFocus = FocusNode(debugLabel: 'Tab name');

  @override
  void initState() {
    super.initState();
    _nameFocus.addListener(() {
      if (!_nameFocus.hasFocus) _finishRename();
    });
  }

  @override
  void dispose() {
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _startRename(BridgeTab tab) {
    setState(() => _renaming = tab.id);
    _name.value = TextEditingValue(
      text: tab.name,
      selection: TextSelection(baseOffset: 0, extentOffset: tab.name.length),
    );
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _nameFocus.requestFocus();
    });
  }

  void _finishRename({bool keep = true}) {
    final tab = _renaming;
    if (tab == null) return;
    setState(() => _renaming = null);
    final name = _name.text.trim();
    final before = widget.tabs.where((shown) => shown.id == tab).firstOrNull;
    if (keep && name.isNotEmpty && before != null && name != before.name) {
      widget.onRename(tab, name);
    }
  }

  void _dropped(String moved, BridgeTab onto) {
    if (moved == onto.id) return;
    final tabs = widget.tabs;
    final from = tabs.indexWhere((tab) => tab.id == moved);
    final to = tabs.indexWhere((tab) => tab.id == onto.id);
    if (from < 0 || to < 0) return;
    final rest = [
      for (final tab in tabs)
        if (tab.id != moved) tab.id,
    ];
    final at = rest.indexOf(onto.id) + (from < to ? 1 : 0);
    widget.onMove(moved, at == 0 ? null : rest[at - 1]);
  }

  Widget _menu(BridgeTab tab, VoidCallback close) => ChoiceMenu(
    onClose: close,
    sections: [
      (
        label: null,
        choices: [
          (
            icon: LucideIcons.pencil,
            label: 'Rename',
            checked: false,
            onTap: () => _startRename(tab),
          ),
        ],
      ),
      (
        label: 'Layout',
        choices: [
          for (final (layout, icon, name) in layouts)
            (
              icon: icon,
              label: name,
              checked: tab.layout == layout,
              onTap: () {
                if (tab.layout != layout) widget.onLayout(tab.id, layout);
              },
            ),
        ],
      ),
      (
        label: null,
        choices: [
          (
            icon: LucideIcons.copy,
            label: 'Duplicate tab',
            checked: false,
            onTap: () => widget.onDuplicate(tab.id),
          ),
          if (widget.tabs.length > 1)
            (
              icon: LucideIcons.trash,
              label: 'Delete tab',
              checked: false,
              onTap: () => widget.onDelete(tab.id),
            ),
        ],
      ),
    ],
  );

  Widget _tab(BridgeTab tab) {
    final ws = context.ws;
    final on = tab.id == widget.active;
    final label = _renaming == tab.id
        ? SizedBox(
            width: 120,
            child: CallbackShortcuts(
              bindings: {
                const SingleActivator(LogicalKeyboardKey.escape): () =>
                    _finishRename(keep: false),
              },
              child: TextField(
                key: const Key('tab name'),
                controller: _name,
                focusNode: _nameFocus,
                onSubmitted: (_) => _finishRename(),
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: const InputDecoration.collapsed(hintText: ''),
              ),
            ),
          )
        : Text(
            tab.name,
            style: TextStyle(
              fontSize: WsMetrics.fsUi,
              fontWeight: on ? WsMetrics.fwMedium : null,
              color: on ? ws.text : ws.textMuted,
            ),
          );
    return QueryAnchor(
      anchor: (toggle, open) {
        final shown = Container(
          height: 32,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          decoration: BoxDecoration(
            color: open ? ws.hover : null,
            border: Border(
              bottom: BorderSide(
                color: on ? ws.text : Colors.transparent,
                width: 2,
              ),
            ),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(layoutIcon(tab.layout), size: 14, color: ws.textMuted),
              const SizedBox(width: WsMetrics.space2),
              label,
            ],
          ),
        );
        return DragTarget<String>(
          onAcceptWithDetails: (dropped) => _dropped(dropped.data, tab),
          builder: (context, _, _) => Draggable<String>(
            data: tab.id,
            axis: Axis.horizontal,
            feedback: Material(color: Colors.transparent, child: shown),
            childWhenDragging: Opacity(opacity: 0.4, child: shown),
            child: Semantics(
              button: true,
              selected: on,
              label: '${tab.name} tab',
              child: GestureDetector(
                key: Key('tab ${tab.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: () {
                  if (_renaming == tab.id) return;
                  if (on) {
                    toggle();
                  } else {
                    widget.onSelect(tab.id);
                  }
                },
                onSecondaryTap: () {
                  if (!on) widget.onSelect(tab.id);
                  toggle();
                },
                child: shown,
              ),
            ),
          ),
        );
      },
      menu: (close) => _menu(tab, close),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 34,
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      alignment: Alignment.bottomLeft,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            for (final tab in widget.tabs) _tab(tab),
            QueryAnchor(
              anchor: (toggle, open) => Semantics(
                button: true,
                label: 'Add a tab',
                child: GestureDetector(
                  key: const Key('add tab'),
                  behavior: HitTestBehavior.opaque,
                  onTap: toggle,
                  child: Container(
                    height: 32,
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space2,
                    ),
                    color: open ? ws.hover : null,
                    child: Icon(
                      LucideIcons.plus,
                      size: 14,
                      color: ws.textFaint,
                    ),
                  ),
                ),
              ),
              menu: (close) => ChoiceMenu(
                title: 'New tab',
                onClose: close,
                sections: [
                  (
                    label: null,
                    choices: [
                      for (final (layout, icon, name) in layouts)
                        (
                          icon: icon,
                          label: name,
                          checked: false,
                          onTap: () => widget.onAdd(layout),
                        ),
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
