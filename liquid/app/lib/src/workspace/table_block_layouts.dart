part of 'table_block.dart';

extension _TableBlockLayouts on _TableBlockState {
  void _shiftMonth(int by) =>
      _update(() => _month = DateTime(_month.year, _month.month + by));

  Future<void> _addDateField() async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: 'Date',
          fieldKind: BridgeFieldKind.date,
          last: true,
        ),
        const [],
      );
      _setDate(added.field);
    } on Object {
      return;
    }
  }

  Widget _monthControls(String locale) {
    final ws = context.ws;
    Widget arrow(Key key, IconData icon, String label, int by) => Semantics(
      button: true,
      label: label,
      child: GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        onTap: () => _shiftMonth(by),
        child: Padding(
          padding: const EdgeInsets.all(WsMetrics.space1),
          child: Icon(icon, size: 14, color: ws.textMuted),
        ),
      ),
    );
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          DateFormat.yMMMM(locale).format(_month),
          style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
        ),
        const SizedBox(width: WsMetrics.space2),
        arrow(
          const Key('previous month'),
          LucideIcons.chevronLeft,
          'Previous month',
          -1,
        ),
        GestureDetector(
          key: const Key('this month'),
          behavior: HitTestBehavior.opaque,
          onTap: () => _update(
            () => _month = DateTime(DateTime.now().year, DateTime.now().month),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
            child: Text(
              'Today',
              style: TextStyle(
                fontSize: WsMetrics.fsCompact,
                color: ws.textMuted,
              ),
            ),
          ),
        ),
        arrow(
          const Key('next month'),
          LucideIcons.chevronRight,
          'Next month',
          1,
        ),
      ],
    );
  }

  TableData _data(BridgeDatabase database, NumberFormat numbers, String days) =>
      TableData(
        page: _page,
        block: _block,
        database: database,
        read: _state.read,
        write: (change) => widget.views.write(change, const []),
        press: (row, field) => _state.press(_block, row, field),
        numbers: numbers,
        days: days,
      );

  Future<void> _addGroupField() async {
    try {
      final added = await widget.views.write(
        mutation(
          BridgeMutationKind.addField,
          page: _page,
          block: _block,
          text: 'Status',
          fieldKind: BridgeFieldKind.select,
          last: true,
        ),
        const [],
      );
      for (final (color, name) in [
        'Not started',
        'In progress',
        'Done',
      ].indexed) {
        await widget.views.write(
          mutation(
            BridgeMutationKind.addOption,
            page: _page,
            block: _block,
            field: added.field,
            text: name,
            color: color,
          ),
          const [],
        );
      }
      _setGroup(added.field);
    } on Object {
      return;
    }
  }

  BridgeField? _groupOf(BridgeDatabase database) {
    bool groups(BridgeField field) =>
        field.kind == BridgeFieldKind.select ||
        field.kind == BridgeFieldKind.checkbox;
    return database.fields
            .where((field) => field.id == _tab.group && groups(field))
            .firstOrNull ??
        (_tab.layout == BridgeLayout.board
            ? database.fields
                  .where((field) => field.kind == BridgeFieldKind.select)
                  .firstOrNull
            : null);
  }

  BridgeField? _dateOf(BridgeDatabase database) {
    final dates = database.fields.where(
      (field) => field.kind == BridgeFieldKind.date,
    );
    return dates.where((field) => field.id == _tab.date).firstOrNull ??
        dates.firstOrNull;
  }
}
