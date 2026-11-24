import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../shortcuts/keymap.dart';
import 'explorer.dart' show untitled;
import 'media_block.dart';
import 'page_views.dart';
import 'slash_commands.dart';
import 'table_pickers.dart';
import '../rust/api/source.dart';
import 'page_icon.dart';
import 'source_kind.dart' show sourceIcon;
import 'workspace_state.dart';

const _width = 560.0;
const _shown = 8;

typedef _Item = ({
  String group,
  Object icon,
  String label,
  String? detail,
  String? keys,
  Future<void> Function() run,
});

class PowerBar extends StatefulWidget {
  const PowerBar({
    super.key,
    required this.state,
    required this.onClose,
    required this.onNewPage,
    required this.onSettings,
  });

  final WorkspaceState state;
  final VoidCallback onClose;
  final VoidCallback onNewPage;
  final VoidCallback onSettings;

  @override
  State<PowerBar> createState() => _PowerBarState();
}

class _PowerBarState extends State<PowerBar> {
  final _text = TextEditingController();
  final _focus = FocusNode(debugLabel: 'PowerBar');
  (String, List<BridgeHit>) _hits = ('', const []);
  int _chosen = 0;
  Timer? _settle;
  int _searches = 0;

  WorkspaceState get _state => widget.state;

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _settle?.cancel();
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _typed() {
    setState(() => _chosen = 0);
    final typed = _text.text;
    _settle?.cancel();
    if (typed.startsWith('/') ||
        typed.startsWith('>') ||
        typed.trim().isEmpty) {
      return;
    }
    _settle = Timer(const Duration(milliseconds: 120), () => _search(typed));
  }

  Future<void> _search(String typed) async {
    final search = ++_searches;
    try {
      final hits = (await _state.read(
        query(BridgeQueryKind.search, search: typed, limit: _shown),
      )).hits;
      if (mounted && search == _searches) {
        setState(() => _hits = (typed, hits));
      }
    } on Object {
      return;
    }
  }

  String? _areaOf(String page) {
    for (final area in _state.areas.value) {
      if (_state.pages(area.id).value.any((shown) => shown.id == page)) {
        return area.id;
      }
    }
    return null;
  }

  String _titleOf(String page) {
    for (final area in _state.areas.value) {
      for (final shown in _state.pages(area.id).value) {
        if (shown.id == page) {
          return shown.title.isEmpty ? untitled : shown.title;
        }
      }
    }
    return untitled;
  }

  Future<void> _openPage(String page) async {
    final area = _areaOf(page);
    if (area != null) _state.open.value = OpenPage(area, page);
  }

  Future<void> _caretTo(String page, String block) async {
    final state = _state;
    for (var tries = 0; tries < 30; tries++) {
      await WidgetsBinding.instance.endOfFrame;
      final views = state.currentPage.value;
      final count = views?.count;
      if (views == null || views.page != page || count == null) continue;
      for (var at = 0; at < count; at++) {
        final view = await views.viewAt(at);
        if (view?.block == block) {
          views.moveCaret(Caret(view!.id, 0));
          return;
        }
      }
      return;
    }
  }

  String? get _areaName => _nameOf(_state.open.value?.area);

  String? get _sourceArea {
    final open = _state.open.value?.area;
    return open != null && open.isNotEmpty
        ? open
        : _state.areas.value.firstOrNull?.id;
  }

  ({String area, BridgeSource source})? _sourceOf(String id) {
    for (final area in _state.areas.value) {
      for (final source in _state.sources(area.id).value) {
        if (source.id == id) return (area: area.id, source: source);
      }
    }
    return null;
  }

  String? _nameOf(String? area) {
    return _state.areas.value
        .where((shown) => shown.id == area)
        .firstOrNull
        ?.name;
  }

  List<_Item> _actions(String filter) {
    final views = _state.currentPage.value;
    return [
      if (views != null)
        (
          group: 'This Page',
          icon: LucideIcons.layoutGrid,
          label: 'Arrange',
          detail: null,
          keys: null,
          run: () async => views.arranging.value = true,
        ),
      (
        group: 'Workspace',
        icon: LucideIcons.filePlus,
        label: 'New Page',
        detail: _areaName == null ? null : 'in $_areaName',
        keys: Keymap.of(context).label(Command.newPage),
        run: () async => widget.onNewPage(),
      ),
      if (_sourceArea case final area?)
        for (final folder in [false, true])
          (
            group: 'Workspace',
            icon: folder ? LucideIcons.folder : LucideIcons.file,
            label: folder ? 'Add Source: Folder…' : 'Add Source: File…',
            detail: 'in ${_nameOf(area)}',
            keys: null,
            run: () => _state.addSource(area, folder: folder),
          ),
      (
        group: 'Workspace',
        icon: LucideIcons.settings,
        label: 'Settings',
        detail: null,
        keys: Keymap.of(context).label(Command.settings),
        run: () async => widget.onSettings(),
      ),
    ].where((item) => item.label.toLowerCase().contains(filter)).toList();
  }

  Future<void> _add(SlashCommand command) async {
    final views = _state.currentPage.value;
    if (views == null) return;
    final state = _state;
    final existing = command.existing
        ? await pickExistingTable(Navigator.of(context).context, state)
        : null;
    if (command.existing && existing == null) return;
    final focused = views.lastFocused;
    final count = views.count ?? 0;
    final last = count == 0 ? null : await views.viewAt(count - 1);
    final after = focused != null && views.indexOf(focused) != null
        ? focused
        : last?.id;
    BridgeMutation change(BridgeMutationKind kind, {String text = ''}) =>
        mutation(kind, page: views.page, after: after, text: text);
    try {
      switch (command) {
        case SlashCommand(existing: true):
          await views.write(
            mutation(
              BridgeMutationKind.addTableView,
              page: views.page,
              after: after,
              block: existing!.table,
              text: existing.title,
              tabs: existing.tabs,
            ),
            const [],
          );
        case SlashCommand(block: BridgeBlockKind.database, :final layout):
          await views.write(
            mutation(
              BridgeMutationKind.addDatabaseBlock,
              page: views.page,
              after: after,
              text: '',
              layout: layout,
            ),
            const [],
          );
        case SlashCommand(block: BridgeBlockKind.whiteboard):
          await views.write(
            change(BridgeMutationKind.addWhiteboardBlock),
            const [],
          );
        case SlashCommand(block: BridgeBlockKind.chart):
          await views.write(change(BridgeMutationKind.addChartBlock), const []);
        case SlashCommand(block: BridgeBlockKind.calculation):
          await views.write(
            change(BridgeMutationKind.addCalculationBlock),
            const [],
          );
        case SlashCommand(block: BridgeBlockKind.media):
          final files = await MediaPlatform.pickFiles();
          if (files.isNotEmpty) await addMedia(views, files, after: after);
        case SlashCommand(block: final kind?)
            when kind == BridgeBlockKind.code || kind == BridgeBlockKind.math:
          final added = await views.write(
            change(BridgeMutationKind.addTextBlock),
            const [],
          );
          await views.write(
            mutation(
              kind == BridgeBlockKind.code
                  ? BridgeMutationKind.turnIntoCode
                  : BridgeMutationKind.turnIntoMath,
              page: views.page,
              block: added.block,
            ),
            const [],
          );
        case SlashCommand(:final style):
          final added = await views.write(
            change(BridgeMutationKind.addTextBlock),
            const [],
          );
          if (style != null && style != BridgeTextStyle.paragraph) {
            await views.write(
              mutation(
                BridgeMutationKind.setTextStyle,
                page: views.page,
                block: added.block,
                style: style,
              ),
              const [],
            );
          }
      }
    } on Object catch (error) {
      state.report('Liquid could not add the Block: $error');
    }
  }

  List<_Item> _items() {
    final typed = _text.text;
    if (typed.startsWith('>')) {
      return _actions(typed.substring(1).trim().toLowerCase());
    }
    if (typed.startsWith('/')) {
      final views = _state.currentPage.value;
      if (views == null) return const [];
      final title = _titleOf(views.page);
      final commands = slashCommandsFor(typed.substring(1).trim()).where(
        (command) =>
            command.block != null || command.style == BridgeTextStyle.paragraph,
      );
      return [
        for (final command in [
          ...commands.where((command) => !command.existing),
          ...commands.where((command) => command.existing),
        ])
          (
            group: command.existing
                ? 'Reuse'
                : 'Add to $title, below the selection',
            icon: command.icon,
            label: command.label,
            detail: command.description,
            keys: null,
            run: () => _add(command),
          ),
      ];
    }
    if (typed.trim().isNotEmpty) {
      final tables = {for (final table in _state.tables.value) table.id: table};
      return [
        for (final hit in _hits.$1 == typed ? _hits.$2 : const <BridgeHit>[])
          switch (hit.found) {
            BridgeFound.page => (
              group: 'Pages',
              icon: _state.iconOf(hit.id),
              label: hit.text.isEmpty ? untitled : hit.text,
              detail: _nameOf(_areaOf(hit.id)),
              keys: null,
              run: () => _openPage(hit.id),
            ),
            BridgeFound.table => (
              group: 'Tables',
              icon: LucideIcons.table,
              label: hit.text.isEmpty ? untitled : hit.text,
              detail: 'Table',
              keys: null,
              run: () async {
                if (tables.containsKey(hit.id)) _state.openTable(hit.id);
              },
            ),
            BridgeFound.source => (
              group: 'Sources',
              icon: sourceIcon(
                hit.text,
                folder:
                    _sourceOf(hit.id)?.source.kind == BridgeSourceKind.folder,
              ),
              label: hit.text,
              detail: _nameOf(_sourceOf(hit.id)?.area),
              keys: null,
              run: () async {
                if (_sourceOf(hit.id) case final found?) {
                  _state.open.value = OpenPage.source(found.area, hit.id);
                }
              },
            ),
            BridgeFound.text => (
              group: 'Text',
              icon: LucideIcons.text,
              label: hit.text,
              detail: hit.page == null ? null : _titleOf(hit.page!),
              keys: null,
              run: () async {
                if (hit.page case final page?) {
                  await _openPage(page);
                  await _caretTo(page, hit.id);
                }
              },
            ),
          },
      ];
    }
    return [
      for (final page
          in _state.recent.value.where((page) => _areaOf(page) != null).take(3))
        (
          group: 'Recent',
          icon: _state.iconOf(page),
          label: _titleOf(page),
          detail: _nameOf(_areaOf(page)),
          keys: null,
          run: () => _openPage(page),
        ),
      ..._actions(''),
    ];
  }

  Future<void> _run(_Item item) async {
    widget.onClose();
    await item.run();
  }

  KeyEventResult _key(FocusNode _, KeyEvent event, List<_Item> items) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.escape:
        widget.onClose();
      case LogicalKeyboardKey.arrowDown:
        if (items.isNotEmpty) {
          setState(() => _chosen = (_chosen + 1) % items.length);
        }
      case LogicalKeyboardKey.arrowUp:
        if (items.isNotEmpty) {
          setState(() => _chosen = (_chosen - 1) % items.length);
        }
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (items.isNotEmpty) {
          unawaited(_run(items[_chosen.clamp(0, items.length - 1)]));
        }
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final items = _items();
    final typed = _text.text;
    final footer = typed.startsWith('>')
        ? 'Only actions your Role allows'
        : typed.startsWith('/')
        ? '⏎ adds it to the Page'
        : '/ Blocks    > Actions';
    final rows = <Widget>[];
    String? group;
    for (final (index, item) in items.indexed) {
      if (item.group != group) {
        group = item.group;
        rows.add(
          Padding(
            padding: const EdgeInsets.fromLTRB(
              WsMetrics.space3,
              WsMetrics.space2,
              WsMetrics.space3,
              WsMetrics.space1,
            ),
            child: Text(
              item.group,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
          ),
        );
      }
      final on = index == _chosen;
      rows.add(
        InkWell(
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          onTap: () => unawaited(_run(item)),
          child: Container(
            height: 34,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            decoration: BoxDecoration(
              color: on ? ws.hover : null,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              children: [
                switch (item.icon) {
                  final IconData icon => Icon(
                    icon,
                    size: 15,
                    color: ws.textMuted,
                  ),
                  final icon => SizedBox(
                    width: 15,
                    child: Center(
                      child: PageIcon('$icon', size: 15, color: ws.textMuted),
                    ),
                  ),
                },
                const SizedBox(width: WsMetrics.space3),
                Expanded(
                  child: Row(
                    children: [
                      Flexible(
                        child: Text(
                          item.label,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: WsMetrics.fsUi,
                            color: ws.text,
                          ),
                        ),
                      ),
                      if (item.detail case final detail?) ...[
                        const SizedBox(width: WsMetrics.space2),
                        Flexible(
                          child: Text(
                            detail,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: WsMetrics.fsCompact,
                              color: ws.textFaint,
                            ),
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                if (item.keys case final keys?)
                  Text(
                    keys,
                    style: TextStyle(
                      fontSize: WsMetrics.fsCompact,
                      color: ws.textFaint,
                    ),
                  ),
              ],
            ),
          ),
        ),
      );
    }
    return Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            onTap: widget.onClose,
            child: ColoredBox(color: Colors.black.withValues(alpha: 0.18)),
          ),
        ),
        Align(
          alignment: Alignment.topCenter,
          child: Padding(
            padding: const EdgeInsets.only(top: 96),
            child: Focus(
              onKeyEvent: (node, event) => _key(node, event, items),
              child: FloatingPanel(
                key: const Key('power bar'),
                width: _width,
                constraints: const BoxConstraints(maxHeight: 520),
                child: Material(
                  type: MaterialType.transparency,
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Container(
                        height: 48,
                        padding: const EdgeInsets.symmetric(
                          horizontal: WsMetrics.space3,
                        ),
                        decoration: BoxDecoration(
                          border: Border(bottom: BorderSide(color: ws.border)),
                        ),
                        child: Row(
                          children: [
                            Icon(
                              LucideIcons.search,
                              size: 16,
                              color: ws.textFaint,
                            ),
                            const SizedBox(width: WsMetrics.space3),
                            Expanded(
                              child: TextField(
                                controller: _text,
                                focusNode: _focus,
                                style: TextStyle(
                                  fontSize: WsMetrics.fsProse,
                                  color: ws.text,
                                ),
                                decoration: InputDecoration.collapsed(
                                  hintText: 'Search or run a command',
                                  hintStyle: TextStyle(color: ws.textFaint),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                      Flexible(
                        child: ListView(
                          shrinkWrap: true,
                          padding: const EdgeInsets.all(WsMetrics.space1),
                          children: rows,
                        ),
                      ),
                      Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: WsMetrics.space3,
                          vertical: WsMetrics.space2,
                        ),
                        decoration: BoxDecoration(
                          border: Border(top: BorderSide(color: ws.border)),
                        ),
                        child: Text(
                          footer,
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
            ),
          ),
        ),
      ],
    );
  }
}
