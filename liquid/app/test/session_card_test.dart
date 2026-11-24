import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/card_agent.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

FakeSession dirtySession() {
  final git = FakeGit(
    status: gitStatusOf(
      branch: 'feature',
      defaultBranch: false,
      changes: [
        fileOf('a.dart', added: 2),
        fileOf('b.dart', added: 1, removed: 1),
      ],
      workingAdded: 3,
      workingRemoved: 1,
      pullRequest: pullRequestOf(),
    ),
  );
  return FakeSession()..gitBackend = git;
}

void main() {
  testWidgets('hiding Automations leaves no Automations section for an agent', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 1400));
    await pumpCard(
      tester,
      session: FakeSession(),
      agents: [agentOf('news-bot', thread: 'thread-1')],
      height: 1200,
    );
    expect(find.text('Automations'), findsOneWidget);

    await pumpCard(
      tester,
      session: FakeSession(),
      agents: [agentOf('news-bot', thread: 'thread-1')],
      settings: SessionCardSettings(hidden: {CardItem.automations}),
      height: 1200,
    );
    expect(find.text('Automations'), findsNothing);
    expect(find.byKey(const Key('add-routine')), findsNothing);
  });

  group('agent instructions', () {
    Future<SessionState> openAgent(
      WidgetTester tester,
      FakeProject project,
    ) async {
      final state = SessionState(project);
      addTearDown(state.dispose);
      await state.loadStart();
      await state.open('thread-1');
      await tester.pumpAndSettle();
      return state;
    }

    Future<void> showAgent(WidgetTester tester, SessionState state) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: Material(
            child: AgentSection(
              state: state,
              hosted: state.agents.open.value!,
              divided: false,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    testWidgets('an older instruction read does not overwrite a newer one', (
      tester,
    ) async {
      final project = _HeldInstructions()
        ..agentList = [agentOf('news-bot', thread: 'thread-1')];
      final state = await openAgent(tester, project);
      await showAgent(tester, state);
      state.agents.changes.value = const [];
      await tester.pump();
      expect(project.held, hasLength(2));

      project.held[1].complete('# news-bot\n\nNewer.\n');
      await tester.pumpAndSettle();
      project.held[0].complete('# news-bot\n\nOlder.\n');
      await tester.pumpAndSettle();
      expect(find.text('Newer.'), findsOneWidget);
      expect(find.text('Older.'), findsNothing);
    });

    testWidgets('a card given another state reads from that state', (
      tester,
    ) async {
      final first = _CountingInstructions()
        ..agentList = [agentOf('news-bot', thread: 'thread-1')]
        ..instructionsText = '# news-bot\n\nFirst.\n';
      final second = _CountingInstructions()
        ..agentList = [agentOf('news-bot', thread: 'thread-1')]
        ..instructionsText = '# news-bot\n\nSecond.\n';
      final firstState = await openAgent(tester, first);
      final secondState = await openAgent(tester, second);
      await showAgent(tester, firstState);
      expect(find.text('First.'), findsOneWidget);

      await showAgent(tester, secondState);
      await tester.pumpAndSettle();
      expect(find.text('Second.'), findsOneWidget);

      final reads = first.reads;
      firstState.agents.changes.value = const [];
      await tester.pumpAndSettle();
      expect(first.reads, reads, reason: 'the old state no longer drives it');
      secondState.agents.changes.value = const [];
      await tester.pumpAndSettle();
      expect(second.reads, greaterThan(1));
    });
  });
}

class _HeldInstructions extends FakeProject {
  final held = <Completer<String>>[];

  @override
  Future<String> agentInstructions(String name) {
    final answer = Completer<String>();
    held.add(answer);
    return answer.future;
  }
}

class _CountingInstructions extends FakeProject {
  int reads = 0;

  @override
  Future<String> agentInstructions(String name) async {
    reads++;
    return instructionsText;
  }
}
