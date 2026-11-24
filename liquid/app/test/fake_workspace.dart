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
import 'package:liquid/src/rust/api/source.dart';
import 'package:liquid/src/rust/api/drawing.dart';
import 'package:liquid/src/rust/api/workspace.dart';

part 'fake_workspace_blocks.dart';
part 'fake_workspace_pages.dart';
part 'fake_workspace_queries.dart';
part 'fake_workspace_roles.dart';
part 'fake_workspace_tables.dart';
part 'fake_workspace_text.dart';

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

BridgeRole _templateRole(
  String id,
  String name,
  BridgeTemplate template,
  BridgeAllows allows,
) => BridgeRole(
  id: id,
  template: template,
  setup: BridgeRoleSetup(
    name: name,
    grants: [BridgeGrant(kind: BridgeTargetKind.workspace, allows: allows)],
  ),
);

BridgeAllows _allowing({bool apply = false}) => BridgeAllows(
  read: true,
  propose: false,
  apply: apply,
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
  final tables = <BridgeTable>[];
  final whiteboards = <String, BridgeDrawing>{};
  final calculations = <String, List<BridgeLine>>{};
  final charts = <String, BridgeChartSetup>{};
  final chartPoints = <String, List<BridgeChartPoint>>{};
  final computed = <String, String>{};
  final referenceNames = <String, String>{};

  String? chartSource(BridgeChartSetup setup) => switch (setup) {
    BridgeChartSetup(:final table?) => databases[table]?.name,
    BridgeChartSetup(:final page?, :final block?) =>
      views[page]?.where((view) => view.block == block).firstOrNull?.title,
    BridgeChartSetup(:final source?) => [
      for (final listed in sourceList.values) ...listed,
    ].where((known) => known.id == source).firstOrNull?.name,
    _ => null,
  };

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
    return applied;
  }

  final automations = <BridgeWorkspaceAutomation>[];
  List<BridgeRan> ranNext = const [];
  final pressed = <(String, String, String)>[];
  final tested = <String>[];

  List<BridgeRan> _takeRan() {
    final ran = ranNext;
    ranNext = const [];
    return ran;
  }

  @override
  Future<List<BridgeRan>> press({
    required String table,
    required String row,
    required String field,
  }) async {
    pressed.add((table, row, field));
    return _takeRan();
  }

  @override
  Future<BridgeRan> test({required String automation}) async {
    tested.add(automation);
    return _takeRan().firstOrNull ??
        BridgeRan(automation: automation, name: '', notices: const []);
  }

  final roles = <BridgeRole>[
    _templateRole(
      canManage,
      'Can manage',
      BridgeTemplate.canManage,
      everything,
    ),
    _templateRole(
      canEdit,
      'Can edit',
      BridgeTemplate.canEdit,
      _allowing(apply: true),
    ),
    _templateRole(canView, 'Can view', BridgeTemplate.canView, _allowing()),
  ];
  final allowances = <String, Map<String, BridgeAllows>>{};
  final entries = <BridgeEntry>[];

  @override
  Future<BridgeHistory> history({required BridgeEntryQuery asked}) async {
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
  Future<void> restore({required String page, required String to}) async =>
      restored.add((page, to));

  final purged = <(String?, String?)>[];

  @override
  Future<void> purge({String? page, String? table}) async =>
      purged.add((page, table));

  final versionAt = <String, FakeWorkspace>{};
  final previewed = <(String, String)>[];

  @override
  Future<LiquidWorkspace> preview({
    required String page,
    required String to,
  }) async {
    previewed.add((page, to));
    await Future<void>.delayed(const Duration(milliseconds: 250));
    return versionAt[to] ?? this;
  }

  Completer<void>? answered;

  BridgeApplied _done({
    String? area,
    String? page,
    String? block,
    String? view,
    String? field,
    String? row,
    String? option,
    String? tab,
    List<String> elements = const [],
    String? line,
    String? automation,
    String? role,
  }) => BridgeApplied(
    action: _id(),
    area: area,
    page: page,
    block: block,
    view: view,
    field: field,
    row: row,
    option: option,
    tab: tab,
    elements: elements,
    line: line,
    automation: automation,
    role: role,
    ran: _takeRan(),
  );

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
    return _rolesMutation(mutation) ??
        _pagesMutation(mutation) ??
        _textMutation(mutation) ??
        _tablesMutation(mutation) ??
        _blocksMutation(mutation)!;
  }

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

  List<String> textsOf(String page) => [
    for (final view in views[page] ?? const <BridgeView>[])
      if (view.kind == BridgeBlockKind.text) texts[view.block]!,
  ];

  void dropTable(String table) {
    tables.removeWhere((shown) => shown.id == table);
    databases.remove(table);
    _tablesChanged();
    _changed(_tableResource(table));
  }

  void putRows(String page, String block, List<BridgeRow> rows) {
    this.rows[block] = rows;
    _changed(_tableResource(block));
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

  final sourceList = <String, List<BridgeSource>>{};
  final sourcePaths = <String, String>{};
  final sourceFiles = <(String, String), BridgeSourceFile>{};
  final sourceFolders = <(String, String), List<BridgeFileEntry>>{};
  final located = <(String, String)>[];

  static BridgeSourceFile missingFile = BridgeSourceFile(
    read: BridgeRead.missing,
    name: '',
    path: '',
    size: BigInt.zero,
    shown: BridgeFileView.other,
    columns: const [],
    rows: const [],
  );

  String? _areaOfSource(String source) {
    for (final MapEntry(:key, :value) in sourceList.entries) {
      if (value.any((known) => known.id == source)) return key;
    }
    return null;
  }

  void _sourcesChanged(String area) =>
      _changed(BridgeResource(kind: BridgeResourceKind.pages, area: area));

  Duration sourcesDelay = Duration.zero;

  @override
  Future<List<BridgeSource>> sources({required String area}) async {
    if (sourcesDelay > Duration.zero) await Future<void>.delayed(sourcesDelay);
    return sourceList[area] ?? const [];
  }

  @override
  Future<String?> sourcePath({required String sourceId}) async =>
      sourcePaths[sourceId];

  @override
  Future<String> addSource({
    required String area,
    String? parent,
    required String path,
  }) async {
    final id = _id();
    final folder = sourceFolders.containsKey((path, ''));
    sourceList[area] = [
      ...?sourceList[area],
      BridgeSource(
        id: id,
        parent: parent,
        name: path.split('/').last,
        kind: folder ? BridgeSourceKind.folder : BridgeSourceKind.file,
      ),
    ];
    sourcePaths[id] = path;
    if (sourceFiles[(path, '')] case final file?) sourceFiles[(id, '')] = file;
    for (final MapEntry(:key, :value) in [...sourceFolders.entries]) {
      if (key.$1 == path) sourceFolders[(id, key.$2)] = value;
    }
    for (final MapEntry(:key, :value) in [...sourceFiles.entries]) {
      if (key.$1 == path) sourceFiles[(id, key.$2)] = value;
    }
    _sourcesChanged(area);
    return id;
  }

  @override
  Future<void> renameSource({
    required String sourceId,
    required String name,
  }) async {
    final area = _areaOfSource(sourceId)!;
    sourceList[area] = [
      for (final known in sourceList[area]!)
        known.id == sourceId
            ? BridgeSource(
                id: known.id,
                parent: known.parent,
                name: name,
                kind: known.kind,
              )
            : known,
    ];
    _sourcesChanged(area);
  }

  @override
  Future<void> removeSource({required String sourceId}) async {
    final area = _areaOfSource(sourceId)!;
    sourceList[area] = [
      for (final known in sourceList[area]!)
        if (known.id != sourceId) known,
    ];
    _sourcesChanged(area);
  }

  @override
  Future<void> locateSource({
    required String sourceId,
    required String path,
  }) async {
    located.add((sourceId, path));
    sourcePaths[sourceId] = path;
  }

  @override
  Future<BridgeFolder> sourceFolder({
    required String sourceId,
    required String inside,
  }) async => switch (sourceFolders[(sourceId, inside)]) {
    final entries? => BridgeFolder(read: BridgeRead.read, entries: entries),
    null => const BridgeFolder(read: BridgeRead.missing, entries: []),
  };

  @override
  Future<BridgeSourceFile> sourceFile({
    required String sourceId,
    required String inside,
  }) async => sourceFiles[(sourceId, inside)] ?? missingFile;

  @override
  Future<List<BridgeSourceUser>> sourceUsers({
    required String sourceId,
  }) async => const [];

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
    _live();
    final controller = StreamController<BridgeChange>();
    _followers[subscription] = (resource, controller);
    controller.add(BridgeChange(resource: resource, seq: _seqs[resource] ?? 0));
    return controller.stream;
  }

  @override
  Future<void> close({required int subscription}) async {
    _live();
    await _followers.remove(subscription)?.$2.close();
  }

  void _live() {
    if (isDisposed) throw StateError('RustArc used after disposed');
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
