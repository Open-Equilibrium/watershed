import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/theme/icons.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

const worktree = '/repo/.worktrees/fix-login';

void main() {
  testWidgets(
    'the environment row names the host, and its folder icon gives the worktree path',
    (tester) async {
      await pumpCard(tester, session: FakeSession());
      expect(find.byKey(const Key('workspace-row')), findsOneWidget);
      expect(find.text('Mac mini'), findsOneWidget);

      final pointer = await tester.createGesture(kind: PointerDeviceKind.mouse);
      addTearDown(pointer.removePointer);
      await pointer.addPointer(location: Offset.zero);
      await pointer.moveTo(
        tester.getCenter(find.byIcon(LucideIcons.folderGit)),
      );
      await tester.pump(const Duration(seconds: 2));
      expect(find.text(worktree), findsOneWidget);
    },
  );

  testWidgets('a right-click copies the full path and reports it', (
    tester,
  ) async {
    final copied = recordClipboard(tester);
    final state = await pumpCard(tester, session: FakeSession());
    await tester.tapAt(
      tester.getCenter(find.byKey(const Key('workspace-row'))),
      buttons: kSecondaryButton,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Copy full path'));
    await tester.pumpAndSettle();
    expect(copied(), worktree);
    expect(state.notice.value?.text, 'Path copied: $worktree');
  });

  testWidgets(
    'a card too short for the full rows leaves the environment row out',
    (tester) async {
      await pumpCard(tester, session: FakeSession(), height: 150);
      expect(find.byKey(const Key('workspace-row')), findsNothing);
    },
  );
}
