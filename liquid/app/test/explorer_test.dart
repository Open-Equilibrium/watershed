import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/controls.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/page_view.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';
import 'page_kit.dart';

const info = SessionInfo(project: 'acme-api');
const readOnly = 'the replica is read-only';
const refusal = 'Liquid did not save this change: the replica is read-only';

void main() {
  final kit = PageKit();

  Future<void> show(WidgetTester tester, {SessionState? session}) async {
    if (session != null) {
      await session.loadStart();
      await session.open('thread-1');
    }
    await kit.show(
      tester,
      const [],
      fresh: true,
      session: session,
      info: session == null ? null : info,
    );
  }

  SessionState agent() => SessionState(
    FakeProject()..threadList = [threadOf('thread-1', title: 'Fix login')],
  );

  Finder title() => find.byKey(const Key('page-title'));
  String shownTitle(WidgetTester tester) =>
      tester.widget<TextField>(title()).controller!.text;
  String storedTitle() => kit.fake.pages.values.single.single.title;
  String pageId() => kit.fake.pages.values.single.single.id;
  List<String?> renames() => [
    for (final mutation in kit.fake.applied)
      if (mutation.kind == BridgeMutationKind.renamePage) mutation.text,
  ];
  Finder inSidebar(String text) =>
      find.descendant(of: find.byType(Sidebar), matching: find.text(text));
  Finder header(String text) => find.descendant(
    of: find.byKey(const Key('page-header')),
    matching: find.text(text),
  );

  Future<void> typeTitle(WidgetTester tester, String text) async {
    await tester.enterText(title(), text);
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
  }

  Future<void> shortcut(
    WidgetTester tester,
    LogicalKeyboardKey keyPressed, {
    bool shift = false,
  }) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(keyPressed);
    if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  Future<void> addPageInside(WidgetTester tester, String row) async {
    await tester.tapAt(
      tester.getCenter(inSidebar(row)),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a page inside'));
    await tester.pumpAndSettle();
  }

  Future<void> composeNihon(WidgetTester tester) async {
    await tester.tap(title());
    await tester.pump();
    kit.fake.applied.clear();
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'nihon',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      ),
    );
    await tester.pump();
  }

  testWidgets('no agent: a blank Page, and one-line titles', (tester) async {
    await show(tester);
    expect(inSidebar('Private'), findsOneWidget);
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(inSidebar('Threads'), findsOneWidget);
    expect(find.byType(ChatView), findsNothing);
    expect(tester.widget<TextField>(title()).focusNode!.hasFocus, isTrue);

    await typeTitle(tester, 'Trip to Lisbon');
    expect(storedTitle(), 'Trip to Lisbon');
    expect(inSidebar('Trip to Lisbon'), findsOneWidget);
    expect(header('Trip to Lisbon'), findsOneWidget);
    expect(header('Private'), findsOneWidget);

    await tester.enterText(title(), 'Plan\nnow');
    await tester.pumpAndSettle();
    expect(storedTitle(), 'Plan now');
    kit.fake.refuse = readOnly;
    await typeTitle(tester, 'Plan now!');
    expect(shownTitle(tester), 'Plan now');
    expect(find.text(refusal), findsOneWidget);
  });

  testWidgets('a ⌘Z after Liquid renames keeps the new title', (tester) async {
    await show(tester);
    await typeTitle(tester, 'Trip');

    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    await kit.workspace.rename(pageId(), 'Porto');
    await tester.pumpAndSettle();
    expect(shownTitle(tester), 'Porto');

    await tester.tap(title());
    await tester.pumpAndSettle();
    await undo(tester);
    expect(shownTitle(tester), 'Porto');
    expect(storedTitle(), 'Porto');
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('the area’s + and ⇧⌘N add Pages, which nest', (tester) async {
    await show(tester);
    await typeTitle(tester, 'Trip');

    await tester.tap(find.byTooltip('New Page or Source'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('new page')));
    await tester.pumpAndSettle();
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(tester.widget<TextField>(title()).focusNode!.hasFocus, isTrue);
    await typeTitle(tester, 'Budget');

    await shortcut(tester, LogicalKeyboardKey.keyN, shift: true);
    await tester.pumpAndSettle();
    final area = kit.fake.areas.single.id;
    expect(kit.fake.pages[area]!.map((page) => page.title), [
      'Trip',
      'Budget',
      '',
    ]);

    await addPageInside(tester, 'Trip');
    await tester.pumpAndSettle();
    final trip = kit.fake.pages[area]!.first.id;
    expect(kit.fake.pages[area]![1].parent, trip);
    expect(kit.workspace.open.value?.page, kit.fake.pages[area]![1].id);
    expect(find.bySemanticsLabel('Collapse'), findsOneWidget);

    await tester.tap(find.bySemanticsLabel('Collapse'));
    await tester.pumpAndSettle();
    expect(inSidebar('Untitled'), findsOneWidget);
    expect(find.bySemanticsLabel('Expand'), findsOneWidget);
  });

  testWidgets('a Page and the agent session take turns', (tester) async {
    await show(tester, session: agent());
    expect(find.byType(ChatView), findsOneWidget);
    expect(find.byType(WorkspacePage), findsNothing);

    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    expect(find.byType(WorkspacePage), findsOneWidget);
    expect(find.byType(ChatView), findsNothing);

    await tester.tap(find.byKey(const Key('session-thread-1')));
    await tester.pumpAndSettle();
    expect(find.byType(ChatView), findsOneWidget);

    await tester.tap(inSidebar('Untitled'));
    await tester.pumpAndSettle();
    await shortcut(tester, LogicalKeyboardKey.digit1);
    expect(find.byType(ChatView), findsOneWidget);
  });

  testWidgets('a Workspace problem shows in the sidebar', (tester) async {
    await show(tester, session: agent());
    kit.fake.refuse = readOnly;
    await shortcut(tester, LogicalKeyboardKey.keyN, shift: true);
    expect(inSidebar(refusal), findsOneWidget);
    expect(
      tester
          .widget<WarningNotice>(
            find.descendant(
              of: find.byType(Sidebar),
              matching: find.byType(WarningNotice),
            ),
          )
          .danger,
      isTrue,
      reason: 'a failure shows as one',
    );
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: agent(),
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

  testWidgets('a title is renamed when its composition ends', (tester) async {
    await show(tester);
    await composeNihon(tester);
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

  for (final (name, end) in <(String, Future<void> Function(WidgetTester))>[
    (
      'quitting',
      (tester) async {
        var drained = false;
        unawaited(kit.workspace.drain().then((_) => drained = true));
        await tester.pump();
        expect(drained, isTrue);
      },
    ),
    (
      'leaving the Page',
      (tester) async {
        await tester.tap(find.byTooltip('Settings'));
        await tester.pumpAndSettle();
      },
    ),
  ]) {
    testWidgets('$name writes an open composition', (tester) async {
      await show(tester);
      await composeNihon(tester);
      await end(tester);
      expect(renames(), ['nihon']);
    });
  }

  testWidgets('an older refused rename never wins', (tester) async {
    await show(tester);
    kit.fake.gate = Completer();
    await tester.enterText(title(), 'P');
    await tester.enterText(title(), 'Pl');
    kit.fake.refuseNext = 1;
    kit.fake.gate!.complete();
    await tester.pumpAndSettle();
    expect(storedTitle(), 'Pl');
    expect(shownTitle(tester), 'Pl');
  });

  testWidgets('a refused rename shows the stored title', (tester) async {
    await show(tester);
    kit.fake.deferredKinds = {BridgeQueryKind.pages};
    kit.fake.deferred = [];
    await tester.enterText(title(), 'P');
    await tester.pumpAndSettle();
    kit.fake.refuseNext = 1;
    await tester.enterText(title(), 'Pl');
    await tester.pumpAndSettle();
    kit.fake.deferredKinds = {};
    for (final release in [...kit.fake.deferred!]) {
      release.complete();
    }
    await tester.pumpAndSettle();
    expect(storedTitle(), 'P');
    expect(shownTitle(tester), 'P');
  });
}
