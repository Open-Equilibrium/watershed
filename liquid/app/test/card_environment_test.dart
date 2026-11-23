import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'card_harness.dart';
import 'fake_backend.dart';

const worktree = '/repo/.worktrees/fix-login';

void main() {
  testWidgets('a right-click copies and reports the full path', (tester) async {
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
}
