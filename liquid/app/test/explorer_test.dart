import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/theme/tokens.g.dart';
import 'package:liquid/src/workspace/page_view.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_backend.dart';
import 'fake_workspace.dart';
import 'focus_kit.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeWorkspace fake;
  late WorkspaceState workspace;

  Future<void> show(WidgetTester tester, {SessionState? session}) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    fake = FakeWorkspace();
    workspace = WorkspaceState(fake);
    await workspace.start(show: session == null);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: session,
          info: session == null ? null : info,
          workspace: workspace,
        ),
      ),
    );
    await tester.pumpAndSettle();
    addTearDown(workspace.dispose);
  }

  Finder title() => find.byKey(const Key('page-title'));
  List<String?> renames() => [
    for (final mutation in fake.applied)
      if (mutation.kind == BridgeMutationKind.renamePage) mutation.text,
  ];
  Finder inSidebar(String text) =>
      find.descendant(of: find.byType(Sidebar), matching: find.text(text));
  Finder header(String text) => find.descendant(
    of: find.byKey(const Key('page-header')),
    matching: find.text(text),
  );

  testWidgets('without an agent the first start opens a blank Page to name', (
    tester,
  ) async {
    await show(tester);
    expect(inSidebar('Private'), findsOneWidget);
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(inSidebar('Agent sessions'), findsOneWidget);
    expect(find.byType(ChatView), findsNothing);
    final field = tester.widget<TextField>(title());
    expect(field.focusNode!.hasFocus, isTrue);

    await tester.enterText(title(), 'Trip to Lisbon');
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    expect(fake.pages.values.single.single.title, 'Trip to Lisbon');
    expect(inSidebar('Trip to Lisbon'), findsOneWidget);
    expect(header('Trip to Lisbon'), findsOneWidget);
    expect(header('Private'), findsOneWidget);
  });

  testWidgets('a title stays one line and a refused rename is undone', (
    tester,
  ) async {
    await show(tester);
    await tester.enterText(title(), 'Plan\nnow');
    await tester.pumpAndSettle();
    expect(fake.pages.values.single.single.title, 'Plan now');

    fake.refuse = 'the replica is read-only';
    await tester.enterText(title(), 'Plan now!');
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(title()).controller!.text, 'Plan now');
    expect(find.text('the replica is read-only'), findsOneWidget);
  });

  testWidgets(
    '⌘Z after Liquid assigns a title does not bring back the title it replaced',
    (tester) async {
      await show(tester);
      await tester.enterText(title(), 'Trip');
      await tester.pumpAndSettle();
      await tester.pumpAndSettle();

      // A title Liquid assigns while the person is not typing in the field.
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pumpAndSettle();
      await workspace.rename(fake.pages.values.single.single.id, 'Porto');
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(title()).controller!.text, 'Porto');

      await tester.tap(title());
      await tester.pumpAndSettle();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(title()).controller!.text, 'Porto');
      expect(fake.pages.values.single.single.title, 'Porto');
    },
    variant: TargetPlatformVariant.only(TargetPlatform.macOS),
  );

  testWidgets('the area’s + and ⇧⌘N add Pages, and Pages nest', (tester) async {
    await show(tester);
    await tester.enterText(title(), 'Trip');
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('New Page'));
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(tester.widget<TextField>(title()).focusNode!.hasFocus, isTrue);
    await tester.enterText(title(), 'Budget');
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    final area = fake.areas.single.id;
    expect(fake.pages[area]!.map((page) => page.title), ['Trip', 'Budget', '']);

    // Notion's "Add a page inside", from the row's menu.
    await tester.tapAt(
      tester.getCenter(inSidebar('Trip')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a page inside'));
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    final trip = fake.pages[area]!.first.id;
    expect(fake.pages[area]![1].parent, trip);
    expect(workspace.open.value?.page, fake.pages[area]![1].id);
    expect(find.bySemanticsLabel('Collapse'), findsOneWidget);

    await tester.tap(find.bySemanticsLabel('Collapse'));
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(find.bySemanticsLabel('Expand'), findsOneWidget);
  });

  testWidgets('a Page and the agent session take turns beside the sidebar', (
    tester,
  ) async {
    await show(tester, session: SessionState(FakeBackend()));
    expect(find.byType(ChatView), findsOneWidget);
    expect(find.byType(WorkspacePage), findsNothing);

    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    expect(find.byType(WorkspacePage), findsOneWidget);
    expect(find.byType(ChatView), findsNothing);

    await tester.tap(find.byKey(const Key('session-card')));
    await tester.pumpAndSettle();
    expect(find.byType(ChatView), findsOneWidget);

    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(find.byType(ChatView), findsOneWidget);
  });

  testWidgets('a Workspace that did not open says why in the sidebar', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: SessionState(FakeBackend()),
          info: info,
          workspaceProblem: 'another Liquid session holds this replica',
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      inSidebar('another Liquid session holds this replica'),
      findsOneWidget,
    );
  });

  testWidgets('choosing a Page in the Explorer closes Settings', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(find.byType(WorkspacePage), findsNothing);
    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    expect(find.byType(WorkspacePage), findsOneWidget);
  });

  testWidgets('a question that came behind a Page waits once shown', (
    tester,
  ) async {
    final backend = FakeBackend();
    await show(tester, session: SessionState(backend));
    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    backend.controller.add(
      event(
        BridgeEventKind.userInputRequested,
        request: const BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.confirm,
            title: 'Run the tests?',
            detail: '',
            options: [],
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(seconds: 2));

    await tester.tap(find.byKey(const Key('session-card')));
    await tester.pump();
    await tester.tap(find.text('Yes'));
    await tester.pump();
    expect(backend.calls.where((call) => call.startsWith('confirm')), isEmpty);
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Yes'));
    expect(backend.calls.last, 'confirm q1 true');
  });

  testWidgets(
    'a Workspace problem shows in the sidebar while no Page is open',
    (tester) async {
      await show(tester, session: SessionState(FakeBackend()));
      fake.refuse = 'the replica is read-only';
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      expect(inSidebar('the replica is read-only'), findsOneWidget);
    },
  );

  testWidgets('a Page title is renamed once its composition ends', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(title());
    await tester.pump();
    fake.applied.clear();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'nihon',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      ),
    );
    await tester.pumpAndSettle();
    expect(renames(), isEmpty);
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: '日本',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.pumpAndSettle();
    expect(renames(), ['日本']);
  });

  testWidgets('quitting writes a composition still open in the title', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(title());
    await tester.pump();
    fake.applied.clear();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'nihon',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      ),
    );
    await tester.pump();
    var drained = false;
    unawaited(workspace.drain().then((_) => drained = true));
    await tester.pump();
    expect(drained, isTrue);
    expect(renames(), ['nihon']);
  });

  testWidgets('leaving the Page writes a composition still open in the title', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(title());
    await tester.pump();
    fake.applied.clear();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'nihon',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      ),
    );
    await tester.pump();
    await tester.tap(find.byTooltip('Settings'));
    await tester.pumpAndSettle();
    expect(renames(), ['nihon']);
  });

  testWidgets('a nested Page’s disclosure opens from the keyboard', (
    tester,
  ) async {
    await show(tester);
    await tester.enterText(find.byKey(const Key('page-title')), 'Trip');
    await tester.pumpAndSettle();
    await tester.tapAt(
      tester.getCenter(inSidebar('Trip')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a page inside'));
    await tester.pumpAndSettle();
    await tester.tap(find.bySemanticsLabel('Collapse'));
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsNothing);

    final disclosure = find.byKey(const Key('disclosure'));
    for (var press = 0; press < 30; press++) {
      final focused = FocusManager.instance.primaryFocus?.context;
      if (focused != null &&
          find
              .descendant(
                of: disclosure,
                matching: find.byElementPredicate((e) => e == focused),
              )
              .evaluate()
              .isNotEmpty) {
        break;
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
    }
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsOneWidget);
  });

  testWidgets('an older refused rename never undoes a newer one', (
    tester,
  ) async {
    await show(tester);
    fake.gate = Completer();
    await tester.enterText(title(), 'P');
    await tester.enterText(title(), 'Pl');
    fake.refuseNext = 1;
    fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(fake.pages.values.single.single.title, 'Pl');
    expect(tester.widget<TextField>(title()).controller!.text, 'Pl');
  });

  testWidgets('a refused rename shows the stored title, not an older one', (
    tester,
  ) async {
    await show(tester);
    // The tree is not read again yet, so it still holds the title before 'P'.
    fake.deferredKinds = {BridgeQueryKind.pages};
    fake.deferred = [];
    await tester.enterText(title(), 'P');
    await tester.pumpAndSettle();
    fake.refuseNext = 1;
    await tester.enterText(title(), 'Pl');
    await tester.pumpAndSettle();
    fake.deferredKinds = {};
    for (final release in [...fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(fake.pages.values.single.single.title, 'P');
    expect(tester.widget<TextField>(title()).controller!.text, 'P');
  });

  testWidgets('focus and hover show in the sidebar', (tester) async {
    await show(tester);
    final context = tester.element(find.byType(Sidebar));
    final theme = Theme.of(context);
    await tabTo(
      tester,
      find.ancestor(of: inSidebar('Untitled'), matching: find.byType(InkWell)),
    );
    expect(
      find.descendant(
        of: find.byType(Sidebar),
        matching: find.byType(ListView),
      ),
      paintsAbove(context.ws.selected, theme.focusColor),
      reason:
          'the highlight is painted above the open Page\'s row, in the '
          'list that clips it',
    );

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(inSidebar('Settings')));
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
    expect(
      find.byType(Sidebar),
      paintsAbove(context.ws.subtle, theme.hoverColor),
      reason: 'the hover is painted above the sidebar',
    );
  });

  testWidgets(
    'a hovered Page row shows its ··· inside the row, at its right end',
    (tester) async {
      await show(tester);
      final row = tester.getRect(
        find
            .ancestor(of: inSidebar('Untitled'), matching: find.byType(InkWell))
            .first,
      );
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: tester.getCenter(inSidebar('Untitled')));
      addTearDown(mouse.removePointer);
      await tester.pumpAndSettle();

      final more = tester.getRect(
        find.descendant(
          of: find.byType(Sidebar),
          matching: find.byTooltip('More actions'),
        ),
      );
      expect(
        row.contains(more.center),
        isTrue,
        reason: 'the highlight covers the ···',
      );
      expect(
        row.right - more.right,
        lessThan(WsMetrics.space5),
        reason: 'at the right end',
      );
    },
  );

  testWidgets(
    'a hovered session card shows its ··· in place of the status word',
    (tester) async {
      await show(tester, session: SessionState(FakeBackend()));
      final card = find.byKey(const Key('session-card'));
      final status = find.descendant(of: card, matching: find.text('idle'));
      final more = find.descendant(
        of: card,
        matching: find.byTooltip('More actions'),
      );
      // How visible `finder` is: its nearest fading ancestor's opacity, 1 when none fades it.
      double opacity(Finder finder) {
        final fades = find.ancestor(of: finder, matching: find.byType(Opacity));
        return fades.evaluate().isEmpty
            ? 1
            : tester.widget<Opacity>(fades.first).opacity;
      }

      expect(opacity(status), 1);
      expect(opacity(more), 0);

      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: tester.getCenter(card));
      addTearDown(mouse.removePointer);
      await tester.pumpAndSettle();
      expect(opacity(status), 0);
      expect(opacity(more), 1);
    },
  );

  testWidgets(
    'Tab reaches a Page row’s ···, ⏎ there opens its menu and esc closes it',
    (tester) async {
      await show(tester);
      await tabTo(
        tester,
        find.ancestor(
          of: inSidebar('Untitled'),
          matching: find.byType(InkWell),
        ),
      );
      await tabTo(
        tester,
        find.descendant(
          of: find.byType(Sidebar),
          matching: find.byTooltip('More actions'),
        ),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(find.text('Add a page inside'), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.text('Add a page inside'), findsNothing);
    },
  );

  testWidgets('every Tab stop around the sidebar shows where it is', (
    tester,
  ) async {
    await show(tester);
    for (var press = 0; press < 8; press++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pumpAndSettle();
      expect(
        FocusManager.instance.primaryFocus?.debugLabel,
        isNot('shell'),
        reason: 'the whole window is no Tab stop',
      );
    }
  });
}
