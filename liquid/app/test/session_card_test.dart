import 'package:flutter/material.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/chat_view.dart';
import 'package:liquid/src/agent/session_card.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/agent/shell.dart';
import 'package:liquid/src/agent/sidebar.dart';
import 'package:liquid/src/diff/diff_panel.dart';
import 'package:liquid/src/diff/diff_settings.dart';
import 'package:liquid/src/diff/diff_view.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/rust/api/git.dart';
import 'package:liquid/src/rust/api/history.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/icons.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

const info = SessionInfo(project: 'acme-api', harness: 'pi', device: 'MacBook');

void main() {
  late FakeBackend backend;
  late SessionState state;

  /// Shows the shell in a window `size` big. `windowSize` is what the window reports to the
  /// layout, when a test needs a wider window than the surface it draws on.
  Future<void> show(
    WidgetTester tester, {
    Size size = const Size(1600, 900),
    Size? windowSize,
    SessionCardSettingsController? cardSettings,
    DiffSettingsController? diffSettings,
    void Function(FakeBackend backend)? setup,
  }) async {
    await tester.binding.setSurfaceSize(size);
    backend = FakeBackend();
    setup?.call(backend);
    state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        builder: (context, child) => windowSize == null
            ? child!
            : MediaQuery(
                data: MediaQuery.of(context).copyWith(size: windowSize),
                child: child!,
              ),
        home: AgentShell(
          state: state,
          info: info,
          cardSettings: cardSettings,
          diffSettings: diffSettings,
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  /// Opens the card's popover from the sliders button, or closes it.
  Future<void> toggleCard(WidgetTester tester) async {
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();
  }

  Future<void> rightClickCard(WidgetTester tester) async {
    await tester.tap(
      find.byKey(const Key('session-details-card')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
  }

  /// Captures what the app copies to the clipboard.
  List<String> captureClipboard(WidgetTester tester) {
    final copied = <String>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied.add((call.arguments as Map)['text'] as String);
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    return copied;
  }

  /// Whether keyboard focus sits on a control inside `finder`.
  bool hasFocusIn(Finder finder) {
    final focused = FocusManager.instance.primaryFocus?.context;
    return focused != null &&
        find
            .descendant(
              of: finder,
              matching: find.byElementPredicate(
                (element) => element == focused,
              ),
            )
            .evaluate()
            .isNotEmpty;
  }

  /// Shows a session whose last agent answer reads `Done`.
  Future<void> showAnswer(WidgetTester tester) async {
    await show(
      tester,
      setup: (backend) => backend
        ..stored = historyOf(
          tree: [
            BridgeTreeNode(id: 'u1', parent: null, kind: BridgeEntryKind.user),
            BridgeTreeNode(
              id: 'a1',
              parent: 'u1',
              kind: BridgeEntryKind.assistant,
            ),
          ],
          leaf: 'a1',
        )
        ..transcripts[''] = BridgeTranscript(
          items: [
            chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
            chatItem('a1', BridgeItemKind.assistantMessage, text: 'Done'),
          ],
          more: false,
        ),
    );
    await state.loadChat();
    await tester.pumpAndSettle();
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(key);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();
  }

  /// Pumps the card alone in a space `height` high, where the window would overflow its composer.
  Future<void> pumpCard(WidgetTester tester, {required double height}) async {
    backend = FakeBackend();
    state = SessionState(backend);
    await state.git.refresh();
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Material(
          child: Align(
            alignment: Alignment.topRight,
            child: SizedBox(
              width: 280,
              height: height,
              child: SessionCard(
                state: state,
                device: 'MacBook',
                settings: SessionCardSettingsController(),
                onChanges: () {},
                beside: true,
                onOutsidePress: () {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  Finder inCard(Finder finder) => find.descendant(
    of: find.byKey(const Key('session-details-card')),
    matching: finder,
  );

  Rect chatColumn(WidgetTester tester) =>
      tester.getRect(find.byKey(const Key('chat-column')));

  testWidgets(
    'the header keeps the title and chips and ends with the details button',
    (tester) async {
      await show(tester);
      final header = find.byKey(const Key('session-header'));
      expect(
        find.descendant(of: header, matching: find.text('acme-api')),
        findsOneWidget,
      );
      for (final gone in [
        'branch-chip',
        'pull-request-chip',
        'git-next',
        'changed-files',
      ]) {
        expect(
          find.descendant(of: header, matching: find.byKey(Key(gone))),
          findsNothing,
          reason: '$gone moved into the session card',
        );
      }
      final button = find.descendant(
        of: header,
        matching: find.byTooltip('Session details'),
      );
      expect(button, findsOneWidget);
      expect(
        tester.getRect(button).right,
        closeTo(tester.getRect(header).right - 16, 1),
        reason: 'at the end of the header',
      );
    },
  );

  testWidgets('the tab strip offers only the Chat', (tester) async {
    await show(tester);
    expect(find.text('Chat'), findsOneWidget);
    expect(find.text('Diff'), findsNothing);
  });

  testWidgets('the details button shows and hides the session card', (
    tester,
  ) async {
    await show(tester);
    expect(find.byKey(const Key('session-details-card')), findsOneWidget);
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('session-details-card')), findsNothing);
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('session-details-card')), findsOneWidget);
  });

  testWidgets('⌘⌥B shows and hides the session card', (tester) async {
    await show(tester);
    Future<void> chord() async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyB);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
      await tester.pumpAndSettle();
    }

    await chord();
    expect(find.byKey(const Key('session-details-card')), findsNothing);
    await chord();
    expect(find.byKey(const Key('session-details-card')), findsOneWidget);
  });

  testWidgets('the card is 280 wide, 12 from the top and right of the chat', (
    tester,
  ) async {
    await show(tester);
    final card = tester.getRect(find.byKey(const Key('session-details-card')));
    final column = chatColumn(tester);
    expect(card.width, 280);
    expect(card.right, closeTo(column.right - 12, 0.5));
    expect(card.top, closeTo(column.top + 12, 0.5), reason: 'beside the chat');
  });

  testWidgets(
    'the card becomes a popover below the header when the chat would be too narrow',
    (tester) async {
      await show(tester, size: const Size(1100, 900));
      await toggleCard(tester);
      expect(
        tester.getRect(find.byKey(const Key('session-details-card'))).top,
        closeTo(chatColumn(tester).top + 4, 0.5),
        reason: 'a popover, under the tabs',
      );
    },
  );

  testWidgets('the card becomes a popover while the Diff panel is open', (
    tester,
  ) async {
    await show(tester, size: const Size(2000, 900));
    expect(
      tester.getRect(find.byKey(const Key('session-details-card'))).top,
      closeTo(chatColumn(tester).top + 12, 0.5),
    );
    await press(tester, LogicalKeyboardKey.keyD);
    expect(
      find.byKey(const Key('session-details-card')),
      findsNothing,
      reason: 'the popover waits closed, as in T3 Code',
    );
    await toggleCard(tester);
    expect(
      tester.getRect(find.byKey(const Key('session-details-card'))).top,
      closeTo(chatColumn(tester).top + 4, 0.5),
    );
  });

  testWidgets('the details button opens the inline card with focus inside it', (
    tester,
  ) async {
    await show(tester);
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Session details'));
    await tester.pumpAndSettle();

    expect(focusIsIn(const Key('session-details-card')), isTrue);
  });

  testWidgets('the popover closes when focus leaves it, as in T3 Code', (
    tester,
  ) async {
    await show(tester, size: const Size(1100, 900));
    await toggleCard(tester);
    expect(focusIsIn(const Key('session-details-card')), isTrue);

    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('session-details-card')), findsNothing);
  });

  testWidgets('an open rows menu keeps the popover when focus leaves it', (
    tester,
  ) async {
    await show(tester, size: const Size(1100, 900));
    await toggleCard(tester);
    await tester.tap(
      find.byKey(const Key('session-details-card')),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();

    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('session-details-card')), findsOneWidget);
  });

  testWidgets('the card scrolls within a space smaller than its content', (
    tester,
  ) async {
    await pumpCard(tester, height: 60);
    expect(
      tester.getSize(find.byKey(const Key('session-details-card'))).height,
      60,
    );
    expect(inCard(find.byType(Scrollable)), findsOneWidget);
  });

  testWidgets('the branch row names the branch and its commits ahead', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) =>
          backend.git = gitStatusOf(branch: 'fix/login-limit', ahead: 2),
    );
    expect(inCard(find.text('fix/login-limit')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('branch-chip')),
        matching: find.text('↑2'),
      ),
      findsOneWidget,
    );
  });

  testWidgets(
    'a checkout outside Git leaves out the branch and offers to initialize Git',
    (tester) async {
      await show(tester, setup: (backend) => backend.git = null);
      expect(inCard(find.byKey(const Key('branch-chip'))), findsNothing);
      expect(inCard(find.byKey(const Key('changes-row'))), findsNothing);
      expect(inCard(find.text('Initialize Git')), findsOneWidget);
    },
  );

  testWidgets(
    'a pull request row shows its number and title and is left out without one',
    (tester) async {
      await show(tester);
      expect(inCard(find.byKey(const Key('pull-request-chip'))), findsNothing);

      backend.current = pullRequestOutcomeOf(
        pullRequest: pullRequestOf(
          number: 412,
          title: 'Add a rate limit to /login',
        ),
      );
      await state.pulls.refresh();
      await tester.pumpAndSettle();
      expect(inCard(find.text('#412')), findsOneWidget);
      expect(inCard(find.text('Add a rate limit to /login')), findsOneWidget);
    },
  );

  testWidgets(
    'the Changes row counts the latest turn and opens the Diff panel',
    (tester) async {
      await show(
        tester,
        setup: (backend) => backend.turnList = [
          turnOf('T1', number: 1, added: 120, removed: 30),
        ],
      );
      await state.readChanges();
      await tester.pumpAndSettle();
      final row = inCard(find.byKey(const Key('changes-row')));
      expect(
        find.descendant(of: row, matching: find.text('+120')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: row, matching: find.text('−30')),
        findsOneWidget,
      );

      await tester.tap(row);
      await tester.pumpAndSettle();
      expect(find.byType(DiffView), findsOneWidget);
    },
  );

  testWidgets('the Git action row is left out of a card too short for it', (
    tester,
  ) async {
    await pumpCard(tester, height: 100);
    expect(inCard(find.byKey(const Key('git-next'))), findsNothing);
    expect(inCard(find.byKey(const Key('branch-chip'))), findsOneWidget);

    await pumpCard(tester, height: 200);
    expect(inCard(find.byKey(const Key('git-next'))), findsOneWidget);
  });

  testWidgets(
    '⌘D opens the Diff panel beside the chat, and ⌘D or ✕ closes it',
    (tester) async {
      await show(tester);
      await press(tester, LogicalKeyboardKey.keyD);
      expect(find.byType(DiffView), findsOneWidget);
      expect(find.byKey(const Key('diff-panel')), findsOneWidget);
      await press(tester, LogicalKeyboardKey.keyD);
      expect(find.byType(DiffView), findsNothing);

      await press(tester, LogicalKeyboardKey.keyD);
      await tester.tap(find.byKey(const Key('diff-panel-close')));
      await tester.pumpAndSettle();
      expect(find.byType(DiffView), findsNothing);
    },
  );

  testWidgets('a hidden row keeps its shortcut', (tester) async {
    await show(
      tester,
      cardSettings: SessionCardSettingsController(
        const SessionCardSettings(hidden: {CardItem.changes}),
      ),
    );
    expect(inCard(find.byKey(const Key('changes-row'))), findsNothing);
    await press(tester, LogicalKeyboardKey.keyD);
    expect(find.byType(DiffView), findsOneWidget);
  });

  testWidgets(
    'right-click offers the rows, all checked, with Reset disabled by default',
    (tester) async {
      await show(tester);
      await tester.tap(
        find.byKey(const Key('session-details-card')),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      for (final label in [
        'Branch and pull request',
        'Git actions',
        'Changes',
        'Statistics',
        'Context window',
      ]) {
        expect(
          tester
              .widget<CheckboxMenuButton>(
                find.widgetWithText(CheckboxMenuButton, label),
              )
              .value,
          isTrue,
        );
      }
      expect(find.text('Reset to default'), findsOneWidget);
      final reset = tester.widget<MenuItemButton>(
        find.ancestor(
          of: find.text('Reset to default'),
          matching: find.byType(MenuItemButton),
        ),
      );
      expect(reset.onPressed, isNull);
    },
  );

  testWidgets(
    'a row unchecked in the menu hides, and Reset to default shows it again',
    (tester) async {
      await show(tester);
      Future<void> rightClick() async {
        await tester.tap(
          find.byKey(const Key('session-details-card')),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
      }

      await rightClick();
      await tester.tap(find.widgetWithText(CheckboxMenuButton, 'Changes'));
      await tester.pumpAndSettle();
      expect(inCard(find.byKey(const Key('changes-row'))), findsNothing);

      await rightClick();
      await tester.tap(find.text('Reset to default'));
      await tester.pumpAndSettle();
      expect(inCard(find.byKey(const Key('changes-row'))), findsOneWidget);
    },
  );

  testWidgets('an agent answer branches the session from that answer', (
    tester,
  ) async {
    await showAnswer(tester);
    await tester.tap(
      find.descendant(
        of: find.byType(ChatView),
        matching: find.byIcon(LucideIcons.ellipsis),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Branch from here'));
    await tester.pumpAndSettle();
    expect(backend.calls, contains('branch a1'));
  });

  testWidgets('Branch from here waits until the agent is idle', (tester) async {
    await show(
      tester,
      setup: (backend) => backend
        ..stored = historyOf(
          tree: [
            BridgeTreeNode(id: 'u1', parent: null, kind: BridgeEntryKind.user),
            BridgeTreeNode(
              id: 'a1',
              parent: 'u1',
              kind: BridgeEntryKind.assistant,
            ),
          ],
          leaf: 'a1',
        )
        ..transcripts[''] = BridgeTranscript(
          items: [
            chatItem('u1', BridgeItemKind.userMessage, text: 'Go'),
            chatItem('a1', BridgeItemKind.assistantMessage, text: 'Done'),
          ],
          more: false,
        ),
    );
    await state.loadChat();
    await tester.pumpAndSettle();
    backend.controller.add(event(BridgeEventKind.turnStarted));
    await tester.pumpAndSettle();
    await tester.tap(
      find.descendant(
        of: find.byType(ChatView),
        matching: find.byIcon(LucideIcons.ellipsis),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Branch from here'));
    await tester.pumpAndSettle();
    expect(backend.calls, isNot(contains('branch a1')));
    expect(find.text('Branch once the agent is idle.'), findsOneWidget);
    // The status bar drops its notice after a while.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets('the Git action row sits in the card, not the header', (
    tester,
  ) async {
    await show(tester);
    expect(inCard(find.byKey(const Key('git-next'))), findsOneWidget);
    expect(inCard(find.byKey(const Key('git-menu'))), findsOneWidget);
  });

  testWidgets(
    'the branch name and its chevron are two targets that both open the branch menu',
    (tester) async {
      await show(
        tester,
        setup: (backend) => backend.branches = BridgeBranches(
          branches: [
            branchOf('main', current: true),
            branchOf('feature/token-bucket'),
          ],
          running: const [],
        ),
      );
      for (final part in ['branch-name', 'branch-chevron']) {
        await tester.tap(inCard(find.byKey(Key(part))));
        await tester.pumpAndSettle();
        expect(find.text('feature/token-bucket'), findsOneWidget, reason: part);
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
      }
    },
  );

  testWidgets('the split hairline shows while the branch row is hovered', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) => backend.git = gitStatusOf(branch: 'main'),
    );
    Color hairline() =>
        tester.widget<Container>(find.byKey(const Key('branch-split'))).color!;
    final ws = tester.element(find.byKey(const Key('branch-split'))).ws;
    expect(hairline(), Colors.transparent);

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: Offset.zero);
    await mouse.moveTo(
      tester.getCenter(inCard(find.byKey(const Key('branch-name')))),
    );
    await tester.pump();
    expect(hairline(), ws.border);

    await mouse.moveTo(Offset.zero);
    await tester.pump();
    expect(hairline(), Colors.transparent);
  });

  testWidgets('the branch row names the full branch in a tooltip', (
    tester,
  ) async {
    await show(
      tester,
      setup: (backend) =>
          backend.git = gitStatusOf(branch: 'fix/login-limit', ahead: 2),
    );
    expect(find.byTooltip('fix/login-limit'), findsOneWidget);
  });

  testWidgets(
    'the pull request row’s tooltip gives its number and full title',
    (tester) async {
      await show(tester);
      backend.current = pullRequestOutcomeOf(
        pullRequest: pullRequestOf(
          number: 412,
          title: 'Add a rate limit to /login',
        ),
      );
      await state.pulls.refresh();
      await tester.pumpAndSettle();
      expect(find.byTooltip('#412 Add a rate limit to /login'), findsOneWidget);
    },
  );

  testWidgets(
    'in a narrow window the card waits closed until the sliders button opens it',
    (tester) async {
      await show(tester, size: const Size(1100, 900));
      expect(find.byKey(const Key('session-details-card')), findsNothing);
      await toggleCard(tester);
      expect(find.byKey(const Key('session-details-card')), findsOneWidget);
    },
  );

  testWidgets(
    'opening the popover moves focus into it, and Escape closes it and returns focus to the sliders button',
    (tester) async {
      await show(tester, size: const Size(1100, 900));
      await toggleCard(tester);
      expect(hasFocusIn(find.byKey(const Key('session-details-card'))), isTrue);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-details-card')), findsNothing);
      expect(hasFocusIn(find.byKey(const Key('session-details'))), isTrue);
    },
  );

  testWidgets(
    'a press outside the popover closes it, and its button toggles it without reopening',
    (tester) async {
      await show(tester, size: const Size(1100, 900));
      await toggleCard(tester);
      await tester.tap(find.text('Chat'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-details-card')), findsNothing);

      await toggleCard(tester);
      expect(find.byKey(const Key('session-details-card')), findsOneWidget);
      await toggleCard(tester);
      expect(find.byKey(const Key('session-details-card')), findsNothing);
    },
  );

  testWidgets(
    'a popover closes when the window widens, and stays closed when it narrows again',
    (tester) async {
      await show(tester, size: const Size(1100, 900));
      await toggleCard(tester);
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      await tester.pumpAndSettle();
      expect(
        tester.getRect(find.byKey(const Key('session-details-card'))).top,
        closeTo(chatColumn(tester).top + 12, 0.5),
        reason: 'the inline card keeps its own open state',
      );
      await tester.binding.setSurfaceSize(const Size(1100, 900));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('session-details-card')), findsNothing);
    },
  );

  testWidgets('a branch chosen from the popover switches the branch', (
    tester,
  ) async {
    await show(
      tester,
      size: const Size(1100, 900),
      setup: (backend) => backend.branches = BridgeBranches(
        branches: [
          branchOf('main', current: true),
          branchOf('feature/token-bucket'),
        ],
        running: const [],
      ),
    );
    await toggleCard(tester);
    await tester.tap(inCard(find.byKey(const Key('branch-name'))));
    await tester.pumpAndSettle();
    await tester.tap(find.text('feature/token-bucket'));
    await tester.pumpAndSettle();
    expect(backend.calls.last, 'switch feature/token-bucket');
    // The status bar drops its notice after a while.
    await tester.pump(const Duration(seconds: 7));
  });

  testWidgets(
    'a card with every row hidden offers Choose what to show, which opens the rows menu',
    (tester) async {
      await show(
        tester,
        cardSettings: SessionCardSettingsController(
          SessionCardSettings(hidden: CardItem.values.toSet()),
        ),
      );
      await tester.tap(inCard(find.text('Choose what to show')));
      await tester.pumpAndSettle();
      expect(
        find.widgetWithText(CheckboxMenuButton, 'Changes'),
        findsOneWidget,
      );
    },
  );

  testWidgets('the card fades the edge its rows scroll past', (tester) async {
    await pumpCard(tester, height: 60);
    expect(inCard(find.byType(ShaderMask)), findsOneWidget);
    await pumpCard(tester, height: 2000);
    expect(inCard(find.byType(ShaderMask)), findsNothing);
  });

  testWidgets(
    'the rows menu stays open while a row is toggled, and Escape closes it',
    (tester) async {
      await show(tester);
      await rightClickCard(tester);
      await tester.tap(find.widgetWithText(CheckboxMenuButton, 'Changes'));
      await tester.pumpAndSettle();
      expect(inCard(find.byKey(const Key('changes-row'))), findsNothing);
      expect(
        find.text('Reset to default'),
        findsOneWidget,
        reason: 'still open',
      );
      expect(
        tester
            .widget<CheckboxMenuButton>(
              find.widgetWithText(CheckboxMenuButton, 'Changes'),
            )
            .value,
        isFalse,
      );

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.text('Reset to default'), findsNothing);
    },
  );

  testWidgets('a press outside the rows menu closes it', (tester) async {
    await show(tester);
    await rightClickCard(tester);
    await tester.tapAt(chatColumn(tester).topLeft + const Offset(60, 240));
    await tester.pumpAndSettle();
    expect(find.text('Reset to default'), findsNothing);
  });

  testWidgets('Reset to default closes the rows menu', (tester) async {
    await show(tester);
    await rightClickCard(tester);
    await tester.tap(find.widgetWithText(CheckboxMenuButton, 'Changes'));
    await tester.pumpAndSettle();
    await rightClickCard(tester);
    await tester.tap(find.text('Reset to default'));
    await tester.pumpAndSettle();
    expect(find.text('Reset to default'), findsNothing);
    expect(inCard(find.byKey(const Key('changes-row'))), findsOneWidget);
  });

  testWidgets(
    'right-clicking the branch copies its name instead of opening the rows menu',
    (tester) async {
      final copied = captureClipboard(tester);
      await show(
        tester,
        setup: (backend) =>
            backend.git = gitStatusOf(branch: 'fix/login-limit'),
      );
      await tester.tap(
        inCard(find.byKey(const Key('branch-name'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.text('Reset to default'), findsNothing);
      await tester.tap(find.text('Copy branch name'));
      await tester.pumpAndSettle();
      expect(copied, ['fix/login-limit']);
    },
  );

  testWidgets(
    'Copy in an answer’s menu copies the answer, above Branch from here',
    (tester) async {
      final copied = captureClipboard(tester);
      await showAnswer(tester);
      await tester.tap(
        find.descendant(
          of: find.byType(ChatView),
          matching: find.byIcon(LucideIcons.ellipsis),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        tester.getTopLeft(find.text('Copy')).dy,
        lessThan(tester.getTopLeft(find.text('Branch from here')).dy),
      );
      await tester.tap(find.text('Copy'));
      await tester.pumpAndSettle();
      expect(copied, ['Done']);
    },
  );

  testWidgets(
    'the Diff panel’s left edge resizes it within its limits, and the width is kept',
    (tester) async {
      final diffSettings = DiffSettingsController();
      await show(
        tester,
        windowSize: const Size(1200, 900),
        diffSettings: diffSettings,
      );
      await press(tester, LogicalKeyboardKey.keyD);
      double width() => tester.getSize(find.byType(DiffPanel)).width;
      final window = MediaQuery.sizeOf(tester.element(find.byType(DiffPanel)))
          .width;
      expect(width(), 540, reason: 'T3 Code’s right panel default');

      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(-10, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), 550);
      expect(diffSettings.value.width, 550);

      // Widest: the window's 70 %, as T3 Code's right panel allows.
      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(-3000, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), closeTo(window * 0.7, 0.5));

      // Narrowest: T3 Code's right panel minimum, which the Diff header fits.
      await tester.drag(
        find.byKey(const Key('diff-resize')),
        const Offset(3000, 0),
      );
      await tester.pumpAndSettle();
      expect(width(), 360);
    },
  );
}

/// Whether keyboard focus is inside the widget with `key`.
bool focusIsIn(Key key) {
  var inside = false;
  FocusManager.instance.primaryFocus?.context?.visitAncestorElements((element) {
    inside = element.widget.key == key;
    return !inside;
  });
  return inside;
}
