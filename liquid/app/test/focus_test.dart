import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/session.dart';
import 'package:liquid/src/rust/api/workspace.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/settings_page.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/workspace/code_block.dart';
import 'package:liquid/src/workspace/workspace_state.dart';

import 'fake_backend.dart';
import 'focus_kit.dart';
import 'page_kit.dart';

const info = SessionInfo(project: 'acme-api');

Future<void> key(WidgetTester tester, LogicalKeyboardKey key) async {
  await tester.sendKeyEvent(key);
  await tester.pumpAndSettle();
}

void main() {
  late FakeProject project;
  var sequence = 0;

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

  Future<void> resolve(WidgetTester tester, String id) async {
    project.session.emit(
      event(BridgeEventKind.requestResolved, ++sequence, requestId: id),
    );
    await tester.pumpAndSettle();
  }

  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  Future<void> openSettings(WidgetTester tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

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

  Finder modelItem() =>
      find.widgetWithText(PopupMenuItem<BridgeModelChoice>, 'Claude Opus 5.5');

  testWidgets('a click on another Page gives the shortcuts back to the shell', (
    tester,
  ) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('First'), TextPart('Second')]);
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    await kit.workspace.apply(
      mutation(
        BridgeMutationKind.createPage,
        area: kit.workspace.open.value!.area,
        text: 'Trip',
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(of: find.byType(Sidebar), matching: find.text('Trip')),
    );
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('power bar')), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    await undo(tester);
    expect(kit.fake.undos, 1);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('esc leaves a Code Block and the shortcuts still work', (
    tester,
  ) async {
    final kit = PageKit();
    await kit.show(tester, const [CodePart('print(1)')]);
    await tester.tap(
      find.descendant(
        of: find.byType(CodeBlock),
        matching: find.byKey(const Key('code')),
      ),
    );
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('power bar')), findsOneWidget);
  });

  testWidgets('Settings over a Page keep ⌘Z away from it; esc closes them', (
    tester,
  ) async {
    final kit = PageKit();
    await kit.show(tester, const [TextPart('First')]);
    await tester.tap(block(0));
    await tester.pumpAndSettle();
    await openSettings(tester);
    expect(find.byType(SettingsPage), findsOneWidget);
    await undo(tester);
    expect(kit.fake.undos, 0);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(SettingsPage), findsNothing);
    await undo(tester);
    expect(kit.fake.undos, 1);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets(
    'a question takes the keys without a click, and esc dismisses it',
    (tester) async {
      await show(tester);
      await ask(tester, 'q1', 'Your name');
      await read(tester);
      await key(tester, LogicalKeyboardKey.escape);
      expect(project.session.calls, ['dismiss q1']);
    },
  );

  testWidgets(
    'a question takes no key before it can be read, and once answered the keys return to the composer',
    (tester) async {
      await show(tester);
      await ask(tester, 'q1', 'Your name');
      await tester.enterText(find.byKey(const Key('answer')), 'draft');
      await key(tester, LogicalKeyboardKey.enter);
      expect(project.session.calls, isEmpty, reason: 'too early to answer');

      await read(tester);
      await key(tester, LogicalKeyboardKey.enter);
      expect(project.session.calls, ['answer q1 name=draft']);
      await resolve(tester, 'q1');
      expect(focused(tester, const Key('composer')), isTrue);
    },
  );

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

  testWidgets('an input method\'s Enter commits its text and sends nothing', (
    tester,
  ) async {
    await show(tester);
    await ask(tester, 'q1', 'Your name');
    await read(tester);
    compose(tester, const Key('answer'), 'nihon');
    await key(tester, LogicalKeyboardKey.enter);
    expect(
      project.session.calls,
      isEmpty,
      reason: 'the Enter commits the input method\'s text first',
    );

    await resolve(tester, 'q1');
    await tester.tap(find.byKey(const Key('composer')));
    compose(tester, const Key('composer'), 'nihon');
    await key(tester, LogicalKeyboardKey.enter);
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
    await key(tester, LogicalKeyboardKey.enter);
    expect(project.calls, contains('delete thread-2 worktree=false'));
  });

  testWidgets('a question closes a menu above it before taking keys', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Model'));
    await tester.pumpAndSettle();
    expect(modelItem(), findsOneWidget);

    await ask(tester, 'q1', 'Your name');
    expect(
      modelItem(),
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
    await openSettings(tester);
    expect(find.byType(SettingsPage), findsOneWidget);

    await ask(tester, 'q1', 'Your name');
    expect(find.byType(SettingsPage), findsNothing);
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await resolve(tester, 'q1');
    expect(focused(tester, const Key('composer')), isTrue);
  });
}
