part of 'table_block.dart';

extension _TableBlockTitle on _TableBlockState {
  void _leave() {
    final views = widget.views;
    final at = views.indexOf(widget.view.id);
    final next = at == null ? null : views.textFrom(at + 1, 1);
    if (next != null) {
      views.moveCaret(Caret(next.id, 0));
    } else {
      _cells.cellsFocus.unfocus();
    }
  }

  void _left() {
    if (_nameFocus.hasFocus) return;
    _name.write();
    if (widget.views.pending(widget.view.id) > 0) return;
    _name.show(widget.view.title);
  }

  void _openTable() => _state.openTable(_block);

  Future<void> _deleteView() async {
    final BridgeAnswer shown;
    try {
      shown = await _state.read(
        query(BridgeQueryKind.tableViews, block: _block, limit: 2),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    if (!mounted) return;
    final remove = mutation(
      BridgeMutationKind.removeView,
      page: _page,
      view: widget.view.id,
    );
    if (shown.count > 1) {
      unawaited(_write(remove));
      return;
    }
    final chosen = await showDialog<LastView>(
      context: context,
      builder: (_) => LastViewDialog(
        name: _database?.name ?? '',
        rows: _allRows ?? _rows.count ?? 0,
      ),
    );
    if (!mounted || chosen == null) return;
    unawaited(
      _write(
        chosen == LastView.table
            ? mutation(BridgeMutationKind.deleteTable, block: _block)
            : remove,
      ),
    );
  }

  void _duplicate() {
    final name = _database?.name ?? widget.view.title;
    unawaited(
      _write(
        mutation(
          BridgeMutationKind.duplicateTable,
          page: _page,
          after: widget.view.id,
          block: _block,
          text: '${name.isEmpty ? untitled : name} (copy)',
          tabs: widget.view.tabs,
        ),
      ),
    );
  }

  Future<void> _copy() async {
    final name = _database?.name ?? widget.view.title;
    final text = name.isEmpty ? untitled : name;
    _state.copiedView = null;
    try {
      await writeClipboard(text);
      final written = await readClipboard();
      _state.copiedView = (
        text: text,
        change: written.change,
        table: _block,
        title: widget.view.title,
        tabs: widget.view.tabs,
      );
    } on Object catch (error) {
      _state.report('Liquid could not copy the View: $error');
    }
  }

  Future<void> _pasteAsView(CopiedView copied) async {
    widget.views.pastedCopy.value = null;
    try {
      await widget.views.write(
        mutation(
          BridgeMutationKind.addTableView,
          page: _page,
          after: widget.view.id,
          block: copied.table,
          text: copied.title,
          tabs: copied.tabs,
        ),
        const [],
      );
      await widget.views.write(
        mutation(BridgeMutationKind.deleteTable, block: _block),
        const [],
      );
    } on Object {
      return;
    }
  }

  Widget _pastedChip() => ValueListenableBuilder(
    valueListenable: widget.views.pastedCopy,
    builder: (context, pasted, _) {
      if (pasted == null || pasted.view != widget.view.id) {
        return const SizedBox.shrink();
      }
      final ws = context.ws;
      return Padding(
        padding: const EdgeInsets.only(top: WsMetrics.space2),
        child: Align(
          alignment: Alignment.centerLeft,
          child: Container(
            height: 30,
            padding: const EdgeInsets.only(left: WsMetrics.space3),
            decoration: BoxDecoration(
              color: ws.surface,
              border: Border.all(color: ws.border),
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              boxShadow: ws.shadow,
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  'Pasted as a copy',
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: ws.textMuted,
                  ),
                ),
                const SizedBox(width: WsMetrics.space2),
                TextButton(
                  onPressed: () => unawaited(_pasteAsView(pasted.copied)),
                  child: const Text('Paste as View'),
                ),
                IconButton(
                  tooltip: 'Keep the copy',
                  iconSize: 14,
                  onPressed: () => widget.views.pastedCopy.value = null,
                  icon: Icon(LucideIcons.x, color: ws.textFaint),
                ),
              ],
            ),
          ),
        ),
      );
    },
  );

  Widget _titleLine(BridgeDatabase? database) {
    final ws = context.ws;
    return MouseRegion(
      onEnter: (_) => _update(() => _hovered = true),
      onExit: (_) => _update(() => _hovered = false),
      child: SizedBox(
        height: 32,
        child: Row(
          children: [
            Icon(LucideIcons.table, size: 16, color: ws.textFaint),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: TextField(
                controller: _name.take(),
                focusNode: _nameFocus,
                selectAllOnFocus: false,
                style: TextStyle(
                  fontSize: WsMetrics.fsProse,
                  fontWeight: WsMetrics.fwSemibold,
                  color: ws.text,
                ),
                decoration: InputDecoration.collapsed(
                  hintText: untitled,
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            if (database != null) ...[
              const SizedBox(width: WsMetrics.space3),
              Flexible(
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    spacing: WsMetrics.space3,
                    children: [
                      ValueListenableBuilder(
                        valueListenable: _state.tables,
                        builder: (context, tables, _) =>
                            tables.any(
                              (table) => table.id == _block && table.protected,
                            )
                            ? const ProtectedMark()
                            : const SizedBox.shrink(),
                      ),
                      Flexible(
                        child: TableNote(
                          name: database.name,
                          onOpen: _openTable,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ],
            MoreMenu(
              label: 'View actions',
              visible: _hovered || touchFirst,
              items: [
                (LucideIcons.copy, 'Duplicate', _duplicate),
                (LucideIcons.clipboardCopy, 'Copy', () => unawaited(_copy())),
                (LucideIcons.trash, 'Delete View', _deleteView),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
