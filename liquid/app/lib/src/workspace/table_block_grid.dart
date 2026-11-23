part of 'table_block.dart';

double _width(BridgeFieldKind kind) => switch (kind) {
  BridgeFieldKind.text => 180,
  BridgeFieldKind.number => 104,
  BridgeFieldKind.select => 128,
  BridgeFieldKind.date => 120,
  BridgeFieldKind.checkbox => 72,
  BridgeFieldKind.file => 180,
  BridgeFieldKind.formula => 120,
  BridgeFieldKind.button => 120,
};

class _NewRow extends StatelessWidget {
  const _NewRow({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          height: WsMetrics.rowTable,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: Row(
            children: [
              Icon(LucideIcons.plus, size: 14, color: ws.textFaint),
              const SizedBox(width: WsMetrics.space2),
              Text(
                'New',
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
}

class _ColumnMenuButton extends StatelessWidget {
  const _ColumnMenuButton({required this.shown, required this.onTap});

  final bool shown;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'Column menu',
    child: GestureDetector(
      key: const Key('column menu'),
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox(
        width: 18,
        height: 24,
        child: Icon(
          LucideIcons.chevronDown,
          size: 12,
          color: shown ? context.ws.textMuted : const Color(0x00000000),
        ),
      ),
    ),
  );
}

extension _TableBlockGrid on _TableBlockState {
  void _setTotal(BridgeField field, BridgeTotal? total) => _setQuery(
    mutation(
      BridgeMutationKind.setTabTotal,
      page: _page,
      view: widget.view.id,
      tab: _tab.id,
      field: field.id,
      total: total,
    ),
  );

  Widget _totalCell(BridgeField field, NumberFormat numbers) {
    final ws = context.ws;
    final set = _tab.totals
        .where((total) => total.field == field.id)
        .firstOrNull;
    final value = _totals[field.id];
    final menu = _totalMenus.putIfAbsent(field.id, OverlayPortalController.new);
    final numeric =
        field.kind == BridgeFieldKind.number ||
        field.kind == BridgeFieldKind.formula;
    final faint = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    return AnchoredMenu(
      controller: menu,
      onTapOutside: (_) => menu.hide(),
      menu: (_) => TotalMenu(
        numeric: numeric,
        chosen: set?.total,
        onPick: (total) {
          menu.hide();
          _setTotal(field, total);
        },
      ),
      child: GestureDetector(
        key: ValueKey('total ${field.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => _update(menu.toggle),
        child: Align(
          alignment: Alignment.centerRight,
          child: set == null
              ? Opacity(
                  opacity: _footerHovered || touchFirst ? 1 : 0,
                  child: Text(
                    'Calculate',
                    style: faint,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                )
              : Text.rich(
                  TextSpan(
                    children: [
                      TextSpan(text: '${totalName(set.total)} ', style: faint),
                      TextSpan(
                        text: switch (value) {
                          null => '',
                          BridgeComputed(kind: BridgeValueKind.number) =>
                            numberText(numbers, value.number),
                          _ => value.value,
                        },
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.text,
                          fontFeatures: const [FontFeature.tabularFigures()],
                        ),
                      ),
                    ],
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
        ),
      ),
    );
  }

  (int, int)? _fileCellAt(Offset global) {
    final box = _rowsKey.currentContext?.findRenderObject();
    final fields = _database?.fields ?? const <BridgeField>[];
    if (box is! RenderBox || !box.hasSize) return null;
    final local = box.globalToLocal(global);
    if (!(Offset.zero & box.size).contains(local)) return null;
    final line = ((local.dy + _down.offset) / WsMetrics.rowTable).floor();
    final row = _lines == null ? line : _lines!.at(line).row;
    if (row == null || row < 0 || row >= (_rows.count ?? 0)) return null;
    final offset = _sideways.hasClients ? _sideways.offset : 0.0;
    var field = 0;
    if (local.dx - offset >= _first) {
      field = _starts.lastIndexWhere((start) => start <= local.dx);
    }
    if (field < 0 || field >= fields.length) return null;
    return fields[field].kind == BridgeFieldKind.file ? (row, field) : null;
  }

  void _silently(VoidCallback move) {
    _scrollingItself = true;
    move();
    _scrollingItself = false;
  }

  void _toTop() {
    if (_down.hasClients) _silently(() => _down.jumpTo(0));
  }

  void _scrolled() {
    if (_scrollingItself) return;
    _cells.closePick();
    _rowHandles.closeMenu();
  }

  void _placed(RenderObject cell) => _silently(() => cell.showOnScreen());

  void _reveal(int row, int field) =>
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        if (_down.hasClients) {
          final top = (_lines?.lineOf(row) ?? row) * WsMetrics.rowTable;
          final view = _down.position.viewportDimension;
          if (top < _down.offset) {
            _silently(() => _down.jumpTo(top));
          } else if (top + WsMetrics.rowTable > _down.offset + view) {
            _silently(() => _down.jumpTo(top + WsMetrics.rowTable - view));
          }
        }
        if (_sideways.hasClients && field > 0 && field + 1 < _starts.length) {
          final view = _sideways.position.viewportDimension;
          final (left, right) = (_starts[field], _starts[field + 1]);
          if (left < _sideways.offset + _first) {
            _silently(() => _sideways.jumpTo(left - _first));
          } else if (right > _sideways.offset + view) {
            _silently(() => _sideways.jumpTo(right - view));
          }
        }
      });

  Widget _grid(
    List<BridgeField> fields,
    double width,
    NumberFormat numbers,
    DateFormat days, {
    required bool footer,
    required String counted,
  }) {
    final ws = context.ws;
    final rest = [
      for (final field in fields.skip(1)) _width(field.kind),
      _addWidth,
    ];
    final others = rest.fold(0.0, (sum, width) => sum + width);
    final first = math.max(_firstWidth, width - others);
    final widths = [first, ...rest];
    final starts = [0.0];
    for (final width in widths) {
      starts.add(starts.last + width);
    }
    _starts = starts;
    _first = first;
    final count = _rows.count ?? 0;
    final lines = _lines;
    final grouped = _grouping;
    final options = {
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select)
          for (final option in field.options) option.id: option,
    };
    Widget content(BridgeField field, BridgeCell? cell) => CellView(
      cell: cell,
      kind: field.kind,
      options: options,
      numbers: numbers,
      days: days,
    );

    Widget line({
      required double height,
      required Color? color,
      required Widget Function(int index) cell,
      VoidCallback? addOutOfView,
      Decoration? Function(int index)? mark,
      void Function(int index)? onTap,
      void Function(int index)? onMenu,
      Key? Function(int index)? keyOf,
      bool Function(int index)? editing,
      bool Function(int index)? picking,
      bool Function(int index)? menu,
      LayerLink? handle,
    }) => ListenableBuilder(
      listenable: _sideways,
      builder: (context, _) {
        final offset = _sideways.hasClients ? _sideways.offset : 0.0;
        Widget box(int index) {
          final box = Container(
            key: keyOf?.call(index),
            width: widths[index],
            height: height,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            foregroundDecoration: mark?.call(index),
            decoration: BoxDecoration(
              color: color,
              border: index > 0
                  ? Border(left: BorderSide(color: ws.border))
                  : null,
              boxShadow: index == 0 && offset > 0
                  ? const [
                      BoxShadow(
                        color: Color(0x40000000),
                        blurRadius: 6,
                        offset: Offset(4, 0),
                        spreadRadius: -4,
                      ),
                    ]
                  : null,
            ),
            child: cell(index),
          );
          if (onTap == null) return box;
          final tapped = GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => onTap(index),
            onSecondaryTap: onMenu == null ? null : () => onMenu(index),
            onLongPress: onMenu == null ? null : () => onMenu(index),
            child: box,
          );
          if (menu?.call(index) ?? false) {
            return AnchoredMenu(
              onTapOutside: (_) => _fieldMenus.close(),
              menu: _fieldMenus.view,
              child: tapped,
            );
          }
          if (picking?.call(index) ?? false) {
            return PickedCell(cells: _cells, onPlaced: _placed, child: tapped);
          }
          return editing?.call(index) ?? false
              ? TextFieldTapRegion(child: tapped)
              : tapped;
        }

        var from = 1;
        while (from < widths.length && starts[from + 1] <= offset + first) {
          from++;
        }
        var to = from;
        while (to < widths.length && starts[to] < offset + width) {
          to++;
        }
        if (to < widths.length) addOutOfView?.call();
        return Stack(
          children: [
            Row(
              children: [
                SizedBox(width: starts[from], height: height),
                for (var index = from; index < to; index++) box(index),
              ],
            ),
            Positioned(
              left: offset,
              top: 0,
              bottom: 0,
              child: handle == null
                  ? box(0)
                  : CompositedTransformTarget(link: handle, child: box(0)),
            ),
          ],
        );
      },
    );

    return SingleChildScrollView(
      key: const Key('table columns'),
      controller: _sideways,
      scrollDirection: Axis.horizontal,
      child: SizedBox(
        width: first + others,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MouseRegion(
              onEnter: (_) => _update(() => _headerHovered = true),
              onExit: (_) => _update(() => _headerHovered = false),
              child: DecoratedBox(
                position: DecorationPosition.foreground,
                decoration: BoxDecoration(
                  border: Border(bottom: BorderSide(color: ws.border)),
                ),
                child: DefaultTextStyle.merge(
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textMuted,
                  ),
                  child: line(
                    height: _headerHeight,
                    color: ws.subtle,
                    addOutOfView: _panelOutOfView,
                    onTap: (index) {
                      if (index >= fields.length) return;
                      _fieldMenus.close();
                      _sortBy(fields[index]);
                    },
                    onMenu: (index) {
                      if (index >= fields.length) return;
                      if (_panel.isShowing) _panel.hide();
                      _fieldMenus.toggle(fields[index]);
                    },
                    menu: (index) =>
                        index < fields.length &&
                        fields[index].id == _fieldMenus.open,
                    cell: (index) => index == fields.length
                        ? AddField(
                            plus: _plus,
                            panel: _panel,
                            onTap: _togglePanel,
                          )
                        : Row(
                            children: [
                              Icon(
                                fieldKindIcon(fields[index].kind),
                                size: 14,
                                color: ws.textFaint,
                              ),
                              const SizedBox(width: WsMetrics.space2),
                              Flexible(child: cellText(fields[index].name)),
                              if (_liveSort case final sort?
                                  when sort.field == fields[index].id) ...[
                                const SizedBox(width: WsMetrics.space1),
                                Icon(
                                  sort.descending
                                      ? LucideIcons.arrowDown
                                      : LucideIcons.arrowUp,
                                  key: const Key('column sorted'),
                                  size: 12,
                                  color: ws.accent,
                                ),
                              ],
                              const Spacer(),
                              _ColumnMenuButton(
                                shown:
                                    _headerHovered ||
                                    touchFirst ||
                                    fields[index].id == _fieldMenus.open,
                                onTap: () {
                                  if (_panel.isShowing) _panel.hide();
                                  _fieldMenus.toggle(fields[index]);
                                },
                              ),
                            ],
                          ),
                  ),
                ),
              ),
            ),
            SizedBox(
              height:
                  math.min(lines?.lineCount ?? count, _shownRows) *
                  WsMetrics.rowTable,
              key: _rowsKey,
              child: ListView.builder(
                key: const Key('table rows'),
                controller: _down,
                itemCount: lines?.lineCount ?? count,
                itemExtent: WsMetrics.rowTable,
                itemBuilder: (context, shownLine) {
                  final at = lines?.at(shownLine);
                  if (at?.group case final index? when grouped != null) {
                    final group = lines!.groups[index];
                    return ListenableBuilder(
                      listenable: _sideways,
                      builder: (context, _) => Padding(
                        padding: EdgeInsets.only(
                          left: _sideways.hasClients ? _sideways.offset : 0,
                        ),
                        child: GroupHeader(
                          label: GroupLabel(
                            field: grouped,
                            value: group.value,
                            days: days,
                            numbers: numbers,
                          ),
                          count: group.count,
                          folded: folds(_tab, group.value),
                          onToggle: () => _toggleFold(group.value),
                          numbers: numbers,
                        ),
                      ),
                    );
                  }
                  final rowIndex = at?.row ?? shownLine;
                  final row = _rows.at(rowIndex);
                  final cells = <String, BridgeCell>{
                    if (row != null)
                      for (final field in fields)
                        field.id: ?cellOf(row, field.id),
                  };
                  final hovered =
                      row != null && row.id == _rowHandles.handleRow;
                  final shown = DecoratedBox(
                    position: DecorationPosition.foreground,
                    decoration: BoxDecoration(
                      border: Border(bottom: BorderSide(color: ws.border)),
                    ),
                    child: line(
                      height: WsMetrics.rowTable,
                      color: hovered && _rowHandles.handle.isShowing
                          ? ws.hover
                          : ws.canvas,
                      handle: hovered ? _rowHandles.handleAnchor : null,
                      keyOf: (index) => row == null || index == fields.length
                          ? null
                          : ValueKey('cell ${row.id} ${fields[index].id}'),
                      onTap: row == null
                          ? null
                          : (index) {
                              if (index < fields.length) {
                                _cells.tap(rowIndex, index);
                              }
                            },
                      mark: (index) => _dropAt == (rowIndex, index)
                          ? BoxDecoration(
                              color: ws.accent.withValues(alpha: 0.06),
                              border: Border.all(color: ws.accent, width: 2),
                            )
                          : _cells.mark(context, rowIndex, index),
                      picking: (index) =>
                          _cells.picking &&
                          _cells.selected == (rowIndex, index),
                      editing: (index) =>
                          _cells.editing &&
                          _cells.selected == (rowIndex, index),
                      cell: (index) {
                        if (index == fields.length) {
                          return const SizedBox.shrink();
                        }
                        final field = fields[index];
                        if (_cells.editing &&
                            _cells.selected == (rowIndex, index) &&
                            row != null) {
                          return _cells.editor(context, field);
                        }
                        if (field.kind == BridgeFieldKind.button &&
                            row != null) {
                          return Align(
                            alignment: Alignment.centerLeft,
                            child: CellButton(
                              label: field.name,
                              onPressed: () => unawaited(
                                _state.press(_block, row.id, field.id),
                              ),
                            ),
                          );
                        }
                        return Align(
                          alignment: switch (field.kind) {
                            BridgeFieldKind.number ||
                            BridgeFieldKind.formula => Alignment.centerRight,
                            BridgeFieldKind.checkbox => Alignment.center,
                            _ => Alignment.centerLeft,
                          },
                          child: content(field, cells[field.id]),
                        );
                      },
                    ),
                  );
                  if (row == null) return shown;
                  return HoverRegion(
                    key: ValueKey('row ${row.id}'),
                    onHover: (hovered, region) =>
                        _rowHandles.hoverRow(row.id, hovered, region),
                    child: shown,
                  );
                },
              ),
            ),
            ListenableBuilder(
              listenable: _sideways,
              builder: (context, child) => Padding(
                padding: EdgeInsets.only(
                  left: _sideways.hasClients ? _sideways.offset : 0,
                ),
                child: child,
              ),
              child: Align(
                alignment: Alignment.centerLeft,
                child: SizedBox(
                  width: width,
                  child: _NewRow(onTap: () => unawaited(_addRow())),
                ),
              ),
            ),
            if (footer)
              MouseRegion(
                onEnter: (_) => _update(() => _footerHovered = true),
                onExit: (_) => _update(() => _footerHovered = false),
                child: DecoratedBox(
                  position: DecorationPosition.foreground,
                  decoration: BoxDecoration(
                    border: Border(top: BorderSide(color: ws.border)),
                  ),
                  child: line(
                    height: _footerHeight,
                    color: ws.subtle,
                    cell: (index) => index == 0
                        ? Align(
                            alignment: Alignment.centerLeft,
                            child: Text(
                              counted,
                              style: TextStyle(
                                fontSize: WsMetrics.fsCompact,
                                color: ws.textFaint,
                              ),
                            ),
                          )
                        : index == fields.length ||
                              fields[index].kind == BridgeFieldKind.button
                        ? const SizedBox.shrink()
                        : _totalCell(fields[index], numbers),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
