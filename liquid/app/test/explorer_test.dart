import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
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

  testWidgets('focus and hover show in the sidebar', (tester) async {
    await show(tester);
    final context = tester.element(find.byType(Sidebar));
    final theme = Theme.of(context);
    await tabTo(
      tester,
      find.ancestor(of: inSidebar('Untitled'), matching: find.byType(InkWell)),
    );
    expect(
      find.byType(Sidebar),
      paintsAbove(context.ws.selected, theme.focusColor),
      reason: 'the highlight is painted above the open Page\'s row',
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
