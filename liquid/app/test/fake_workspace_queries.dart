part of 'fake_workspace.dart';

extension _FakeQueries on FakeWorkspace {
  BridgeComputed _total(List<BridgeRow> rows, BridgeFieldTotal total) {
    final cells = [
      for (final row in rows)
        row.cells.where((cell) => cell.field == total.field).firstOrNull?.value,
    ];
    final numbers = [
      for (final cell in cells)
        if (cell?.kind == BridgeCellKind.number) cell!.number,
    ];
    final filled = cells.where((cell) => cell != null).length;
    final number = switch (total.total) {
      BridgeTotal.count => rows.length.toDouble(),
      BridgeTotal.empty => (rows.length - filled).toDouble(),
      BridgeTotal.filled => filled.toDouble(),
      BridgeTotal.sum => numbers.fold(0.0, (sum, n) => sum + n),
      BridgeTotal.average =>
        numbers.fold(0.0, (sum, n) => sum + n) / numbers.length,
      BridgeTotal.min => numbers.reduce((a, b) => a < b ? a : b),
      BridgeTotal.max => numbers.reduce((a, b) => a > b ? a : b),
    };
    return BridgeComputed(
      field: total.field,
      value: '$number',
      kind: BridgeValueKind.number,
      number: number,
    );
  }

  BridgeAnswer _answer(BridgeQuery query) {
    List<T> window<T>(List<T> all) =>
        all.skip(query.start).take(query.limit).toList();
    BridgeAnswer answer({
      List<BridgeArea> areas = const [],
      List<BridgePage> pages = const [],
      List<BridgeView> views = const [],
      int count = 0,
      bool arranged = false,
      String? text,
      BridgeTextStyle? style,
      String? language,
      BridgeMedia? media,
      BridgeDrawing? drawing,
      BridgeDatabase? database,
      List<BridgeRow> rows = const [],
      List<BridgeGroup> groups = const [],
      List<BridgeHit> hits = const [],
      List<BridgeTable> tables = const [],
      List<BridgeShown> shown = const [],
      String? icon,
      List<BridgeLine> lines = const [],
      List<BridgeComputed> totals = const [],
      BridgeChart? chart,
      List<BridgeResource> follows = const [],
      List<BridgeWorkspaceAutomation> automations = const [],
      List<BridgeRole> roles = const [],
      List<BridgeRoleAllows> allows = const [],
    }) => BridgeAnswer(
      areas: areas,
      pages: pages,
      views: views,
      count: count,
      arranged: arranged,
      text: text,
      style: style,
      language: language,
      media: media,
      drawing: drawing,
      database: database,
      rows: rows,
      groups: groups,
      hits: hits,
      tables: tables,
      shown: shown,
      icon: icon,
      lines: lines,
      totals: totals,
      chart: chart,
      follows: follows,
      automations: automations,
      roles: roles,
      allows: allows,
    );
    final code = _isCode(query.block);
    return switch (query.kind) {
      BridgeQueryKind.areas => answer(
        areas: window(areas),
        count: areas.length,
      ),
      BridgeQueryKind.pages => answer(
        pages: window(pages[query.area]!),
        count: pages[query.area]!.length,
      ),
      BridgeQueryKind.views => answer(
        views: [
          for (final view in window(views[query.page] ?? const <BridgeView>[]))
            _decoded(view),
        ],
        count: views[query.page]?.length ?? 0,
        arranged: (views[query.page] ?? const <BridgeView>[]).any(
          (view) => view.tile != null,
        ),
      ),
      BridgeQueryKind.text => answer(
        text: code ? null : texts[query.block],
        style: code ? null : styles[query.block],
      ),
      BridgeQueryKind.code => answer(
        text: code ? texts[query.block] : null,
        language: code ? languages[query.block] ?? '' : null,
      ),
      BridgeQueryKind.math => answer(
        text: _kindOf(query.page!, query.block!) == BridgeBlockKind.math
            ? texts[query.block]
            : null,
      ),
      BridgeQueryKind.media => answer(media: media[query.block]),
      BridgeQueryKind.drawing => answer(
        drawing: switch (whiteboards[query.block]) {
          final board? => BridgeDrawing(
            style: board.style,
            elements: _withinBudget([
              for (final element in board.elements)
                if (element.docPage >= query.start &&
                    element.docPage < query.start + query.limit)
                  element,
            ]),
          ),
          null when media.containsKey(query.block) => const BridgeDrawing(
            style: BridgeDrawingStyle.handDrawn,
            elements: [],
          ),
          null => null,
        },
      ),
      BridgeQueryKind.database => answer(database: databases[query.block]),
      BridgeQueryKind.chart => answer(
        chart: switch (charts[query.block]) {
          final setup? => BridgeChart(
            setup: setup,
            source: chartSource(setup),
            columns: [
              if (setup.source case final source?)
                ...?sourceFiles[(source, '')]?.columns,
            ],
            points: chartSource(setup) == null
                ? const []
                : chartPoints[query.block] ?? const [],
          ),
          null => null,
        },
      ),
      BridgeQueryKind.calculation => answer(
        lines: [
          for (final kept in calculations[query.block] ?? const <BridgeLine>[])
            line(
              kept.id,
              BridgeLineSetup(
                label: kept.label,
                formula: kept.formula,
                style: kept.style,
              ),
            ),
        ],
        follows: {
          for (final kept in calculations[query.block] ?? const <BridgeLine>[])
            for (final token in RegExp(
              r'\{([^}.]+)\.[^}.]+\}',
            ).allMatches(kept.formula))
              BridgeResource(kind: BridgeResourceKind.table, block: token[1]),
        }.toList(),
      ),
      BridgeQueryKind.rows => () {
        final shown = _shownRows(query);
        return answer(
          count: shown.length,
          rows: window(shown),
          totals: [for (final total in query.totals) _total(shown, total)],
        );
      }(),
      BridgeQueryKind.groups => answer(groups: _groups(query)),
      BridgeQueryKind.search => answer(hits: _search(query)),
      BridgeQueryKind.workspace => answer(text: name, icon: icon),
      BridgeQueryKind.tables => answer(
        count: tables.length,
        tables: window(tables),
      ),
      BridgeQueryKind.roles => answer(count: roles.length, roles: [...roles]),
      BridgeQueryKind.allows => answer(
        allows: [
          for (final role in roles)
            BridgeRoleAllows(
              role: role.id,
              allows:
                  allowances[query.block]?[role.id] ??
                  role.setup.grants
                      .where(
                        (grant) =>
                            grant.kind == BridgeTargetKind.workspace ||
                            grant.id == query.block,
                      )
                      .map((grant) => grant.allows)
                      .firstOrNull ??
                  nothing,
            ),
        ],
      ),
      BridgeQueryKind.automations => answer(
        count: automations.length,
        automations: window(automations),
      ),
      BridgeQueryKind.tableViews => () {
        final shown = shownIn(query.block!);
        return answer(count: shown.length, shown: window(shown));
      }(),
    };
  }

  BridgeView _decoded(BridgeView view) => _withView(
    view,
    tabs: [
      for (final tab in view.tabs)
        changedTab(
          tab,
          filter: switch (tab.filter) {
            final filter? => (
              BridgeFilter(
                field: filter.field,
                matches: filter.matches,
                value: switch (filter.value) {
                  final value? => BridgeCell(
                    kind: value.kind,
                    text: value.text,
                    number: value.number,
                    checked: value.checked,
                    files: [...value.files],
                  ),
                  null => null,
                },
                until: filter.until,
              ),
            ),
            null => null,
          },
        ),
    ],
  );

  List<BridgeHit> _search(BridgeQuery query) {
    final words = (query.search ?? '')
        .toLowerCase()
        .split(RegExp(r'[^\p{L}\p{N}]+', unicode: true))
        .where((word) => word.isNotEmpty)
        .toList();
    if (words.isEmpty) return const [];
    bool found(String text) {
      final held = text.toLowerCase().split(
        RegExp(r'[^\p{L}\p{N}]+', unicode: true),
      );
      return words.every((word) => held.any((one) => one.startsWith(word)));
    }

    return [
      for (final tree in pages.values)
        for (final page in tree)
          if (found(page.title))
            BridgeHit(
              found: BridgeFound.page,
              id: page.id,
              page: page.id,
              text: page.title,
            ),
      for (final table in tables)
        if (found(table.name))
          BridgeHit(found: BridgeFound.table, id: table.id, text: table.name),
      for (final MapEntry(key: page, value: shown) in views.entries)
        for (final view in shown)
          if (view.kind == BridgeBlockKind.text &&
              found(texts[view.block] ?? ''))
            BridgeHit(
              found: BridgeFound.text,
              id: view.block,
              page: page,
              text: texts[view.block]!,
            ),
    ];
  }

  List<BridgeGroup> _groups(BridgeQuery query) {
    final field = databases[query.block]!.fields.firstWhere(
      (field) => field.id == query.field,
    );
    final shown = _shownRows(
      BridgeQuery(
        kind: BridgeQueryKind.rows,
        block: query.block,
        start: 0,
        limit: 0,
        sort: query.sort,
        filter: query.filter,
        search: query.search,
        totals: const [],
      ),
    );
    BridgeCell? held(BridgeRow row) =>
        row.cells.where((cell) => cell.field == field.id).firstOrNull?.value;
    final values = field.kind == BridgeFieldKind.checkbox
        ? [checkboxCell(false), checkboxCell(true)]
        : [
            emptyCell,
            for (final option in field.options) selectCell(option.id),
          ];
    List<BridgeRow> within(BridgeCell value) => [
      for (final row in shown)
        if (switch (value.kind) {
          BridgeCellKind.checkbox =>
            (held(row)?.checked ?? false) == value.checked,
          BridgeCellKind.empty => held(row) == null,
          _ => held(row)?.text == value.text,
        })
          row,
    ];
    return [
      for (final value in values)
        if (value.kind != BridgeCellKind.empty || within(value).isNotEmpty)
          BridgeGroup(
            value: value,
            count: within(value).length,
            rows: within(value).take(query.limit).toList(),
            totals: [
              for (final total in query.totals) _total(within(value), total),
            ],
          ),
    ];
  }

  List<BridgeRow> _shownRows(BridgeQuery query) {
    BridgeCell? cellOf(BridgeRow row, String field) =>
        row.cells.where((cell) => cell.field == field).firstOrNull?.value;
    bool passes(BridgeRow row, BridgeFilter filter) {
      final cell = cellOf(row, filter.field);
      final value = filter.value;
      return switch (filter.matches) {
        BridgeMatch.between =>
          cell?.text != null &&
              cell!.text!.compareTo(value!.text!) >= 0 &&
              cell.text!.compareTo(filter.until!) <= 0,
        BridgeMatch.empty => cell == null,
        BridgeMatch.notEmpty => cell != null,
        BridgeMatch.contains =>
          cell?.text?.toLowerCase().contains(value!.text!.toLowerCase()) ??
              false,
        BridgeMatch.is_ =>
          (cell != null &&
                  cell.kind == value!.kind &&
                  cell.text == value.text &&
                  cell.number == value.number &&
                  cell.checked == value.checked) ||
              (cell == null &&
                  value!.kind == BridgeCellKind.checkbox &&
                  !value.checked),
        BridgeMatch.isNot => !passes(
          row,
          BridgeFilter(
            field: filter.field,
            matches: BridgeMatch.is_,
            value: value,
          ),
        ),
        BridgeMatch.above || BridgeMatch.below => throw UnimplementedError(
          'Automation conditions filter no View',
        ),
      };
    }

    final search = query.search?.toLowerCase() ?? '';
    final options = {
      for (final field in databases[query.block]?.fields ?? <BridgeField>[])
        for (final option in field.options) option.id: option.name,
    };
    bool found(BridgeRow row) => row.cells.any(
      (cell) => switch (cell.value.kind) {
        BridgeCellKind.text => cell.value.text!.toLowerCase().contains(search),
        BridgeCellKind.select =>
          options[cell.value.text]?.toLowerCase().contains(search) ?? false,
        _ => false,
      },
    );
    final shown = [
      for (final row in rows[query.block] ?? const <BridgeRow>[])
        if ((query.filter == null || passes(row, query.filter!)) &&
            (query.within == null || passes(row, query.within!)) &&
            (search.isEmpty || found(row)))
          row,
    ];
    if (query.sort case final sort?) {
      Comparable<Object?>? key(BridgeRow row) {
        final cell = cellOf(row, sort.field);
        return cell == null
            ? null
            : (cell.kind == BridgeCellKind.number
                      ? cell.number
                      : cell.text ?? '')
                  as Comparable<Object?>;
      }

      final places = {for (final (at, row) in shown.indexed) row.id: at};
      shown.sort((a, b) {
        final order = switch ((key(a), key(b))) {
          (null, null) => 0,
          (null, _) => 1,
          (_, null) => -1,
          (final a?, final b?) =>
            sort.descending ? b.compareTo(a) : a.compareTo(b),
        };
        return order != 0 ? order : places[a.id]!.compareTo(places[b.id]!);
      });
    }
    return shown;
  }
}

const emptyCell = BridgeCell(
  kind: BridgeCellKind.empty,
  number: 0,
  checked: false,
  files: [],
);

BridgeCell checkboxCell(bool checked) =>
    _cell(BridgeCellKind.checkbox, checked: checked);

BridgeCell selectCell(String option) =>
    _cell(BridgeCellKind.select, text: option);

BridgeCell _cell(BridgeCellKind kind, {String? text, bool checked = false}) =>
    BridgeCell(
      kind: kind,
      text: text,
      number: 0,
      checked: checked,
      files: const [],
    );
