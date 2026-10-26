import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(
  title: 'Rate limit for /login',
  project: 'acme-api',
  harness: 'pi',
  device: 'MacBook',
);

/// Keys in the whole shell, where the shell's own focus is taken first: dialogs, the Diff tab
/// and the input area take the keys without a click, and only keys meant for them.
void main() {
  late FakeBackend backend;

  Future<void> show(WidgetTester tester) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'a.txt',
          status: BridgeChangeStatus.modified,
          added: 1,
          removed: 1,
        ),
        BridgeFileChange(
          path: 'b.txt',
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
        home: AgentShell(state: SessionState(backend), info: info),
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

  BridgeEvent approval(String id, {String command = 'git push'}) => event(
    BridgeEventKind.requestOpened,
    request: BridgeRequest(
      id: id,
      approval: BridgeApproval(
        kind: BridgeRequestKind.commandExecution,
        tool: 'bash',
        arguments: '{"command":"$command"}',
        readOnly: false,
      ),
    ),
  );

  /// Lets an approval take keys (ADR-0207).
  Future<void> read(WidgetTester tester) =>
      tester.pump(const Duration(milliseconds: 500));

  bool focused(WidgetTester tester, Key key) =>
      tester.widget<TextField>(find.byKey(key)).focusNode?.hasFocus ?? false;

  testWidgets('a confirmation takes ⏎ at once', (tester) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.confirm));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('confirm q1 true'));
  });

  testWidgets('a selection takes ↓ and ⏎ at once', (tester) async {
    await show(tester);
    await emit(
      tester,
      question(BridgeQuestionKind.select, options: ['Sliding', 'Fixed']),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(backend.calls, contains('answer q1 Fixed'));
  });

  testWidgets('input and editor questions focus their field', (tester) async {
    await show(tester);
    await emit(tester, question(BridgeQuestionKind.input));
    expect(focused(tester, const Key('answer')), isTrue);
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

  testWidgets('the Diff tab takes ↑↓ at once', (tester) async {
    await show(tester);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit3);
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

  testWidgets('a long call in an approval scrolls from the keyboard', (
    tester,
  ) async {
    await show(tester);
    final command = List.generate(40, (line) => 'echo $line').join(r'\n');
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    await emit(tester, approval('a1', command: command));
    await read(tester);
    final scroll = find
        .ancestor(
          of: find.textContaining('echo 39'),
          matching: find.byType(Scrollable),
        )
        .first;
    double offset() => tester.state<ScrollableState>(scroll).position.pixels;
    expect(offset(), 0);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();
    final line = offset();
    expect(line, greaterThan(0));
    await tester.sendKeyEvent(LogicalKeyboardKey.pageDown);
    await tester.pumpAndSettle();
    expect(offset(), greaterThan(line));
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(
      backend.calls.last,
      'decide a1 allowOnce',
      reason: 'the approval keeps the keys',
    );
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('an approval takes no key pressed before it could be read', (
    tester,
  ) async {
    await show(tester);
    await emit(tester, event(BridgeEventKind.turnStarted, turn: 1));
    await emit(tester, approval('a1'));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.enter);
    await read(tester);
    await tester.sendKeyRepeatEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.enter);
    expect(
      backend.calls.where((call) => call.startsWith('decide')),
      isEmpty,
      reason: 'neither early keys nor a held key decide',
    );

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(backend.calls.last, 'decide a1 allowOnce');
    await emit(
      tester,
      event(BridgeEventKind.requestResolved, request: BridgeRequest(id: 'a1')),
    );
    expect(focused(tester, const Key('composer')), isTrue);
  });

  testWidgets('an approval or a question closes Settings', (tester) async {
    await show(tester);
    for (final (request, shown, resolved) in [
      (approval('a1'), 'Run a shell command?', BridgeEventKind.requestResolved),
      (
        question(BridgeQuestionKind.confirm),
        'Question',
        BridgeEventKind.userInputResolved,
      ),
    ]) {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.comma);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
      expect(find.text('Previous or next file'), findsOneWidget);

      await emit(tester, request);
      expect(find.text('Previous or next file'), findsNothing);
      expect(find.text(shown), findsOneWidget);
      await read(tester);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await emit(
        tester,
        event(resolved, request: BridgeRequest(id: request.request!.id)),
      );
    }
  });
}
