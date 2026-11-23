part of 'table_block.dart';

extension _TableBlockEdits on _TableBlockState {
  void _panelOutOfView() {
    if (!_panel.isShowing || _fieldsWriting > 0) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_sideways.hasClients) return;
      final position = _sideways.position;
      if (position.pixels <= position.maxScrollExtent - _addWidth) {
        _closePanel();
      }
    });
  }

  void _closePanel() {
    if (_scrollingItself) return;
    if (_panel.isShowing) _panel.hide();
    _fieldMenus.close();
  }

  Future<void> _insertRow(String? after) async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          after: after,
        ),
        const [],
      );
      await _takeFilterValue(added.row);
    } on Object catch (_) {}
  }

  Future<void> _takeFilterValue(String? row) async {
    if (_liveFilter case BridgeFilter(:final field, value: final value?)
        when value.kind != BridgeCellKind.checkbox || value.checked) {
      await widget.views.write(
        mutation(
          BridgeMutationKind.setCell,
          page: _page,
          block: _block,
          row: row,
          field: field,
          cell: value,
        ),
        const [],
      );
    }
  }

  Future<String> _addOption(BridgeField field, String name, int color) async {
    final added = await widget.views.write(
      mutation(
        BridgeMutationKind.addOption,
        page: _page,
        block: _block,
        field: field.id,
        text: name,
        color: color,
      ),
      const [],
    );
    return added.option!;
  }

  Future<void> _addRow() async {
    _rowsWriting++;
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addRow,
          page: _page,
          block: _block,
          last: true,
        ),
        const [],
      );
      _rowAdded = true;
      await _takeFilterValue(added.row);
    } on Object catch (_) {
    } finally {
      _rowsWriting--;
      _readsAtRowAdd = _reads;
      if (_rowAdded && _rowsWriting == 0 && mounted) unawaited(_read());
    }
  }

  void _togglePanel() {
    if (_panel.isShowing) return _panel.hide();
    _fieldMenus.close();
    _panelOpened++;
    _panel.show();
  }

  void _menusChanged() {
    if (mounted) _update(() {});
  }

  Future<void> _addField(String name, BridgeFieldKind kind) async {
    final opened = _panelOpened;
    final BridgeApplied added;
    _fieldsWriting++;
    try {
      added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: name,
          fieldKind: kind,
          last: true,
        ),
        const [],
      );
    } finally {
      _fieldsWriting--;
      if (mounted && _panel.isShowing) _update(() {});
    }
    if (!mounted) return;
    if (opened == _panelOpened) _panel.hide();
    if (_database?.fields.any((field) => field.id == added.field) ?? false) {
      _update(() => _toLastField = true);
    } else {
      _fieldAdded = added.field;
      _readsAtFieldAdd = _reads;
      unawaited(_read());
    }
  }
}
