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

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

/// Keys in the whole shell, where the shell's own focus is taken first: dialogs, the Diff panel
/// and the input area take the keys without a click, and only keys meant for them.
void main() {
  late FakeBackend backend;

  Future<void> show(
    WidgetTester tester, {
    List<BridgeModel> models = const [],
  }) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()
      ..modelList = models
      ..changed = [
        BridgeFileChange(
          path: 'a.txt',
          shown: 'a.txt',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 1,
        ),
        BridgeFileChange(
          path: 'b.txt',
          shown: 'b.txt',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 1,
        ),
      ]
      ..patches = {
        'a.txt': '@@ -1 +1 @@\n-a\n+first file\n',
        'b.txt': '@@ -1 +1 @@\n-b\n+second file\n',
      };
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: AgentShell(
          state: SessionState(backend),
          info: info,
          parseDiff: parsePatch,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Future<void> emit(WidgetTester tester, BridgeEvent event) async {
    backend.controller.add(event);
    await tester.pumpAndSettle();
  }

  BridgeEvent question(BridgeQuestionKind kind, {List<String>? options}) =>
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q1',
          question: BridgeQuestion(
            kind: kind,
            title: 'Question',
            detail: '',
            options: options ?? const [],
          ),
        ),
      );

  BridgeEvent confirm(String id) => event(
    BridgeEventKind.userInputRequested,
    request: BridgeRequest(
      id: id,
      question: BridgeQuestion(
        kind: BridgeQuestionKind.confirm,
        title: 'Push now?',
        detail: '',
        options: const [],
      ),
    ),
  );

  BridgeEvent resolved(String id) =>
      event(BridgeEventKind.userInputResolved, request: BridgeRequest(id: id));

  /// Lets a question take keys.
  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

  testWidgets('a confirmation takes ⏎ without a click first', (tester) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.confirm));
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('confirm q1 true'));
  });

  testWidgets('a selection takes ↓ and ⏎ without a click first', (
    tester,
  ) async {
    await show(tester);
    await emit(
      tester,
      question(BridgeQuestionKind.select, options: ['Sliding', 'Fixed']),
    );
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('answer q1 Fixed'));
  });

  testWidgets('input and editor questions focus their field', (tester) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.input));
    expect(focused(tester, const Key('answer')), isTrue);
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('dismiss q1'));

    backend.controller.add(
      event(
        BridgeEventKind.userInputRequested,
        request: BridgeRequest(
          id: 'q2',
          question: BridgeQuestion(
            kind: BridgeQuestionKind.editor,
            title: 'Edit',
            detail: 'text',
            options: const [],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(focused(tester, const Key('answer')), isTrue);
  });

  testWidgets('the Diff panel takes ↑↓ at once', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyD);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(
      find.textContaining('first file', findRichText: true),
      findsOneWidget,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();
    expect(
      find.textContaining('second file', findRichText: true),
      findsOneWidget,
    );
  });

  testWidgets('a decided dialog gives the keys back to the input area', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.confirm));
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    expect(backend.calls.last, 'dismiss q1');
    await emit(
      tester,
      event(
        BridgeEventKind.userInputResolved,
        request: BridgeRequest(id: 'q1'),
      ),
    );
    expect(focused(tester, const Key('composer')), isTrue);
  });

  testWidgets('a question takes no key pressed before it could be read', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(tester, confirm('q1'));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
    await read(tester);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
    expect(
      backend.calls.where(
        (call) => call.startsWith('confirm') || call.startsWith('dismiss'),
      ),
      isEmpty,
      reason: 'neither early keys nor a held key answer',
    );

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'confirm q1 true');
    await emit(tester, resolved('q1'));
    expect(focused(tester, const Key('composer')), isTrue);
  });

  testWidgets('a question waits again once Liquid is back in front', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, confirm('q1'));
    // It arrived while the user worked in another app.
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await read(tester);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(
      backend.calls.where((call) => call.startsWith('confirm')),
      isEmpty,
      reason: 'the key that brought Liquid back answers nothing',
    );

    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'confirm q1 true');
  });

  testWidgets('a question closes a menu above it before taking keys', (
    tester,
  ) async {
    await show(
      tester,
      models: const [
        BridgeModel(provider: 'faux', id: 'faux-1', name: 'Faux One'),
      ],
    );
    await tester.tap(find.byTooltip('Model'));
    await tester.pumpAndSettle();
    expect(find.text('Faux One'), findsOneWidget);

    await emit(tester, confirm('q1'));
    expect(
      find.text('Faux One'),
      findsNothing,
      reason: 'the menu would hide the question',
    );
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'confirm q1 true');
  });

  testWidgets('a key held past an answer does nothing more', (tester) async {
    await show(tester);
    await tester.enterText(find.byKey(const Key('composer')), 'half-written');
    await emit(tester, event(BridgeEventKind.turnStarted));
    for (final (id, key) in [
      ('q1', LogicalKeyboardKey.enter),
      ('q2', LogicalKeyboardKey.escape),
    ]) {
      await emit(tester, confirm(id));
      await read(tester);
      await tester.sendKeyDownEvent(key);
      await emit(tester, resolved(id));
      // The key repeats in the input area, which has the keys again; it takes the repeat
      // itself, so the text field types no new line either.
      expect(await tester.sendKeyRepeatEvent(key), isTrue, reason: '$key');
      await tester.sendKeyUpEvent(key);
      await tester.pumpAndSettle();
    }
    expect(backend.calls, ['confirm q1 true', 'dismiss q2']);
  });

  testWidgets('a question closes Settings', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    expect(find.text('Previous or next file'), findsOneWidget);

    await emit(tester, confirm('q1'));
    expect(find.text('Previous or next file'), findsNothing);
    expect(find.text('Push now?'), findsOneWidget);
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await emit(tester, resolved('q1'));
    expect(
      focused(tester, const Key('composer')),
      isTrue,
      reason: 'the keys go back to the input area',
    );
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

  testWidgets('focus shows on the profile choice and on notice links', (
    tester,
  ) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    final context = tester.element(find.text('Standard'));
    final focus = Theme.of(context).focusColor;
    final standard = find.ancestor(
      of: find.text('Standard'),
      matching: find.byType(InkWell),
    );
    await tabTo(tester, standard);
    expect(
      find.byType(SettingsPage),
      paintsAbove(context.ws.canvas, focus),
      reason: 'the highlight is painted above the selected profile',
    );

    await tester.tap(find.text('⌘L'));
    await tester.pumpAndSettle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyN);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
    final link = find.text('Use anyway, clear New Session');
    Focus.of(tester.element(link)).requestFocus();
    await tester.pumpAndSettle();
    expect(link, findsOneWidget);
    expect(
      find.byType(SettingsPage),
      paintsAbove(context.ws.warningSubtle, focus),
      reason: 'the highlight is painted above the notice',
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

  /// Sets the field's text as an input method leaves it while composing.
  void compose(WidgetTester tester, Finder field, String text) {
    tester.widget<TextField>(field).controller!.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
      composing: TextRange(start: 0, end: text.length),
    );
  }

  testWidgets('Enter that ends a composition answers nothing', (tester) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.input));
    await read(tester);
    compose(tester, find.byKey(const Key('answer')), 'nihon');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.startsWith('answer')),
      isEmpty,
      reason: 'the Enter commits the input method\'s text first',
    );
  });

  testWidgets('Enter that ends a composition sends nothing', (tester) async {
    await show(tester);
    await tester.tap(find.byKey(const Key('composer')));
    compose(tester, find.byKey(const Key('composer')), 'nihon');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(
      backend.calls.where((call) => call.startsWith('send')),
      isEmpty,
      reason: 'the Enter commits the input method\'s text first',
    );
  });

  testWidgets('a key on a closing card never answers the next request', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted));
    await emit(tester, confirm('q1'));
    await emit(tester, confirm('q2'));
    await read(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    // q1 closes; its card stays until the next frame.
    backend.controller.add(resolved('q1'));
    await tester.idle();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls.where((call) => call.startsWith('confirm')), [
      'confirm q1 true',
    ], reason: 'q2 was never shown long enough to be read');
  });
}
