import 'dart:async';
import 'dart:typed_data';

import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/media.dart';
import 'package:liquid/src/rust/api/replica.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';

class FakeWorkspace implements LiquidWorkspace {
  final areas = <BridgeArea>[];
  final pages = <String, List<BridgePage>>{};
  final applied = <BridgeMutation>[];

  final views = <String, List<BridgeView>>{};
  final texts = <String, String>{};
  final styles = <String, BridgeTextStyle>{};
  final languages = <String, String>{};

  final files = <String, Uint8List>{};
  final attachments = <String, Uint8List>{};
  final media = <String, BridgeMedia>{};
  final saved = <String, Uint8List>{};

  Completer<void>? uploadGate;

  final databases = <String, BridgeDatabase>{};
  final rows = <String, List<BridgeRow>>{};

  final whiteboards = <String, BridgeDrawing>{};

  var readBudget = 1 << 30;

  String? refuse;

  int refuseNext = 0;

  Set<BridgeMutationKind> refuseKinds = {};

  Completer<void>? gate;
  Set<BridgeMutationKind>? gatedKinds;

  Duration Function()? delay;

  final asked = <BridgeQuery>[];

  String? refuseQueries;

  BridgeQueryKind? refuseNextQuery;

  List<Completer<void>>? deferred;
  Set<BridgeQueryKind> deferredKinds = BridgeQueryKind.values.toSet();

  final _followers = <int, (BridgeResource, StreamController<BridgeChange>)>{};
  final _seqs = <BridgeResource, int>{};
  int _ids = 0;

  Set<BridgeResource> get followed => {
    for (final (resource, _) in _followers.values) resource,
  };

  String _id() => (_ids++).toRadixString(16).padLeft(32, '0');

  @override
  Future<BridgeApplied> apply({required BridgeMutation mutation}) async {
    final applied = await _apply(mutation);
    await answered?.future;
    return applied;
  }

  Completer<void>? answered;

  Future<BridgeApplied> _apply(BridgeMutation mutation) async {
    if (gatedKinds?.contains(mutation.kind) ?? true) await gate?.future;
    if (delay case final delay?) await Future<void>.delayed(delay());
    if (refuse case final refused?) throw refused;
    if (refuseKinds.contains(mutation.kind)) throw 'refused';
    if (refuseNext > 0) {
      refuseNext--;
      throw 'refused';
    }
    applied.add(mutation);
    switch (mutation.kind) {
      case BridgeMutationKind.createArea:
        final area = BridgeArea(id: _id(), name: mutation.text!);
        areas.add(area);
        pages[area.id] = [];
        _changed(const BridgeResource(kind: BridgeResourceKind.areas));
        return BridgeApplied(elements: const [], action: _id(), area: area.id);
      case BridgeMutationKind.createPage:
        final tree = pages[mutation.area]!;
        final page = BridgePage(
          id: _id(),
          parent: mutation.parent,
          title: mutation.text!,
        );
        var at = tree.length;
        if (mutation.parent case final parent?) {
          at = tree.indexWhere((page) => page.id == parent) + 1;
          while (at < tree.length && _below(tree, tree[at], parent)) {
            at++;
          }
        }
        tree.insert(at, page);
        _changed(
          BridgeResource(kind: BridgeResourceKind.pages, area: mutation.area),
        );
        return BridgeApplied(elements: const [], action: _id(), page: page.id);
      case BridgeMutationKind.renamePage:
        for (final MapEntry(key: area, value: tree) in pages.entries) {
          final at = tree.indexWhere((page) => page.id == mutation.page);
          if (at < 0) continue;
          final page = tree[at];
          tree[at] = BridgePage(
            id: page.id,
            parent: page.parent,
            title: mutation.text!,
          );
          _changed(BridgeResource(kind: BridgeResourceKind.pages, area: area));
        }
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addTextBlock:
        final view = _newView(BridgeBlockKind.text);
        texts[view.block] = mutation.text!;
        styles[view.block] = BridgeTextStyle.paragraph;
        return _placeView(mutation, view);
      case BridgeMutationKind.editText:
        final text = texts[mutation.block]!;
        texts[mutation.block!] = text.replaceRange(
          mutation.at,
          mutation.at + mutation.delete,
          mutation.text!,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.splitTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final text = texts[mutation.block]!;
        final BridgeView created;
        if (mutation.at == 0) {
          final shown = page[at];
          page[at] = BridgeView(
            id: shown.id,
            block: shown.block,
            title: shown.title,
            boundary: false,
            kind: shown.kind,
          );
          created = _block(
            mutation.page!,
            at,
            '',
            mutation.style,
            boundary: shown.boundary,
          );
        } else {
          texts[mutation.block!] = text.substring(0, mutation.at);
          _changed(_blockResource(mutation));
          created = _block(
            mutation.page!,
            at + 1,
            text.substring(mutation.at),
            mutation.style,
          );
        }
        return BridgeApplied(
          elements: const [],
          action: _id(),
          block: created.block,
          view: created.id,
        );
      case BridgeMutationKind.joinTextBlock:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        if (at <= 0 || page[at - 1].kind != BridgeBlockKind.text) {
          throw 'not found';
        }
        if (page[at].boundary) throw 'a Section starts here';
        final above = page[at - 1].block;
        texts[above] = texts[above]! + texts.remove(page[at].block)!;
        page.removeAt(at);
        _changed(
          BridgeResource(
            kind: BridgeResourceKind.block,
            page: mutation.page,
            block: above,
          ),
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.turnIntoCode || BridgeMutationKind.turnIntoText:
        final code = mutation.kind == BridgeMutationKind.turnIntoCode;
        final (from, to) = code
            ? (BridgeBlockKind.text, BridgeBlockKind.code)
            : (BridgeBlockKind.code, BridgeBlockKind.text);
        final page = views[mutation.page]!;
        if (_kindOf(mutation.page!, mutation.block!) != from) throw 'not found';
        for (final (at, shown) in page.indexed) {
          if (shown.block != mutation.block) continue;
          page[at] = BridgeView(
            id: shown.id,
            block: shown.block,
            title: shown.title,
            boundary: shown.boundary,
            kind: to,
          );
        }
        languages.remove(mutation.block);
        if (!code) styles[mutation.block!] = BridgeTextStyle.paragraph;
        _viewsChanged(mutation.page);
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.setCodeLanguage:
        if (_kindOf(mutation.page!, mutation.block!) != BridgeBlockKind.code) {
          throw 'not found';
        }
        if (mutation.text!.isEmpty) {
          languages.remove(mutation.block);
        } else {
          languages[mutation.block!] = mutation.text!;
        }
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.setTextStyle:
        styles[mutation.block!] = mutation.style;
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.setSectionBoundary:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final shown = page[at];
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          title: shown.title,
          boundary: mutation.boundary,
          kind: shown.kind,
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.setViewSort || BridgeMutationKind.setViewFilter:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final shown = page[at];
        final sorts = mutation.kind == BridgeMutationKind.setViewSort;
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          title: shown.title,
          boundary: shown.boundary,
          kind: shown.kind,
          sort: sorts ? mutation.sort : shown.sort,
          filter: sorts ? shown.filter : mutation.filter,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.removeView:
        views[mutation.page]!.removeWhere((view) => view.id == mutation.view);
        _viewsChanged(mutation.page);
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addRow:
        final table = rows[mutation.block]!;
        if (mutation.after case final after?
            when !table.any((row) => row.id == after)) {
          throw 'not found';
        }
        final row = BridgeRow(id: _id(), cells: const []);
        table.insert(
          mutation.last
              ? table.length
              : mutation.after == null
              ? 0
              : table.indexWhere((row) => row.id == mutation.after) + 1,
          row,
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id(), row: row.id);
      case BridgeMutationKind.removeRow:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        table.removeAt(at);
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id());
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
        );
        fields.insert(
          mutation.last
              ? fields.length
              : mutation.after == null
              ? 0
              : fields.indexWhere((field) => field.id == mutation.after) + 1,
          field,
        );
        databases[mutation.block!] = BridgeDatabase(
          name: database.name,
          fields: fields,
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(
          elements: const [],
          action: _id(),
          field: field.id,
        );
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
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addOption:
        final database = databases[mutation.block]!;
        final option = BridgeOption(
          id: _id(),
          name: mutation.text!,
          color: mutation.color,
        );
        databases[mutation.block!] = BridgeDatabase(
          name: database.name,
          fields: [
            for (final field in database.fields)
              field.id == mutation.field
                  ? BridgeField(
                      id: field.id,
                      name: field.name,
                      kind: field.kind,
                      options: [...field.options, option],
                    )
                  : field,
          ],
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(
          elements: const [],
          action: _id(),
          option: option.id,
        );
      case BridgeMutationKind.renameField || BridgeMutationKind.removeField:
        final database = databases[mutation.block]!;
        if (!database.fields.any((field) => field.id == mutation.field)) {
          throw 'not found';
        }
        final rename = mutation.kind == BridgeMutationKind.renameField;
        databases[mutation.block!] = BridgeDatabase(
          name: database.name,
          fields: [
            for (final field in database.fields)
              if (field.id != mutation.field)
                field
              else if (rename)
                BridgeField(
                  id: field.id,
                  name: mutation.text!,
                  kind: field.kind,
                  options: field.options,
                ),
          ],
        );
        if (!rename) {
          rows[mutation.block!] = [
            for (final row in rows[mutation.block]!)
              BridgeRow(
                id: row.id,
                cells: [
                  for (final cell in row.cells)
                    if (cell.field != mutation.field) cell,
                ],
              ),
          ];
        }
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.renameDatabase:
        final database = databases[mutation.block]!;
        databases[mutation.block!] = BridgeDatabase(
          name: mutation.text!,
          fields: database.fields,
        );
        _renameTable(mutation.block!, mutation.text!);
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addMediaBlock:
        final file = attachments[mutation.attachment]!;
        final view = _newView(BridgeBlockKind.media);
        media[view.block] = BridgeMedia(
          name: mutation.text!,
          mediaType: mutation.mediaType ?? '',
          attachment: mutation.attachment!,
          size: file.length,
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.addWhiteboardBlock:
        final view = _newView(BridgeBlockKind.whiteboard);
        whiteboards[view.block] = const BridgeDrawing(
          style: BridgeDrawingStyle.handDrawn,
          elements: [],
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.setDrawingStyle:
        final board = whiteboards[mutation.block] ?? (throw 'not found');
        if (board.style == mutation.drawingStyle) {
          throw 'the write changes nothing';
        }
        whiteboards[mutation.block!] = BridgeDrawing(
          style: mutation.drawingStyle!,
          elements: board.elements,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.editDrawing:
        final board =
            whiteboards[mutation.block] ??
            (media.containsKey(mutation.block)
                ? const BridgeDrawing(
                    style: BridgeDrawingStyle.handDrawn,
                    elements: [],
                  )
                : throw 'not found');
        final edit = mutation.drawing!;
        final elements = [...board.elements];
        for (final removed in edit.remove) {
          final at = elements.indexWhere((element) => element.id == removed);
          if (at < 0) throw 'not found';
          elements.removeAt(at);
        }
        for (final change in edit.change) {
          final at = elements.indexWhere((element) => element.id == change.id);
          if (at < 0) throw 'not found';
          elements[at] = changed(elements[at], change);
        }
        final ids = [for (final _ in edit.add) _id()];
        for (final (index, element) in edit.add.indexed) {
          elements.add(withId(element, ids[index]));
        }
        whiteboards[mutation.block!] = BridgeDrawing(
          style: board.style,
          elements: elements,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: ids, action: _id());
      case BridgeMutationKind.addDatabaseBlock:
        final view = _newView(BridgeBlockKind.database, title: mutation.text!);
        _newTable(view.block, _areaOf(mutation.page!), mutation.text!);
        databases[view.block] = BridgeDatabase(
          name: mutation.text!,
          fields: [
            BridgeField(
              id: _id(),
              name: 'Name',
              kind: BridgeFieldKind.text,
              options: const [],
            ),
          ],
        );
        rows[view.block] = [];
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
            ),
          ],
        );
        rows[table] = [];
        return BridgeApplied(elements: const [], action: _id(), block: table);
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
            sort: mutation.sort,
            filter: mutation.filter,
          ),
        );
      case BridgeMutationKind.renameView:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        final shown = page[at];
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          title: mutation.text!,
          boundary: shown.boundary,
          kind: shown.kind,
          sort: shown.sort,
          filter: shown.filter,
        );
        _viewsChanged(mutation.page);
        if (shown.kind == BridgeBlockKind.database &&
            tables.any(
              (table) => table.id == shown.block && table.name.isEmpty,
            )) {
          _renameTable(shown.block, mutation.text!);
        }
        return BridgeApplied(elements: const [], action: _id());
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
        _changed(const BridgeResource(kind: BridgeResourceKind.tables));
        _changed(_tableResource(mutation.block));
        return BridgeApplied(elements: const [], action: _id());
    }
  }

  final tables = <BridgeTable>[];

  void _newTable(String id, String area, String name) {
    tables.add(BridgeTable(id: id, area: area, name: name));
    _changed(const BridgeResource(kind: BridgeResourceKind.tables));
  }

  void _renameTable(String id, String name) {
    final at = tables.indexWhere((table) => table.id == id);
    if (at < 0) return;
    tables[at] = BridgeTable(id: id, area: tables[at].area, name: name);
    final database = databases[id];
    if (database != null) {
      databases[id] = BridgeDatabase(name: name, fields: database.fields);
    }
    _changed(const BridgeResource(kind: BridgeResourceKind.tables));
  }

  String _areaOf(String page) => pages.entries
      .firstWhere((entry) => entry.value.any((shown) => shown.id == page))
      .key;

  List<BridgeShown> shownIn(String table) => [
    for (final MapEntry(key: page, value: shown) in views.entries)
      for (final view in shown)
        if (view.kind == BridgeBlockKind.database && view.block == table)
          BridgeShown(
            page: page,
            view: view.id,
            title: view.title,
            sort: view.sort,
            filter: view.filter,
          ),
  ];

  BridgeResource _tableResource(String? block) =>
      BridgeResource(kind: BridgeResourceKind.table, block: block);

  BridgeView _newView(
    BridgeBlockKind kind, {
    bool boundary = false,
    String title = '',
  }) => BridgeView(
    id: _id(),
    block: _id(),
    title: title,
    boundary: boundary,
    kind: kind,
  );

  BridgeApplied _placeView(BridgeMutation mutation, BridgeView view) {
    final page = views.putIfAbsent(mutation.page!, () => []);
    page.insert(
      mutation.after == null
          ? 0
          : page.indexWhere((shown) => shown.id == mutation.after) + 1,
      view,
    );
    _viewsChanged(mutation.page);
    return BridgeApplied(
      elements: const [],
      action: _id(),
      block: view.block,
      view: view.id,
    );
  }

  void _viewsChanged(String? page) =>
      _changed(BridgeResource(kind: BridgeResourceKind.views, page: page));

  BridgeBlockKind? _kindOf(String page, String block) =>
      views[page]?.where((view) => view.block == block).firstOrNull?.kind;

  bool _isCode(String? block) => views.values.any(
    (page) => page.any(
      (view) => view.block == block && view.kind == BridgeBlockKind.code,
    ),
  );

  BridgeResource _blockResource(BridgeMutation mutation) => BridgeResource(
    kind: BridgeResourceKind.block,
    page: mutation.page,
    block: mutation.block,
  );

  BridgeView _block(
    String page,
    int at,
    String text,
    BridgeTextStyle style, {
    bool boundary = false,
  }) {
    final view = _newView(BridgeBlockKind.text, boundary: boundary);
    texts[view.block] = text;
    styles[view.block] = style;
    views[page]!.insert(at, view);
    _viewsChanged(page);
    return view;
  }

  List<String> textsOf(String page) => [
    for (final view in views[page] ?? const <BridgeView>[])
      if (view.kind == BridgeBlockKind.text) texts[view.block]!,
  ];

  void dropTable(String table) {
    tables.removeWhere((shown) => shown.id == table);
    databases.remove(table);
    _changed(const BridgeResource(kind: BridgeResourceKind.tables));
    _changed(_tableResource(table));
  }

  void putRows(String page, String block, List<BridgeRow> rows) {
    this.rows[block] = rows;
    _changed(_tableResource(block));
  }

  bool _below(List<BridgePage> tree, BridgePage page, String parent) {
    for (var at = page.parent; at != null;) {
      if (at == parent) return true;
      at = tree.firstWhere((page) => page.id == at).parent;
    }
    return false;
  }

  List<BridgeElement> _withinBudget(List<BridgeElement> elements) {
    final pages = {for (final element in elements) element.docPage}.toList()
      ..sort();
    var budget = readBudget;
    final kept = <int>{};
    for (final page in pages) {
      final count = elements.where((element) => element.docPage == page).length;
      if (count > budget) break;
      budget -= count;
      kept.add(page);
    }
    return [
      for (final element in elements)
        if (kept.contains(element.docPage)) element,
    ];
  }

  @override
  Future<BridgeAnswer> query({required BridgeQuery query}) async {
    asked.add(query);
    if (refuseQueries case final refused?) throw refused;
    if (refuseNextQuery == query.kind) {
      refuseNextQuery = null;
      throw 'refused';
    }
    final answer = _answer(query);
    if (deferred case final held? when deferredKinds.contains(query.kind)) {
      final release = Completer<void>();
      held.add(release);
      await release.future;
    }
    return answer;
  }

  BridgeAnswer _answer(BridgeQuery query) {
    List<T> window<T>(List<T> all) =>
        all.skip(query.start).take(query.limit).toList();
    BridgeAnswer answer({
      List<BridgeArea> areas = const [],
      List<BridgePage> pages = const [],
      List<BridgeView> views = const [],
      int count = 0,
      String? text,
      BridgeTextStyle? style,
      String? language,
      BridgeMedia? media,
      BridgeDrawing? drawing,
      BridgeDatabase? database,
      List<BridgeRow> rows = const [],
      List<BridgeTable> tables = const [],
      List<BridgeShown> shown = const [],
    }) => BridgeAnswer(
      areas: areas,
      pages: pages,
      views: views,
      count: count,
      text: text,
      style: style,
      language: language,
      media: media,
      drawing: drawing,
      database: database,
      rows: rows,
      tables: tables,
      shown: shown,
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
      ),
      BridgeQueryKind.text => answer(
        text: code ? null : texts[query.block],
        style: code ? null : styles[query.block],
      ),
      BridgeQueryKind.code => answer(
        text: code ? texts[query.block] : null,
        language: code ? languages[query.block] ?? '' : null,
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
      BridgeQueryKind.rows => () {
        final shown = _shownRows(query);
        return answer(count: shown.length, rows: window(shown));
      }(),
      BridgeQueryKind.tables => answer(
        count: tables.length,
        tables: window(tables),
      ),
      BridgeQueryKind.tableViews => () {
        final shown = shownIn(query.block!);
        return answer(count: shown.length, shown: window(shown));
      }(),
    };
  }

  BridgeView _decoded(BridgeView view) {
    final filter = view.filter;
    final value = filter?.value;
    if (filter == null || value == null) return view;
    return BridgeView(
      id: view.id,
      block: view.block,
      title: view.title,
      boundary: view.boundary,
      kind: view.kind,
      sort: view.sort,
      filter: BridgeFilter(
        field: filter.field,
        matches: filter.matches,
        value: BridgeCell(
          kind: value.kind,
          text: value.text,
          number: value.number,
          checked: value.checked,
          files: [...value.files],
        ),
      ),
    );
  }

  List<BridgeRow> _shownRows(BridgeQuery query) {
    BridgeCell? cellOf(BridgeRow row, String field) =>
        row.cells.where((cell) => cell.field == field).firstOrNull?.value;
    bool passes(BridgeRow row, BridgeFilter filter) {
      final cell = cellOf(row, filter.field);
      final value = filter.value;
      return switch (filter.matches) {
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

  String _attachment(Uint8List bytes) {
    final name = Object.hashAll(bytes).toUnsigned(32).toRadixString(16);
    attachments[name.padLeft(64, '0')] = bytes;
    return name.padLeft(64, '0');
  }

  @override
  Future<String> addBytes({required List<int> bytes}) async =>
      _attachment(Uint8List.fromList(bytes));

  @override
  Stream<BridgeUpload> addFile({required String path}) async* {
    final bytes = files[path];
    if (bytes == null) throw '$path: No such file or directory';
    const part = 1 << 20;
    for (var at = part; at < bytes.length; at += part) {
      yield BridgeUpload(written: at, size: bytes.length);
    }
    await uploadGate?.future;
    yield BridgeUpload(
      written: bytes.length,
      size: bytes.length,
      attachment: _attachment(bytes),
    );
  }

  @override
  Future<Uint8List> readAttachment({required String attachment}) async =>
      attachments[attachment] ?? (throw 'not found');

  @override
  Future<void> saveAttachment({
    required String attachment,
    required String path,
  }) async => saved[path] = attachments[attachment] ?? (throw 'not found');

  @override
  Stream<BridgeChange> subscribe({
    required int subscription,
    required BridgeResource resource,
  }) {
    final controller = StreamController<BridgeChange>();
    _followers[subscription] = (resource, controller);
    controller.add(BridgeChange(resource: resource, seq: _seqs[resource] ?? 0));
    return controller.stream;
  }

  @override
  Future<void> close({required int subscription}) async {
    await _followers.remove(subscription)?.$2.close();
  }

  void _changed(BridgeResource resource) {
    final seq = _seqs[resource] = (_seqs[resource] ?? 0) + 1;
    for (final (followed, controller) in _followers.values) {
      if (followed == resource) {
        controller.add(BridgeChange(resource: resource, seq: seq));
      }
    }
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

BridgeElement withId(BridgeElement element, String id) => BridgeElement(
  id: id,
  kind: element.kind,
  x: element.x,
  y: element.y,
  w: element.w,
  h: element.h,
  rotation: element.rotation,
  color: element.color,
  width: element.width,
  docPage: element.docPage,
  pen: element.pen,
  points: element.points,
  shape: element.shape,
  seed: element.seed,
  arrowhead: element.arrowhead,
  corners: element.corners,
  text: element.text,
);

BridgeElement changed(BridgeElement element, BridgeElementChange change) =>
    BridgeElement(
      id: element.id,
      kind: element.kind,
      x: change.x,
      y: change.y,
      w: change.w,
      h: change.h,
      rotation: change.rotation,
      color: change.color,
      width: change.width,
      docPage: element.docPage,
      pen: element.pen,
      points: element.points,
      shape: element.shape,
      seed: element.seed,
      arrowhead: change.arrowhead ?? element.arrowhead,
      corners: element.corners,
      text: change.text ?? element.text,
    );
