part of 'fake_workspace.dart';

extension _FakeTables on FakeWorkspace {
  BridgeApplied? _tablesMutation(BridgeMutation mutation) {
    switch (mutation.kind) {
      case BridgeMutationKind.protectTable || BridgeMutationKind.unprotectTable:
        final at = tables.indexWhere((table) => table.id == mutation.block);
        tables[at] = BridgeTable(
          id: tables[at].id,
          area: tables[at].area,
          name: tables[at].name,
          protected: mutation.kind == BridgeMutationKind.protectTable,
        );
        _tablesChanged();
        return _done();
      case BridgeMutationKind.addTab ||
          BridgeMutationKind.renameTab ||
          BridgeMutationKind.setTabLayout ||
          BridgeMutationKind.setTabGroup ||
          BridgeMutationKind.setTabDate ||
          BridgeMutationKind.setTabSort ||
          BridgeMutationKind.setTabFilter ||
          BridgeMutationKind.setTabFolded ||
          BridgeMutationKind.setTabTotal ||
          BridgeMutationKind.moveTab ||
          BridgeMutationKind.removeTab:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        if (at < 0) throw 'not found';
        final shown = page[at];
        final tabs = [...shown.tabs];
        final index = tabs.indexWhere((tab) => tab.id == mutation.tab);
        String? created;
        if (mutation.kind == BridgeMutationKind.addTab) {
          final tab = withTabId(mutation.tabs.first, created = _id());
          tabs.insert(
            mutation.after == null
                ? 0
                : tabs.indexWhere((tab) => tab.id == mutation.after) + 1,
            tab,
          );
        } else if (index < 0) {
          throw 'not found';
        } else if (mutation.kind == BridgeMutationKind.removeTab) {
          if (tabs.length == 1) throw 'not allowed';
          tabs.removeAt(index);
        } else if (mutation.kind == BridgeMutationKind.moveTab) {
          final moved = tabs.removeAt(index);
          tabs.insert(
            mutation.after == null
                ? 0
                : tabs.indexWhere((tab) => tab.id == mutation.after) + 1,
            moved,
          );
        } else {
          final tab = tabs[index];
          tabs[index] = changedTab(
            tab,
            name: mutation.kind == BridgeMutationKind.renameTab
                ? mutation.text
                : null,
            layout: mutation.kind == BridgeMutationKind.setTabLayout
                ? mutation.layout
                : null,
            group: mutation.kind == BridgeMutationKind.setTabGroup
                ? (mutation.field,)
                : null,
            date: mutation.kind == BridgeMutationKind.setTabDate
                ? (mutation.field,)
                : null,
            sort: mutation.kind == BridgeMutationKind.setTabSort
                ? (mutation.sort,)
                : null,
            filter: mutation.kind == BridgeMutationKind.setTabFilter
                ? (mutation.filter,)
                : null,
            folded: mutation.kind == BridgeMutationKind.setTabFolded
                ? mutation.tabs.first.folded
                : null,
            totals: mutation.kind == BridgeMutationKind.setTabTotal
                ? [
                    for (final total in tab.totals)
                      if (total.field != mutation.field) total,
                    if (mutation.total case final total?)
                      BridgeFieldTotal(field: mutation.field!, total: total),
                  ]
                : null,
          );
        }
        page[at] = _withView(shown, tabs: tabs);
        _viewsChanged(mutation.page);
        return _done(tab: created);
      case BridgeMutationKind.addRow:
        final table = rows[mutation.block]!;
        if (mutation.after case final after?
            when !table.any((row) => row.id == after)) {
          throw 'not found';
        }
        final row = BridgeRow(id: _id(), cells: const [], computed: const []);
        table.insert(
          mutation.last
              ? table.length
              : mutation.after == null
              ? 0
              : table.indexWhere((row) => row.id == mutation.after) + 1,
          row,
        );
        _changed(_tableResource(mutation.block));
        return _done(row: row.id);
      case BridgeMutationKind.moveRow:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        final moved = table.removeAt(at);
        table.insert(
          mutation.last
              ? table.length
              : mutation.after == null
              ? 0
              : table.indexWhere((row) => row.id == mutation.after) + 1,
          moved,
        );
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.removeRow:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        table.removeAt(at);
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.addField:
        final database = databases[mutation.block]!;
        final fields = [...database.fields];
        if (mutation.after case final after?
            when !fields.any((field) => field.id == after)) {
          throw 'not found';
        }
        final field = BridgeField(
          id: _id(),
          name: mutation.text!,
          kind: mutation.fieldKind!,
          options: const [],
          formula: '',
        );
        fields.insert(
          mutation.last
              ? fields.length
              : mutation.after == null
              ? 0
              : fields.indexWhere((field) => field.id == mutation.after) + 1,
          field,
        );
        _setFields(mutation.block, fields);
        _changed(_tableResource(mutation.block));
        return _done(field: field.id);
      case BridgeMutationKind.setCell:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        final cell = mutation.cell!;
        final stored = table[at].cells
            .where((kept) => kept.field == mutation.field)
            .firstOrNull
            ?.value;
        if (cell.kind == BridgeCellKind.empty
            ? stored == null
            : stored == cell) {
          throw 'the write changes nothing';
        }
        table[at] = BridgeRow(
          id: table[at].id,
          cells: [
            for (final kept in table[at].cells)
              if (kept.field != mutation.field) kept,
            if (cell.kind != BridgeCellKind.empty)
              BridgeFieldCell(field: mutation.field!, value: cell),
          ],
          computed: table[at].computed,
        );
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.addOption:
        final database = databases[mutation.block]!;
        final option = BridgeOption(
          id: _id(),
          name: mutation.text!,
          color: mutation.color,
        );
        _setFields(mutation.block, [
          for (final field in database.fields)
            field.id == mutation.field
                ? BridgeField(
                    id: field.id,
                    name: field.name,
                    kind: field.kind,
                    options: [...field.options, option],
                    formula: '',
                  )
                : field,
        ]);
        _changed(_tableResource(mutation.block));
        return _done(option: option.id);
      case BridgeMutationKind.setFieldFormula:
        final database = databases[mutation.block]!;
        final at = database.fields.indexWhere(
          (field) => field.id == mutation.field,
        );
        if (at < 0) throw 'not found';
        final field = database.fields[at];
        if (field.kind != BridgeFieldKind.formula) {
          throw 'the value is not of its kind or not valid';
        }
        _setFields(mutation.block, [
          for (final known in database.fields)
            known.id == field.id
                ? BridgeField(
                    id: field.id,
                    name: field.name,
                    kind: field.kind,
                    options: field.options,
                    formula: mutation.text!,
                  )
                : known,
        ]);
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.renameField || BridgeMutationKind.removeField:
        final database = databases[mutation.block]!;
        if (!database.fields.any((field) => field.id == mutation.field)) {
          throw 'not found';
        }
        final rename = mutation.kind == BridgeMutationKind.renameField;
        _setFields(mutation.block, [
          for (final field in database.fields)
            if (field.id != mutation.field)
              field
            else if (rename)
              BridgeField(
                id: field.id,
                name: mutation.text!,
                kind: field.kind,
                options: field.options,
                formula: field.formula,
              ),
        ]);
        if (!rename) {
          rows[mutation.block!] = [
            for (final row in rows[mutation.block]!)
              BridgeRow(
                id: row.id,
                cells: [
                  for (final cell in row.cells)
                    if (cell.field != mutation.field) cell,
                ],
                computed: row.computed,
              ),
          ];
        }
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.renameDatabase:
        final database = databases[mutation.block]!;
        databases[mutation.block!] = BridgeDatabase(
          name: mutation.text!,
          fields: database.fields,
        );
        _renameTable(mutation.block!, mutation.text!);
        _changed(_tableResource(mutation.block));
        return _done();
      case BridgeMutationKind.addDatabaseBlock:
        final layout = mutation.layout;
        final extra = switch (layout) {
          BridgeLayout.table => null,
          BridgeLayout.board => BridgeField(
            id: _id(),
            name: 'Status',
            kind: BridgeFieldKind.select,
            options: [
              for (final (color, name) in [
                'Not started',
                'In progress',
                'Done',
              ].indexed)
                BridgeOption(id: _id(), name: name, color: color),
            ],
            formula: '',
          ),
          BridgeLayout.calendar => BridgeField(
            id: _id(),
            name: 'Date',
            kind: BridgeFieldKind.date,
            options: const [],
            formula: '',
          ),
        };
        final view = _newView(
          BridgeBlockKind.database,
          title: mutation.text!,
          tabs: [
            tabOf(
              layout,
              id: _id(),
              group: layout == BridgeLayout.board ? extra?.id : null,
              date: layout == BridgeLayout.calendar ? extra?.id : null,
            ),
          ],
        );
        _newTable(view.block, _areaOf(mutation.page!), mutation.text!);
        databases[view.block] = BridgeDatabase(
          name: mutation.text!,
          fields: [
            BridgeField(
              id: _id(),
              name: 'Name',
              kind: BridgeFieldKind.text,
              options: const [],
              formula: '',
            ),
            ?extra,
          ],
        );
        rows[view.block] = [];
        return _placeView(mutation, view);
      case BridgeMutationKind.duplicateTable:
        final source = databases[mutation.block] ?? (throw 'not found');
        final view = _newView(
          BridgeBlockKind.database,
          title: mutation.text!,
          tabs: [for (final tab in mutation.tabs) withTabId(tab, _id())],
        );
        _newTable(view.block, _areaOf(mutation.page!), mutation.text!);
        databases[view.block] = BridgeDatabase(
          name: mutation.text!,
          fields: source.fields,
        );
        rows[view.block] = [...rows[mutation.block]!];
        return _placeView(mutation, view);
      case BridgeMutationKind.createTable:
        final table = _id();
        _newTable(table, mutation.area!, mutation.text!);
        databases[table] = BridgeDatabase(
          name: mutation.text!,
          fields: [
            BridgeField(
              id: _id(),
              name: 'Name',
              kind: BridgeFieldKind.text,
              options: const [],
              formula: '',
            ),
          ],
        );
        rows[table] = [];
        return _done(block: table);
      case BridgeMutationKind.addTableView:
        if (!tables.any((table) => table.id == mutation.block)) {
          throw 'not found';
        }
        return _placeView(
          mutation,
          BridgeView(
            id: _id(),
            block: mutation.block!,
            title: mutation.text!,
            boundary: false,
            kind: BridgeBlockKind.database,
            tabs: mutation.tabs.isEmpty
                ? [tabOf(BridgeLayout.table, id: _id())]
                : [for (final tab in mutation.tabs) withTabId(tab, _id())],
          ),
        );
      case BridgeMutationKind.deleteTable:
        if (!tables.any((table) => table.id == mutation.block)) {
          throw 'not found';
        }
        tables.removeWhere((table) => table.id == mutation.block);
        databases.remove(mutation.block);
        for (final MapEntry(key: page, value: shown) in views.entries) {
          final before = shown.length;
          shown.removeWhere(
            (view) =>
                view.kind == BridgeBlockKind.database &&
                view.block == mutation.block,
          );
          if (shown.length != before) _viewsChanged(page);
        }
        _tablesChanged();
        _changed(_tableResource(mutation.block));
        return _done();
      default:
        return null;
    }
  }

  void _tablesChanged() =>
      _changed(const BridgeResource(kind: BridgeResourceKind.tables));

  void _setFields(String? block, List<BridgeField> fields) =>
      databases[block!] = BridgeDatabase(
        name: databases[block]!.name,
        fields: fields,
      );

  void _newTable(String id, String area, String name) {
    tables.add(BridgeTable(id: id, area: area, name: name, protected: false));
    _tablesChanged();
  }

  void _renameTable(String id, String name) {
    final at = tables.indexWhere((table) => table.id == id);
    if (at < 0) return;
    tables[at] = BridgeTable(
      id: id,
      area: tables[at].area,
      name: name,
      protected: tables[at].protected,
    );
    final database = databases[id];
    if (database != null) {
      databases[id] = BridgeDatabase(name: name, fields: database.fields);
    }
    _tablesChanged();
  }

  List<BridgeShown> shownIn(String table) => [
    for (final MapEntry(key: page, value: shown) in views.entries)
      for (final view in shown)
        if (view.kind == BridgeBlockKind.database && view.block == table)
          BridgeShown(
            page: page,
            view: view.id,
            title: view.title,
            tabs: view.tabs,
          ),
  ];

  BridgeResource _tableResource(String? block) =>
      BridgeResource(kind: BridgeResourceKind.table, block: block);
}

BridgeTab tabOf(
  BridgeLayout layout, {
  String id = '',
  String? name,
  String? group,
  String? date,
  BridgeSort? sort,
  BridgeFilter? filter,
}) => BridgeTab(
  id: id,
  name:
      name ??
      switch (layout) {
        BridgeLayout.table => 'Table',
        BridgeLayout.board => 'Board',
        BridgeLayout.calendar => 'Calendar',
      },
  layout: layout,
  group: group,
  date: date,
  sort: sort,
  filter: filter,
  folded: const [],
  totals: const [],
);

BridgeTab withTabId(BridgeTab tab, String id) => changedTab(tab, id: id);

BridgeTab changedTab(
  BridgeTab tab, {
  String? id,
  String? name,
  BridgeLayout? layout,
  (String?,)? group,
  (String?,)? date,
  (BridgeSort?,)? sort,
  (BridgeFilter?,)? filter,
  List<BridgeCell>? folded,
  List<BridgeFieldTotal>? totals,
}) => BridgeTab(
  id: id ?? tab.id,
  name: name ?? tab.name,
  layout: layout ?? tab.layout,
  group: group == null ? tab.group : group.$1,
  date: date == null ? tab.date : date.$1,
  sort: sort == null ? tab.sort : sort.$1,
  filter: filter == null ? tab.filter : filter.$1,
  folded: folded ?? tab.folded,
  totals: totals ?? tab.totals,
);
