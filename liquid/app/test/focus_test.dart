import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/action_area.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_page.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';

const info = SessionInfo(project: 'acme-api');

/// Keys in the whole shell: a dialog, a question and the composer take the keys without a click,
/// and only the keys meant for them.
void main() {
  late FakeProject project;
  var sequence = 0;

  /// Shows an open session with `threads` in the sidebar.
  Future<void> show(
    WidgetTester tester, {
    List<BridgeThreadSummary> threads = const [],
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    sequence = 0;
    project = FakeProject()..threadList = threads;
    final state = SessionState(project);
    await state.open('thread-1');
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(state: state, info: info, parseDiff: parsePatch),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// Asks the agent `text`, which takes the composer's place once the request arrives.
  Future<void> ask(WidgetTester tester, String id, String text) async {
    project.session.emit(
      event(
        BridgeEventKind.requestOpened,
        ++sequence,
        request: questionsOf(id, [questionOf('name', text)]),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// Lets a question take keys, as it does 0.5 s after it could first be read.
  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

  /// Sets the field's text as an input method leaves it while composing.
  void compose(WidgetTester tester, Key key, String text) {
    tester
        .widget<TextField>(find.byKey(key))
        .controller!
        .value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
      composing: TextRange(start: 0, end: text.length),
    );
  }

  testWidgets(
    'a question takes the keys without a click, and esc dismisses it',
    (tester) async {
      await show(tester);
      await ask(tester, 'q1', 'Your name');
      await read(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(project.session.calls, ['dismiss q1']);
    },
  );

  testWidgets('a question takes no key pressed before it could be read', (
    tester,
  ) async {
    await show(tester);
    await ask(tester, 'q1', 'Your name');
    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(project.session.calls, isEmpty, reason: 'too early to answer');

    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(project.session.calls, ['answer q1 name=draft']);
  });

  testWidgets('a key held from before a question does nothing in it', (
    tester,
  ) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
    await ask(tester, 'q1', 'Your name');
    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    await read(tester);
    expect(
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter),
      isTrue,
      reason: 'the repeat is taken by the card, so it types no new line',
    );
    await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
    expect(project.session.calls, isEmpty);

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(project.session.calls, ['answer q1 name=draft']);
  });

  testWidgets('a question waits again once Liquid is back in front', (
    tester,
  ) async {
    await show(tester);
    await ask(tester, 'q1', 'Your name');
    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    // It arrived while the user worked in another app.
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await read(tester);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(
      project.session.calls,
      isEmpty,
      reason: 'the key that brought Liquid back answers nothing',
    );

    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(project.session.calls, ['answer q1 name=draft']);
  });

  testWidgets('once answered, the keys go back to the composer', (
    tester,
  ) async {
    await show(tester);
    await ask(tester, 'q1', 'Your name');
    await tester.enterText(find.byKey(const Key('answer')), 'draft');
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    project.session.emit(
      event(BridgeEventKind.requestResolved, ++sequence, requestId: 'q1'),
    );
    await tester.pumpAndSettle();
    expect(focused(tester, const Key('composer')), isTrue);
  });

  testWidgets('a key in a composition answers and sends nothing', (
    tester,
  ) async {
    await show(tester);
    await ask(tester, 'q1', 'Your name');
    await read(tester);
    compose(tester, const Key('answer'), 'nihon');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      project.session.calls,
      isEmpty,
      reason: 'the Enter commits the input method\'s text first',
    );
  });

  testWidgets('Enter that ends a composition in the composer sends nothing', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('composer')));
    compose(tester, const Key('composer'), 'nihon');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      project.session.calls.where((call) => call.startsWith('send')),
      isEmpty,
      reason: 'the Enter commits the input method\'s text first',
    );
  });

  testWidgets('a danger dialog takes ⏎ without a click', (tester) async {
    await show(tester, threads: [threadOf('thread-2', title: 'Fix login')]);
    await tester.tap(find.text('Fix login'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Delete'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(project.calls, contains('delete thread-2 worktree=false'));
  });

  testWidgets('a question closes a menu above it before taking keys', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Model'));
    await tester.pumpAndSettle();
    expect(
      find.widgetWithText(PopupMenuItem<BridgeModelChoice>, 'Claude Opus 5.5'),
      findsOneWidget,
    );

    await ask(tester, 'q1', 'Your name');
    expect(
      find.widgetWithText(PopupMenuItem<BridgeModelChoice>, 'Claude Opus 5.5'),
      findsNothing,
      reason: 'the menu would hide the question',
    );
    await read(tester);
    await tester.enterText(find.byKey(const Key('answer')), 'Bob');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(project.session.calls, ['answer q1 name=Bob']);
  });

  testWidgets('a question closes Settings, and the keys go back after it', (
    tester,
  ) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(find.byType(SettingsPage), findsOneWidget);

    await ask(tester, 'q1', 'Your name');
    expect(find.byType(SettingsPage), findsNothing);
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    project.session.emit(
      event(BridgeEventKind.requestResolved, ++sequence, requestId: 'q1'),
    );
    await tester.pumpAndSettle();
    expect(focused(tester, const Key('composer')), isTrue);
  });

  testWidgets('focus shows on controls inside the input area', (tester) async {
    await show(tester);
    final context = tester.element(find.byType(Composer));
    await tabTo(tester, find.byTooltip('Model'));
    expect(
      find.byType(Composer),
      paintsAbove(context.ws.surface, Theme.of(context).focusColor),
      reason: 'the highlight is painted above the input area',
    );
  });

  testWidgets('focus and hover show in the Settings sections', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    final context = tester.element(find.byType(SettingsPage));
    final theme = Theme.of(context);
    Finder section(String label) => find.ancestor(
      of: find
          .descendant(of: find.byType(SettingsPage), matching: find.text(label))
          .first,
      matching: find.byType(InkWell),
    );
    await tabTo(tester, section('Shortcuts'));
    expect(
      find.byType(SettingsPage),
      paintsAbove(context.ws.selected, theme.focusColor),
      reason: 'the highlight is painted above the chosen section',
    );

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: tester.getCenter(section('Shortcuts')));
    addTearDown(mouse.removePointer);
    await tester.pumpAndSettle();
    expect(
      find.byType(SettingsPage),
      paintsAbove(context.ws.selected, theme.hoverColor),
      reason: 'the hover is painted above the chosen section',
    );
  });
}
