import 'dart:async';
import 'dart:typed_data';

import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/media.dart';
import 'package:liquid/src/rust/api/replica.dart';
import 'package:liquid/src/rust/api/whiteboard.dart';
import 'package:liquid/src/rust/api/workspace.dart';

/// An in-memory Workspace behind the bridge's calls: areas and Page trees, with a change for
/// every write to a followed resource, as the bridge sends them.
class FakeWorkspace implements LiquidWorkspace {
  final areas = <BridgeArea>[];
  final pages = <String, List<BridgePage>>{};
  final applied = <BridgeMutation>[];

  /// Each Page's Views in order, each Text or Code Block's text, each Text Block's style
  /// and each Code Block's language other than plain text.
  final views = <String, List<BridgeView>>{};
  final texts = <String, String>{};
  final styles = <String, BridgeTextStyle>{};
  final languages = <String, String>{};

  /// The files on disk the app may add, by path; the finished attachments, by name; each
  /// Media Block's file; and each copy saved, by path.
  final files = <String, Uint8List>{};
  final attachments = <String, Uint8List>{};
  final media = <String, BridgeMedia>{};
  final saved = <String, Uint8List>{};

  /// Holds each file's last step, its name, until it completes.
  Completer<void>? uploadGate;

  /// Each Database Block's name and fields, and its rows in order.
  final databases = <String, BridgeDatabase>{};
  final rows = <String, List<BridgeRow>>{};

  /// Each Whiteboard's style and elements, bottom to top.
  final whiteboards = <String, BridgeWhiteboard>{};

  /// Fails every apply with this text.
  String? refuse;

  /// Fails this many of the next applies.
  int refuseNext = 0;

  /// Fails every apply of these kinds.
  Set<BridgeMutationKind> refuseKinds = {};

  /// Holds applies until it completes; only those of [gatedKinds] when set.
  Completer<void>? gate;
  Set<BridgeMutationKind>? gatedKinds;

  /// How long each apply takes, as a slow disk would.
  Duration Function()? delay;

  /// Every query asked, in order.
  final asked = <BridgeQuery>[];

  /// Fails every query with this text.
  String? refuseQueries;

  /// Fails the next query of this kind, once.
  BridgeQueryKind? refuseNextQuery;

  /// While set, each query of [deferredKinds] answers from the state at its call but waits
  /// for its own release.
  List<Completer<void>>? deferred;
  Set<BridgeQueryKind> deferredKinds = BridgeQueryKind.values.toSet();

  final _followers = <int, (BridgeResource, StreamController<BridgeChange>)>{};
  final _seqs = <BridgeResource, int>{};
  int _ids = 0;

  /// The followed resources, once each.
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

  /// Holds each answer after its write applied and its changes went out, until it
  /// completes, as the bridge may answer after them.
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
        // After the parent's last descendant, as the core orders a tree.
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
        final page = views.putIfAbsent(mutation.page!, () => []);
        final at = mutation.after == null
            ? 0
            : page.indexWhere((view) => view.id == mutation.after) + 1;
        final view = _block(
          mutation.page!,
          at,
          mutation.text!,
          BridgeTextStyle.paragraph,
        );
        return BridgeApplied(elements: const [], action: _id(), block: view.block, view: view.id);
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
          // At the start the new empty Text Block goes above and takes the boundary.
          final shown = page[at];
          page[at] = BridgeView(
            id: shown.id,
            block: shown.block,
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
        // The core joins only Text Blocks.
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
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
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
            boundary: shown.boundary,
            kind: to,
          );
        }
        languages.remove(mutation.block);
        if (!code) styles[mutation.block!] = BridgeTextStyle.paragraph;
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
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
          boundary: mutation.boundary,
          kind: shown.kind,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.removeView:
        views[mutation.page]!.removeWhere((view) => view.id == mutation.view);
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addRow:
        final table = rows[mutation.block]!;
        // The core refuses a row to go after one it does not have.
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
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id(), row: row.id);
      case BridgeMutationKind.removeRow:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        table.removeAt(at);
        _changed(_blockResource(mutation));
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
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id(), field: field.id);
      case BridgeMutationKind.setCell:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        final cell = mutation.cell!;
        final stored = table[at].cells
            .where((kept) => kept.field == mutation.field)
            .firstOrNull
            ?.value;
        // The core refuses a write that changes nothing.
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
        _changed(_blockResource(mutation));
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
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id(), option: option.id);
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
          // The field's values go with it.
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
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.renameDatabase:
        final database = databases[mutation.block]!;
        databases[mutation.block!] = BridgeDatabase(
          name: mutation.text!,
          fields: database.fields,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.addMediaBlock:
        final file = attachments[mutation.attachment]!;
        final page = views.putIfAbsent(mutation.page!, () => []);
        final view = BridgeView(
          id: _id(),
          block: _id(),
          boundary: false,
          kind: BridgeBlockKind.media,
        );
        media[view.block] = BridgeMedia(
          name: mutation.text!,
          mediaType: mutation.mediaType ?? '',
          attachment: mutation.attachment!,
          size: file.length,
        );
        page.insert(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          view,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(elements: const [], action: _id(), block: view.block, view: view.id);
      case BridgeMutationKind.addWhiteboardBlock:
        final page = views.putIfAbsent(mutation.page!, () => []);
        final view = BridgeView(
          id: _id(),
          block: _id(),
          boundary: false,
          kind: BridgeBlockKind.whiteboard,
        );
        whiteboards[view.block] = const BridgeWhiteboard(
          style: BridgeWhiteboardStyle.handDrawn,
          elements: [],
        );
        page.insert(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          view,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(
          elements: const [],
          action: _id(),
          block: view.block,
          view: view.id,
        );
      case BridgeMutationKind.setWhiteboardStyle:
        final board = whiteboards[mutation.block] ?? (throw 'not found');
        if (board.style == mutation.whiteboardStyle) {
          throw 'the write changes nothing';
        }
        whiteboards[mutation.block!] = BridgeWhiteboard(
          style: mutation.whiteboardStyle!,
          elements: board.elements,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: const [], action: _id());
      case BridgeMutationKind.editWhiteboard:
        final board = whiteboards[mutation.block] ?? (throw 'not found');
        final edit = mutation.whiteboard!;
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
        whiteboards[mutation.block!] = BridgeWhiteboard(
          style: board.style,
          elements: elements,
        );
        _changed(_blockResource(mutation));
        return BridgeApplied(elements: ids, action: _id());
      case BridgeMutationKind.addDatabaseBlock:
        final page = views.putIfAbsent(mutation.page!, () => []);
        final view = BridgeView(
          id: _id(),
          block: _id(),
          boundary: false,
          kind: BridgeBlockKind.database,
        );
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
        page.insert(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          view,
        );
        _changed(
          BridgeResource(kind: BridgeResourceKind.views, page: mutation.page),
        );
        return BridgeApplied(elements: const [], action: _id(), block: view.block, view: view.id);
    }
  }

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

  /// A new Text Block with its View at `at`.
  BridgeView _block(
    String page,
    int at,
    String text,
    BridgeTextStyle style, {
    bool boundary = false,
  }) {
    final view = BridgeView(
      id: _id(),
      block: _id(),
      boundary: boundary,
      kind: BridgeBlockKind.text,
    );
    texts[view.block] = text;
    styles[view.block] = style;
    views[page]!.insert(at, view);
    _changed(BridgeResource(kind: BridgeResourceKind.views, page: page));
    return view;
  }

  /// The Page's Text Blocks' texts in order.
  List<String> textsOf(String page) => [
    for (final view in views[page] ?? const <BridgeView>[])
      if (view.kind == BridgeBlockKind.text) texts[view.block]!,
  ];

  /// Puts `rows` into the Database `block`, as writes elsewhere would.
  void putRows(String page, String block, List<BridgeRow> rows) {
    this.rows[block] = rows;
    _changed(
      BridgeResource(kind: BridgeResourceKind.block, page: page, block: block),
    );
  }

  bool _below(List<BridgePage> tree, BridgePage page, String parent) {
    for (var at = page.parent; at != null;) {
      if (at == parent) return true;
      at = tree.firstWhere((page) => page.id == at).parent;
    }
    return false;
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
    return switch (query.kind) {
      BridgeQueryKind.areas => BridgeAnswer(
        areas: window(areas),
        pages: const [],
        views: const [],
        count: areas.length,
        rows: const [],
      ),
      BridgeQueryKind.pages => BridgeAnswer(
        areas: const [],
        pages: window(pages[query.area]!),
        views: const [],
        count: pages[query.area]!.length,
        rows: const [],
      ),
      BridgeQueryKind.views => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: window(views[query.page] ?? const []),
        count: views[query.page]?.length ?? 0,
        rows: const [],
      ),
      BridgeQueryKind.text => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        text: _isCode(query.block) ? null : texts[query.block],
        style: _isCode(query.block) ? null : styles[query.block],
        rows: const [],
      ),
      BridgeQueryKind.code => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        text: _isCode(query.block) ? texts[query.block] : null,
        language: _isCode(query.block) ? languages[query.block] ?? '' : null,
        rows: const [],
      ),
      BridgeQueryKind.media => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        media: media[query.block],
        rows: const [],
      ),
      BridgeQueryKind.whiteboard => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        whiteboard: whiteboards[query.block],
        rows: const [],
      ),
      BridgeQueryKind.database => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: 0,
        database: databases[query.block],
        rows: const [],
      ),
      BridgeQueryKind.rows => BridgeAnswer(
        areas: const [],
        pages: const [],
        views: const [],
        count: rows[query.block]?.length ?? 0,
        rows: window(rows[query.block] ?? const []),
      ),
    };
  }

  /// The name an attachment of `bytes` gets: its content, as the core names it by hash.
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
    // The first change acknowledges the subscription, as the bridge's does.
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

/// `element` as the core keeps it under `id`.
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
  pen: element.pen,
  points: element.points,
  shape: element.shape,
  seed: element.seed,
  arrowhead: element.arrowhead,
  corners: element.corners,
  text: element.text,
);

/// `element` after `change`, as the core applies one.
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
      pen: element.pen,
      points: element.points,
      shape: element.shape,
      seed: element.seed,
      arrowhead: change.arrowhead ?? element.arrowhead,
      corners: element.corners,
      text: change.text ?? element.text,
    );
