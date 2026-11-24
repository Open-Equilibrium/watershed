import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/workspace/page_icon.dart';

import 'fake_workspace.dart';
import 'page_kit.dart';

const you = BridgeActor(kind: BridgeActorKind.person, id: 'benni');

BridgeEntry entry(
  String id,
  String title, {
  String? page,
  bool revertable = true,
  bool undone = false,
  bool purged = false,
  String? deletedTable,
}) => BridgeEntry(
  actions: [id],
  at: DateTime.now().millisecondsSinceEpoch,
  actor: you,
  origin: BridgeOrigin.app,
  allowedBy: 'Can manage',
  title: title,
  page: page,
  undone: undone,
  revertable: revertable,
  purged: purged,
  deletedTable: deletedTable,
);

Future<void> tap(WidgetTester tester, Finder finder) async {
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

Finder inHistory(Finder finder) =>
    find.descendant(of: find.byKey(const Key('history')), matching: finder);

class _Watch extends FakeWorkspace {
  _Watch(this.onDispose);

  final void Function()? onDispose;

  @override
  void dispose() {
    onDispose?.call();
    super.dispose();
  }
}

void main() {
  testWidgets('History lists the Workspace\'s entries by day and reverts '
      'one after asking', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    kit.fake.entries
      ..add(entry('a2', 'Changed a cell of “Expenses”', page: kit.page))
      ..add(entry('a1', 'Edited text on “Budget”', undone: true))
      ..add(
        entry(
          'a0',
          'Edited text · content deleted for good',
          revertable: false,
          purged: true,
        ),
      );
    await tap(tester, find.byKey(const Key('sidebar history')));
    expect(inHistory(find.text('Today')), findsOneWidget);
    expect(inHistory(find.text('Changed a cell of “Expenses”')), findsWidgets);
    expect(inHistory(find.text('Undone')), findsOneWidget);

    await tap(
      tester,
      inHistory(find.text('Edited text · content deleted for good')),
    );
    expect(
      find.textContaining('Its content was deleted for good'),
      findsOneWidget,
    );
    await tap(
      tester,
      inHistory(find.text('Changed a cell of “Expenses”')).first,
    );
    await tap(tester, find.text('Revert…'));
    expect(find.text('Revert “Changed a cell of “Expenses””?'), findsOneWidget);
    await tap(tester, find.text('Revert'));
    expect(kit.fake.reverted.single, ['a2']);

    kit.fake.revertOutcome = const BridgeReverted(
      conflict: true,
      purged: false,
    );
    await tap(tester, find.text('Revert…'));
    await tap(tester, find.text('Revert'));
    expect(find.textContaining('cannot be reverted cleanly'), findsOneWidget);
    await tap(tester, find.text('Restore version…'));
    expect(find.byKey(const Key('page history')), findsOneWidget);
  });

  testWidgets('⌘Z undoes your last change and ⇧⌘Z redoes it, also while '
      'typing', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    await tap(tester, block(0));
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect((kit.fake.undos, kit.fake.redos), (1, 1));
  });

  testWidgets('⌘Z in a field whose typing is not yet a change undoes the '
      'typing, not History', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TablePart([])]);
    await tap(tester, find.byKey(const Key('table search')));
    final search = find.byKey(const Key('table search field'));
    await tester.enterText(search, 'r');
    await tester.pump(const Duration(milliseconds: 600));
    await tester.enterText(search, 'rent');
    await tester.pump(const Duration(milliseconds: 600));
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(kit.fake.undos, 0);
    expect(tester.widget<TextField>(search).controller!.text, 'r');
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('Page history shows a version read-only, restores it and '
      'deletes the earlier versions after asking', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    final now = DateTime.now().millisecondsSinceEpoch;
    kit.fake.versionsOf[kit.page] = [
      BridgeVersion(after: 'v2', at: now, actor: you, savePoint: false),
      BridgeVersion(after: 'v1', at: now - 600000, actor: you, savePoint: true),
    ];
    kit.fake.versionAt['v1'] = FakeWorkspace()
      ..areas.addAll(kit.fake.areas)
      ..pages.addAll(kit.fake.pages)
      ..views.addAll(kit.fake.views)
      ..texts.addAll({for (final block in kit.fake.texts.keys) block: 'May'});
    Finder shown(String text) => find.descendant(
      of: find.byKey(const Key('page history')),
      matching: find.text(text),
    );
    await tap(tester, find.byTooltip('Page history'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('page history')), findsOneWidget);
    expect(find.text('Save point'), findsOneWidget);
    expect(shown('October'), findsOneWidget);

    await tap(tester, find.text('Save point'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expect(kit.fake.previewed.last, (kit.page, 'v1'));
    expect(shown('October'), findsNothing);
    expect(shown('May'), findsOneWidget);
    expect(find.textContaining('read-only'), findsOneWidget);
    expect(find.byTooltip('Page history'), findsOneWidget);
    await tap(tester, find.text('Restore version'));
    expect(kit.fake.restored.single, (kit.page, 'v1'));

    await tap(tester, find.byTooltip('Page history actions'));
    await tap(tester, find.text('Delete earlier versions…'));
    expect(
      find.text('Delete the earlier versions of “Untitled”?'),
      findsOneWidget,
    );
    await tap(tester, find.text('Delete earlier versions'));
    expect(kit.fake.purged.single, (kit.page, null));
  });

  testWidgets('picking another Page history version disposes the old preview '
      'only after its Page has left the screen', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    final now = DateTime.now().millisecondsSinceEpoch;
    kit.fake.versionsOf[kit.page] = [
      BridgeVersion(after: 'v2', at: now, actor: you, savePoint: false),
      BridgeVersion(after: 'v1', at: now - 600000, actor: you, savePoint: true),
    ];
    final juneText = find.descendant(
      of: find.byKey(const Key('page history')),
      matching: find.text('June'),
    );
    bool? juneShown;
    FakeWorkspace copy(String text, [void Function()? onDispose]) =>
        _Watch(onDispose)
          ..areas.addAll(kit.fake.areas)
          ..pages.addAll(kit.fake.pages)
          ..views.addAll(kit.fake.views)
          ..texts.addAll({
            for (final block in kit.fake.texts.keys) block: text,
          });
    kit.fake.versionAt['v2'] = copy(
      'June',
      () => juneShown = juneText.evaluate().isNotEmpty,
    );
    kit.fake.versionAt['v1'] = copy('May');
    await tap(tester, find.byTooltip('Page history'));
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();

    await tester.runAsync(() => tester.tap(find.text('Save point')));
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 600)),
    );
    await tester.pumpAndSettle();
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 600)),
    );
    expect(juneShown, isFalse);
  });

  testWidgets('a deleted Table\'s entry deletes it for good after asking, and '
      'a Table\'s menu deletes its earlier versions', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TablePart([])]);
    final table = kit.fake.databases.keys.single;
    kit.fake.entries.add(
      entry('d1', 'Deleted the Table “Bills”', deletedTable: 'bills'),
    );
    await tap(tester, find.byKey(const Key('sidebar history')));
    await tap(tester, inHistory(find.text('Deleted the Table “Bills”')).first);
    await tap(tester, find.text('Delete for good…'));
    await tap(tester, find.text('Delete for good'));
    expect(kit.fake.purged.single, (null, 'bills'));

    await tap(tester, find.byTooltip('Close'));
    kit.workspace.openTable(table);
    await tester.pumpAndSettle();
    await tap(tester, find.byTooltip('Table actions'));
    await tap(tester, find.text('Delete earlier versions…'));
    await tap(tester, find.text('Delete earlier versions'));
    expect(kit.fake.purged.last, (null, table));
  });

  testWidgets('the Page filter lists each Page with its icon', (tester) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('October')]);
    await kit.workspace.setPageIcon(kit.page, '💰');
    await tester.pumpAndSettle();
    await tap(tester, find.byKey(const Key('sidebar history')));
    await tap(tester, find.byKey(const Key('filter page')));
    expect(
      find.descendant(
        of: find.byType(PopupMenuItem<VoidCallback>),
        matching: find.byType(PageIcon),
      ),
      findsOneWidget,
    );
  });
}
