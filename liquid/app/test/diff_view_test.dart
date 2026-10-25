import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_view.dart';
import 'package:liquid/src/rust/api/events.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';

import 'fake_backend.dart';

void main() {
  testWidgets(
    'lists changed files as folders and shows the selected file\'s diff',
    (tester) async {
      await tester.binding.setSurfaceSize(const Size(1600, 900));
      final backend = FakeBackend()
        ..changed = [
          BridgeFileChange(
            path: 'src/routes/login.ts',
            status: BridgeChangeStatus.modified,
            added: 1,
            removed: 1,
          ),
          BridgeFileChange(
            path: 'src/middleware/rateLimit.ts',
            status: BridgeChangeStatus.added,
            added: 2,
            removed: 0,
          ),
        ]
        ..patches = {
          'src/middleware/rateLimit.ts':
              '@@ -0,0 +1,2 @@\n+export const limit = 5;\n+// per minute\n',
          'src/routes/login.ts':
              '@@ -1,2 +1,2 @@\n-const old = 1;\n+const fresh = 2;\n same();\n',
        };
      final state = SessionState(backend);
      await tester.pumpWidget(
        MaterialApp(
          theme: liquidTheme(Brightness.light),
          home: Material(child: DiffView(state: state)),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('2 files · +3 −1'), findsOneWidget);
      for (final name in [
        'src',
        'middleware',
        'rateLimit.ts',
        'routes',
        'login.ts',
      ]) {
        expect(find.text(name), findsOneWidget);
      }
      expect(
        find.text('src/middleware/rateLimit.ts'),
        findsOneWidget,
        reason: 'the first file opens',
      );
      expect(
        find.textContaining('export const limit = 5;', findRichText: true),
        findsOneWidget,
      );

      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.pumpAndSettle();
      expect(find.text('src/routes/login.ts'), findsOneWidget);
      expect(
        find.textContaining('const fresh = 2;', findRichText: true),
        findsOneWidget,
      );
      expect(
        find.textContaining('const old = 1;', findRichText: true),
        findsOneWidget,
      );
    },
  );

  testWidgets('says when a run changed nothing', (tester) async {
    final state = SessionState(FakeBackend());
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Material(child: DiffView(state: state)),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('No changes in this run'), findsOneWidget);
  });

  testWidgets('says why a diff cannot be shown', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1600, 900));
    final backend = FakeBackend()
      ..changed = [
        BridgeFileChange(
          path: 'big.txt',
          status: BridgeChangeStatus.modified,
          added: 200000,
          removed: 0,
        ),
      ]
      ..patchError = 'the diff is larger than 4 MiB';
    final state = SessionState(backend);
    await tester.pumpWidget(
      MaterialApp(
        theme: liquidTheme(Brightness.light),
        home: Material(child: DiffView(state: state)),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      (state.notice.value?.level, state.notice.value?.text),
      (
        BridgeLevel.error,
        'Could not show the changes: the diff is larger than 4 MiB',
      ),
    );
  });
}
