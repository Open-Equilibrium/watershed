import 'dart:async';
import 'dart:typed_data';

import 'package:liquid/src/rust/api/automation.dart';
import 'package:liquid/src/rust/api/calculation.dart';
import 'package:liquid/src/rust/api/chart.dart';
import 'package:liquid/src/rust/api/database.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/rust/api/media.dart';
import 'package:liquid/src/rust/api/replica.dart';
import 'package:liquid/src/rust/api/role.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';

const canManage = '00000000000000000000000000000001';
const canEdit = '00000000000000000000000000000002';
const canView = '00000000000000000000000000000003';

const everything = BridgeAllows(
  read: true,
  propose: true,
  apply: true,
  approve: true,
  manage: true,
);

const nothing = BridgeAllows(
  read: false,
  propose: false,
  apply: false,
  approve: false,
  manage: false,
);

class FakeWorkspace implements LiquidWorkspace {
  @override
  bool isDisposed = false;

  @override
  void dispose() => isDisposed = true;

  String name = '';
  String icon = '';
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

  final calculations = <String, List<BridgeLine>>{};

  final charts = <String, BridgeChartSetup>{};

  final chartPoints = <String, List<BridgeChartPoint>>{};

  String? chartSource(BridgeChartSetup setup) => switch (setup) {
    BridgeChartSetup(:final table?) => databases[table]?.name,
    BridgeChartSetup(:final page?, :final block?) =>
      views[page]?.where((view) => view.block == block).firstOrNull?.title,
    _ => null,
  };

  final computed = <String, String>{};

  final referenceNames = <String, String>{};

  BridgeLine line(String id, BridgeLineSetup setup) {
    final formula = setup.formula.trim();
    final literal = RegExp(r'^"(.*)"$').firstMatch(formula);
    final value =
        computed[formula] ??
        (double.tryParse(formula) != null ? formula : literal?[1] ?? '');
    return BridgeLine(
      id: id,
      label: setup.label,
      formula: setup.formula,
      style: setup.style,
      value: value,
      kind: value.isEmpty
          ? BridgeValueKind.blank
          : value.startsWith('#')
          ? BridgeValueKind.error
          : double.tryParse(value) != null
          ? BridgeValueKind.number
          : BridgeValueKind.text,
      references: [
        for (final token in RegExp(r'\{[0-9a-f.]+\}').allMatches(formula))
          referenceNames[token[0]],
      ],
    );
  }

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
    final ran = ranNext;
    if (ran.isEmpty) return applied;
    ranNext = const [];
    return BridgeApplied(
      action: applied.action,
      area: applied.area,
      page: applied.page,
      block: applied.block,
      view: applied.view,
      field: applied.field,
      row: applied.row,
      option: applied.option,
      tab: applied.tab,
      elements: applied.elements,
      line: applied.line,
      automation: applied.automation,
      ran: ran,
    );
  }

  final automations = <BridgeWorkspaceAutomation>[];

  List<BridgeRan> ranNext = const [];

  final pressed = <(String, String, String)>[];

  final tested = <String>[];

  @override
  Future<List<BridgeRan>> press({
    required String table,
    required String row,
    required String field,
  }) async {
    pressed.add((table, row, field));
    final ran = ranNext;
    ranNext = const [];
    return ran;
  }

  @override
  Future<BridgeRan> test({required String automation}) async {
    tested.add(automation);
    final ran = ranNext;
    ranNext = const [];
    return ran.firstOrNull ??
        BridgeRan(automation: automation, name: '', notices: const []);
  }

  final roles = <BridgeRole>[
    for (final (index, (name, template, allows)) in const [
      ('Can manage', BridgeTemplate.canManage, everything),
      (
        'Can edit',
        BridgeTemplate.canEdit,
        BridgeAllows(
          read: true,
          propose: false,
          apply: true,
          approve: false,
          manage: false,
        ),
      ),
      (
        'Can view',
        BridgeTemplate.canView,
        BridgeAllows(
          read: true,
          propose: false,
          apply: false,
          approve: false,
          manage: false,
        ),
      ),
    ].indexed)
      BridgeRole(
        id: '${index + 1}'.padLeft(32, '0'),
        template: template,
        setup: BridgeRoleSetup(
          name: name,
          grants: [
            BridgeGrant(kind: BridgeTargetKind.workspace, allows: allows),
          ],
        ),
      ),
  ];

  final allowances = <String, Map<String, BridgeAllows>>{};

  final entries = <BridgeEntry>[];

  final asking = <BridgeEntryQuery>[];

  @override
  Future<BridgeHistory> history({required BridgeEntryQuery asked}) async {
    asking.add(asked);
    final start = asked.before == null
        ? 0
        : entries.indexWhere((entry) => entry.actions.last == asked.before) + 1;
    final shown = entries
        .skip(start)
        .where((entry) => asked.page == null || entry.page == asked.page)
        .toList();
    return BridgeHistory(
      entries: shown.take(asked.limit.toInt()).toList(),
      more: shown.length > asked.limit.toInt(),
    );
  }

  final reverted = <List<String>>[];

  BridgeReverted revertOutcome = const BridgeReverted(
    action: 'reverted',
    conflict: false,
    purged: false,
  );

  @override
  Future<BridgeReverted> revert({required List<String> actions}) async {
    reverted.add(actions);
    return revertOutcome;
  }

  int undos = 0;
  int redos = 0;

  @override
  Future<bool> undo() async {
    undos++;
    return true;
  }

  @override
  Future<bool> redo() async {
    redos++;
    return true;
  }

  final versionsOf = <String, List<BridgeVersion>>{};

  @override
  Future<List<BridgeVersion>> versions({required String page}) async =>
      versionsOf[page] ?? const [];

  final restored = <(String, String)>[];

  @override
  Future<void> restore({required String page, required String to}) async {
    restored.add((page, to));
  }

  final purged = <(String?, String?)>[];

  @override
  Future<void> purge({String? page, String? table}) async {
    purged.add((page, table));
  }

  FakeWorkspace? version;

  final previewed = <(String, String)>[];

  @override
  Future<LiquidWorkspace> preview({
    required String page,
    required String to,
  }) async {
    previewed.add((page, to));
    return version ?? this;
  }

  void _rolesChanged() =>
      _changed(const BridgeResource(kind: BridgeResourceKind.roles));

  void _automationsChanged() =>
      _changed(const BridgeResource(kind: BridgeResourceKind.automations));

  void _replaceAutomation(
    String? id,
    BridgeWorkspaceAutomation Function(BridgeWorkspaceAutomation) change,
  ) {
    final at = automations.indexWhere((kept) => kept.id == id);
    if (at < 0) throw 'not found';
    automations[at] = change(automations[at]);
    _automationsChanged();
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
      case BridgeMutationKind.createAutomation:
        final id = _id();
        final setup = mutation.automationSetup!;
        automations.add(
          BridgeWorkspaceAutomation(
            id: id,
            setup: setup,
            on_: true,
            owner: setup.trigger?.table,
            role: canEdit,
          ),
        );
        _automationsChanged();
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          automation: id,
        );
      case BridgeMutationKind.setAutomation:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: mutation.automationSetup!,
            on_: kept.on_,
            last: kept.last,
            owner: mutation.automationSetup!.trigger?.table,
            role: kept.role,
          ),
        );
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.setAutomationOn:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: kept.setup,
            on_: mutation.on_,
            attention: kept.attention,
            last: kept.last,
            owner: kept.owner,
            role: kept.role,
          ),
        );
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.setAutomationRole:
        _replaceAutomation(
          mutation.automation,
          (kept) => BridgeWorkspaceAutomation(
            id: kept.id,
            setup: kept.setup,
            on_: kept.on_,
            attention: kept.attention,
            last: kept.last,
            owner: kept.owner,
            role: mutation.role!,
          ),
        );
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.createRole:
        final id = _id();
        roles.add(BridgeRole(id: id, setup: mutation.roleSetup!));
        _rolesChanged();
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          role: id,
        );
      case BridgeMutationKind.setRole:
        final at = roles.indexWhere((role) => role.id == mutation.role);
        roles[at] = BridgeRole(
          id: roles[at].id,
          setup: mutation.roleSetup!,
          template: roles[at].template,
        );
        _rolesChanged();
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.removeRole:
        roles.removeWhere((role) => role.id == mutation.role);
        _rolesChanged();
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.protectTable || BridgeMutationKind.unprotectTable:
        final at = tables.indexWhere((table) => table.id == mutation.block);
        tables[at] = BridgeTable(
          id: tables[at].id,
          area: tables[at].area,
          name: tables[at].name,
          protected: mutation.kind == BridgeMutationKind.protectTable,
        );
        _changed(const BridgeResource(kind: BridgeResourceKind.tables));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.removeAutomation:
        automations.removeWhere((kept) => kept.id == mutation.automation);
        _automationsChanged();
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.renameWorkspace:
        name = mutation.text!;
        _changed(const BridgeResource(kind: BridgeResourceKind.workspace));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.setWorkspaceIcon:
        icon = mutation.text!;
        _changed(const BridgeResource(kind: BridgeResourceKind.workspace));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.createArea:
        final area = BridgeArea(id: _id(), name: mutation.text!);
        areas.add(area);
        pages[area.id] = [];
        _changed(const BridgeResource(kind: BridgeResourceKind.areas));
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          area: area.id,
        );
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
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          page: page.id,
        );
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
            tabs: shown.tabs,
            tile: shown.tile,
            inner: shown.inner,
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
          ran: const [],
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.turnIntoCode ||
          BridgeMutationKind.turnIntoMath ||
          BridgeMutationKind.turnIntoText:
        final kind = _kindOf(mutation.page!, mutation.block!);
        final to = switch (mutation.kind) {
          BridgeMutationKind.turnIntoCode => BridgeBlockKind.code,
          BridgeMutationKind.turnIntoMath => BridgeBlockKind.math,
          _ => BridgeBlockKind.text,
        };
        final from = to == BridgeBlockKind.text ? kind : BridgeBlockKind.text;
        if (kind != from ||
            (to == BridgeBlockKind.text &&
                kind != BridgeBlockKind.code &&
                kind != BridgeBlockKind.math)) {
          throw 'not found';
        }
        final page = views[mutation.page]!;
        for (final (at, shown) in page.indexed) {
          if (shown.block != mutation.block) continue;
          page[at] = BridgeView(
            id: shown.id,
            block: shown.block,
            title: shown.title,
            boundary: shown.boundary,
            kind: to,
            tabs: shown.tabs,
            tile: shown.tile,
            inner: shown.inner,
          );
        }
        languages.remove(mutation.block);
        if (to == BridgeBlockKind.text) {
          styles[mutation.block!] = BridgeTextStyle.paragraph;
        }
        _viewsChanged(mutation.page);
        _changed(_blockResource(mutation));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.setTextStyle:
        styles[mutation.block!] = mutation.style;
        _changed(_blockResource(mutation));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
          tabs: shown.tabs,
          tile: shown.tile,
          inner: shown.inner,
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          title: shown.title,
          boundary: shown.boundary,
          kind: shown.kind,
          tabs: tabs,
          tile: shown.tile,
          inner: shown.inner,
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          tab: created,
        );
      case BridgeMutationKind.setTile || BridgeMutationKind.setInnerTile:
        final page = views[mutation.page]!;
        final at = page.indexWhere((view) => view.id == mutation.view);
        if (at < 0) throw 'not found';
        final shown = page[at];
        final inner = mutation.kind == BridgeMutationKind.setInnerTile;
        page[at] = BridgeView(
          id: shown.id,
          block: shown.block,
          title: shown.title,
          boundary: shown.boundary,
          kind: shown.kind,
          tabs: shown.tabs,
          tile: inner ? shown.tile : mutation.tile,
          inner: inner ? mutation.tile : shown.inner,
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.moveViews:
        final page = views[mutation.page]!;
        final from = page.indexWhere((view) => view.id == mutation.view);
        final to = page.indexWhere((view) => view.id == mutation.lastView);
        if (from < 0 || to < from) throw 'text position out of range';
        final run = page.sublist(from, to + 1);
        if (run.any((view) => view.id == mutation.after)) {
          throw 'text position out of range';
        }
        page.removeRange(from, to + 1);
        page.insertAll(
          mutation.after == null
              ? 0
              : page.indexWhere((view) => view.id == mutation.after) + 1,
          run,
        );
        _viewsChanged(mutation.page);
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.removeView:
        views[mutation.page]!.removeWhere((view) => view.id == mutation.view);
        _viewsChanged(mutation.page);
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          row: row.id,
        );
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.removeRow:
        final table = rows[mutation.block]!;
        final at = table.indexWhere((row) => row.id == mutation.row);
        if (at < 0) throw 'not found';
        table.removeAt(at);
        _changed(_tableResource(mutation.block));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        databases[mutation.block!] = BridgeDatabase(
          name: database.name,
          fields: fields,
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(
          ran: const [],
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
          computed: table[at].computed,
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
                      formula: '',
                    )
                  : field,
          ],
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          option: option.id,
        );
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
        databases[mutation.block!] = BridgeDatabase(
          name: database.name,
          fields: [
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
          ],
        );
        _changed(_tableResource(mutation.block));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
                  formula: field.formula,
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
                computed: row.computed,
              ),
          ];
        }
        _changed(_tableResource(mutation.block));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.renameDatabase:
        final database = databases[mutation.block]!;
        databases[mutation.block!] = BridgeDatabase(
          name: mutation.text!,
          fields: database.fields,
        );
        _renameTable(mutation.block!, mutation.text!);
        _changed(_tableResource(mutation.block));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
      case BridgeMutationKind.addCalculationBlock:
        final view = _newView(BridgeBlockKind.calculation);
        calculations[view.block] = [
          line(
            _id(),
            const BridgeLineSetup(
              label: '',
              formula: '',
              style: BridgeLineStyle.plain,
            ),
          ),
        ];
        return _placeView(mutation, view);
      case BridgeMutationKind.addChartBlock:
        final view = _newView(BridgeBlockKind.chart);
        charts[view.block] = const BridgeChartSetup(
          kind: BridgeChartKind.bar,
          value: BridgeChartValue.count,
        );
        return _placeView(mutation, view);
      case BridgeMutationKind.setChart:
        if (!charts.containsKey(mutation.block)) throw 'not found';
        charts[mutation.block!] = mutation.chart!;
        _changed(_blockResource(mutation));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.addLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final at = mutation.after == null
            ? 0
            : lines.indexWhere((line) => line.id == mutation.after) + 1;
        final id = _id();
        lines.insert(at, line(id, mutation.lineSetup!));
        _changed(_blockResource(mutation));
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          line: id,
        );
      case BridgeMutationKind.setLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final at = lines.indexWhere((line) => line.id == mutation.line);
        if (at < 0) throw 'not found';
        final formula = mutation.lineSetup!.formula;
        if (formula.trimRight().endsWith('+')) {
          throw "the formula can't be read from character ${formula.length + 1}";
        }
        lines[at] = line(mutation.line!, mutation.lineSetup!);
        _changed(_blockResource(mutation));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
      case BridgeMutationKind.removeLine:
        final lines = calculations[mutation.block] ?? (throw 'not found');
        final before = lines.length;
        lines.removeWhere((line) => line.id == mutation.line);
        if (lines.length == before) throw 'not found';
        _changed(_blockResource(mutation));
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(ran: const [], elements: ids, action: _id());
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
        return BridgeApplied(
          ran: const [],
          elements: const [],
          action: _id(),
          block: table,
        );
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
          tabs: shown.tabs,
          tile: shown.tile,
          inner: shown.inner,
        );
        _viewsChanged(mutation.page);
        if (shown.kind == BridgeBlockKind.database &&
            tables.any(
              (table) => table.id == shown.block && table.name.isEmpty,
            )) {
          _renameTable(shown.block, mutation.text!);
        }
        return BridgeApplied(ran: const [], elements: const [], action: _id());
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
        return BridgeApplied(ran: const [], elements: const [], action: _id());
    }
  }

  final tables = <BridgeTable>[];

  void _newTable(String id, String area, String name) {
    tables.add(BridgeTable(id: id, area: area, name: name, protected: false));
    _changed(const BridgeResource(kind: BridgeResourceKind.tables));
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
            tabs: view.tabs,
          ),
  ];

  BridgeResource _tableResource(String? block) =>
      BridgeResource(kind: BridgeResourceKind.table, block: block);

  BridgeView _newView(
    BridgeBlockKind kind, {
    bool boundary = false,
    String title = '',
    List<BridgeTab> tabs = const [],
  }) => BridgeView(
    id: _id(),
    block: _id(),
    title: title,
    boundary: boundary,
    kind: kind,
    tabs: tabs,
  );

  final device = <String, String>{};

  final recalled = <String>[];

  bool refuseRemembered = false;

  @override
  Future<String?> remembered({required String key}) async {
    recalled.add(key);
    if (refuseRemembered) throw 'refused';
    return device[key];
  }

  @override
  Future<void> remember({required String key, String? value}) async {
    if (value == null) {
      device.remove(key);
    } else {
      device[key] = value;
    }
  }

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
      ran: const [],
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

  BridgeView _decoded(BridgeView view) => BridgeView(
    id: view.id,
    block: view.block,
    title: view.title,
    boundary: view.boundary,
    kind: view.kind,
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
    tile: view.tile,
    inner: view.inner,
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
        BridgeMatch.isNot || BridgeMatch.above || BridgeMatch.below =>
          throw UnimplementedError('Automation conditions filter no View'),
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

const emptyCell = BridgeCell(
  kind: BridgeCellKind.empty,
  number: 0,
  checked: false,
  files: [],
);

BridgeCell checkboxCell(bool checked) => BridgeCell(
  kind: BridgeCellKind.checkbox,
  number: 0,
  checked: checked,
  files: const [],
);

BridgeCell selectCell(String option) => BridgeCell(
  kind: BridgeCellKind.select,
  text: option,
  number: 0,
  checked: false,
  files: const [],
);

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
