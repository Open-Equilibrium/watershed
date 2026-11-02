import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/workspace/page_views.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_workspace.dart';

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
    expect(state.problem.value, 'the replica is read-only');
    fake.refuse = null;
    await state.rename(page, 'B');
    expect(state.problem.value, isNull);
    expect(fake.pages.values.single.single.title, 'B');
    await state.dispose();
  });

  test('disposing closes every subscription', () async {
    final fake = FakeWorkspace();
    final state = WorkspaceState(fake);
    await state.start();
    await pumpEventQueue();
    expect(fake.followed, {
      const BridgeResource(kind: BridgeResourceKind.areas),
      BridgeResource(
        kind: BridgeResourceKind.pages,
        area: fake.areas.single.id,
      ),
    });
    await state.dispose();
    expect(fake.followed, isEmpty);
  });

  test('a Workspace whose first read fails says so', () async {
    final fake = FakeWorkspace()..refuseQueries = 'disk I/O error';
    final (state, problem) = await startWorkspace(fake, show: true);
    expect(problem, 'The Workspace did not load: disk I/O error');
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
      String? after;
      for (var index = 0; index < 150; index++) {
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

    test(
      'read a window again when the Page changed while it was read',
      () async {
        fake.deferredKinds = {BridgeQueryKind.views};
        fake.deferred = [];
        expect(views.at(120), isNull);
        await pumpEventQueue();
        final window = fake.deferred!.single;
        // A change while the window is on its way.
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
