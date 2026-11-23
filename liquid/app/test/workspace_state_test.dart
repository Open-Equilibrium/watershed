import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/workspace/page_views.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

Future<void> addBlocks(WorkspaceState state, String page, int count) async {
  String? after;
  for (var index = 0; index < count; index++) {
    final added = await state.apply(
      mutation(
        BridgeMutationKind.addTextBlock,
        page: page,
        after: after,
        text: '$index',
      ),
    );
    after = added.view;
  }
}

void main() {
  test(
    'a new Workspace starts with the area Private and one blank Page, open',
    () async {
      final fake = FakeWorkspace();
      final state = WorkspaceState(fake);
      await state.start();
      await pumpEventQueue();

      expect(state.areas.value.map((area) => area.name), ['Private']);
      final private = state.areas.value.single.id;
      final page = state.pages(private).value.single;
      expect(page.title, '');
      expect(state.open.value?.page, page.id);
      expect(state.open.value?.fresh, isTrue);
      await state.dispose();
    },
  );

  test('a Workspace with Pages opens its first one', () async {
    final fake = FakeWorkspace();
    final first = WorkspaceState(fake);
    await first.start();
    await first.dispose();

    final state = WorkspaceState(fake);
    await state.start();
    await pumpEventQueue();
    expect(fake.areas, hasLength(1));
    expect(state.open.value?.page, fake.pages.values.single.single.id);
    expect(state.open.value?.fresh, isFalse);
    await state.dispose();
  });

  test('a renamed or nested Page shows in its area’s tree', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    final area = fake.areas.single.id;
    final trip = state.open.value!.page;
    await state.rename(trip, 'Trip to Lisbon');
    await state.newPage(area, parent: trip);
    await pumpEventQueue();

    final tree = state.pages(area).value;
    expect(tree.map((page) => page.title), ['Trip to Lisbon', '']);
    expect(tree.last.parent, trip);
    expect(state.open.value?.page, tree.last.id);
    await state.dispose();
  });

  test('writes reach the Workspace in the order asked', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    final page = state.open.value!.page;
    fake.gate = Completer();
    final writes = [
      for (final title in ['L', 'Li', 'Lis']) state.rename(page, title),
    ];
    fake.gate!.complete();
    await Future.wait(writes);
    expect(fake.applied.skip(2).map((change) => change.text), [
      'L',
      'Li',
      'Lis',
    ]);
    await state.dispose();
  });

  test('a refused write is reported and the next one still runs', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    final page = state.open.value!.page;
    fake.refuse = 'the replica is read-only';
    await expectLater(state.rename(page, 'A'), throwsA(isA<String>()));
    expect(
      state.problem.value,
      'Liquid did not save this change: the replica is read-only',
    );
    fake.refuse = null;
    await state.rename(page, 'B');
    expect(state.problem.value, isNull);
    expect(fake.pages.values.single.single.title, 'B');
    await state.dispose();
  });

  test(
    'draining waits for every write asked for, also those asked meanwhile',
    () async {
      final fake = FakeWorkspace();
      final state = WorkspaceState(fake);
      await state.start();
      final page = state.open.value!.page;
      fake.gate = Completer();
      for (final title in ['L', 'Li', 'Lis']) {
        unawaited(state.rename(page, title));
      }
      var drained = false;
      unawaited(state.drain().then((_) => drained = true));
      await pumpEventQueue();
      expect(drained, isFalse);
      unawaited(state.rename(page, 'Lisbon'));
      fake.gate!.complete();
      await pumpEventQueue();
      expect(drained, isTrue);
      expect(fake.pages.values.single.single.title, 'Lisbon');
      await state.dispose();
    },
  );

  test('disposing closes every subscription', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    await pumpEventQueue();
    expect(fake.followed, {
      const BridgeResource(kind: BridgeResourceKind.areas),
      const BridgeResource(kind: BridgeResourceKind.tables),
      const BridgeResource(kind: BridgeResourceKind.automations),
      const BridgeResource(kind: BridgeResourceKind.roles),
      const BridgeResource(kind: BridgeResourceKind.workspace),
      BridgeResource(
        kind: BridgeResourceKind.pages,
        area: fake.areas.single.id,
      ),
    });
    await state.dispose();
    expect(fake.followed, isEmpty);
  });

  test('a failed read shows until a read succeeds, a refused change until a change does', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    final page = state.open.value!.page;
    final pages = query(
      BridgeQueryKind.pages,
      area: state.areas.value.single.id,
    );

    fake.refuseQueries = 'disk I/O error';
    await state.rename(page, 'A');
    await pumpEventQueue();
    expect(
      state.problem.value,
      'Liquid could not read the Workspace, so what shows may be out of date: '
      'disk I/O error',
    );
    fake.refuseQueries = null;
    await state.read(pages);
    expect(state.problem.value, isNull);

    state.readFailed('disk I/O error');
    fake.refuse = 'the replica is read-only';
    await expectLater(state.rename(page, 'B'), throwsA(isA<String>()));
    expect(
      state.problem.value,
      'Liquid did not save this change: the replica is read-only\n'
      'Liquid could not read the Workspace, so what shows may be out of date: '
      'disk I/O error',
    );
    await state.read(pages);
    expect(
      state.problem.value,
      'Liquid did not save this change: the replica is read-only',
      reason: 'a read does not say the change was kept',
    );
    await state.dispose();
  });

  test('a Workspace whose first read fails says so', () async {
    final fake = FakeWorkspace()..refuseQueries = 'disk I/O error';
    final (state, problem) = await startWorkspace(fake, show: true);
    expect(problem, 'The Workspace did not load: disk I/O error');
    await state.dispose();
  });

  test('a long Page rereads only the windows of Views it last used', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    final page = state.open.value!.page;
    await addBlocks(state, page, 2000);
    final views = PageViews(state, page)..start();
    await pumpEventQueue();
    for (var index = 0; index < 2000; index += 100) {
      expect(await views.viewAt(index), isNotNull);
    }
    fake.asked.clear();
    await state.apply(
      mutation(BridgeMutationKind.addTextBlock, page: page, text: 'new'),
    );
    await pumpEventQueue();
    final reads = fake.asked.where(
      (asked) => asked.kind == BridgeQueryKind.views,
    );
    expect(
      reads.length,
      lessThanOrEqualTo(11),
      reason: 'the windows kept, plus the first',
    );
    expect(
      views.at(1999)?.id,
      isNotNull,
      reason: 'the window last used is kept',
    );
    views.dispose();
    await state.dispose();
  });

  group('a Page’s Views', () {
    late FakeWorkspace fake;
    late WorkspaceState state;
    late String page;
    late PageViews views;

    setUp(() async {
      fake = FakeWorkspace();
      state = WorkspaceState(fake);
      await state.start();
      page = state.open.value!.page;
      await addBlocks(state, page, 150);
      views = PageViews(state, page)..start();
      await pumpEventQueue();
    });

    tearDown(() async {
      views.dispose();
      await state.dispose();
    });

    test('have nothing before the first or after the last', () async {
      expect(views.count, 150);
      expect(views.at(0), isNotNull);
      expect(views.at(-1), isNull);
      expect(views.at(150), isNull);
    });

    test('never show a window read beside an older one', () async {
      final shown = <List<String?>>[];
      views.addListener(
        () => shown.add([
          for (var index = 95; index < 105; index++) views.at(index)?.id,
        ]),
      );
      fake.deferredKinds = {BridgeQueryKind.views};
      fake.deferred = [];
      await state.apply(
        mutation(BridgeMutationKind.addTextBlock, page: page, text: 'new'),
      );
      await pumpEventQueue();
      expect(views.at(120), isNull);
      await pumpEventQueue();
      while (fake.deferred!.isNotEmpty) {
        fake.deferred!.removeLast().complete();
        await pumpEventQueue();
      }
      await pumpEventQueue();
      for (final ids in shown) {
        final present = ids.whereType<String>().toList();
        expect(present.toSet(), hasLength(present.length), reason: '$ids');
      }
      expect(views.indexOf(fake.views[page]![120].id), 120);
    });

    test('a numbering read never undoes a newer style', () async {
      final block = fake.views[page]![1].block;
      fake.deferredKinds = {BridgeQueryKind.text};
      fake.deferred = [];
      fake.styles[block] = BridgeTextStyle.numbered;
      final run = views.readRun(2);
      await pumpEventQueue();
      views.setStyle(block, BridgeTextStyle.paragraph);
      for (final release in fake.deferred!) {
        release.complete();
      }
      fake.deferred = null;
      await run;
      expect(views.styles.value[block], BridgeTextStyle.paragraph);
    });

    test('never show windows read across a change of order', () async {
      expect(views.at(120), isNull);
      await pumpEventQueue();
      final shown = <List<String?>>[];
      views.addListener(
        () => shown.add([
          for (var index = 95; index < 105; index++) views.at(index)?.id,
        ]),
      );
      fake.deferredKinds = {BridgeQueryKind.views};
      fake.deferred = [];
      Future<void> addAt(int index) => state.apply(
        mutation(
          BridgeMutationKind.addTextBlock,
          page: page,
          after: fake.views[page]![index].id,
          text: 'new',
        ),
      );
      await addAt(98);
      await pumpEventQueue();
      await addAt(98);
      await pumpEventQueue();
      while (fake.deferred!.isNotEmpty) {
        fake.deferred!.removeAt(0).complete();
        await pumpEventQueue();
      }
      for (final ids in shown) {
        final present = ids.whereType<String>().toList();
        expect(present.toSet(), hasLength(present.length), reason: '$ids');
      }
    });

    test(
      'read a window again when the Page changed while it was read',
      () async {
        fake.deferredKinds = {BridgeQueryKind.views};
        fake.deferred = [];
        expect(views.at(120), isNull);
        await pumpEventQueue();
        final window = fake.deferred!.single;
        await state.apply(
          mutation(BridgeMutationKind.addTextBlock, page: page, text: 'new'),
        );
        await pumpEventQueue();
        final reread = [...fake.deferred!.skip(1)];
        fake.deferred = null;
        for (final release in reread) {
          release.complete();
        }
        await pumpEventQueue();
        window.complete();
        await pumpEventQueue();
        expect(views.indexOf(fake.views[page]![120].id), 120);
      },
    );
  });
}
