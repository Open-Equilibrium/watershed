import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card.dart';
import 'package:liquid/src/agent/session_card_settings.dart';
import 'package:liquid/src/agent/session_menus.dart' show runOpener;
import 'package:liquid/src/diff/turn_scope.dart';
import 'package:liquid/src/session/session_state.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/theme/tokens.g.dart';

import 'fake_backend.dart';

/// Opens thread-1 on `session`, in `worktree`, and pumps the session card beside the chat, `height`
/// high, as the card's tests and the sections' tests see it.
Future<SessionState> pumpCard(
  WidgetTester tester, {
  required FakeSession session,
  SessionCardSettings settings = const SessionCardSettings(),
  double height = 600,
  bool beside = true,
  ValueChanged<DiffScope>? onChanges,
  VoidCallback? onOutsidePress,
  String? worktree = '/repo/.worktrees/fix-login',
}) async {
  final project = FakeProject(session: session)
    ..threadList = [threadOf('thread-1', worktree: worktree)];
  final state = SessionState(project);
  addTearDown(state.dispose);
  await state.open('thread-1');
  await state.loadStart();
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
              settings: SessionCardSettingsController(settings),
              onChanges: onChanges ?? (_) {},
              beside: beside,
              onOutsidePress: onOutsidePress ?? () {},
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return state;
}

/// Records what the app copies to the clipboard; the getter returns the last copy.
String? Function() recordClipboard(WidgetTester tester) {
  String? copied;
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    SystemChannels.platform,
    (call) async {
      if (call.method == 'Clipboard.setData') {
        copied = (call.arguments as Map)['text'] as String?;
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
  return () => copied;
}

/// Records what the browser opener is asked to open, instead of opening it.
List<String> recordOpens() {
  final opened = <String>[];
  final original = runOpener;
  runOpener = (program, arguments) async {
    opened.add(arguments.single);
    return ProcessResult(0, 0, '', '');
  };
  addTearDown(() => runOpener = original);
  return opened;
}

/// A colour of the light theme's palette, picked by `pick`.
Color toneOf(Color Function(WsPalette palette) pick) =>
    pick(liquidTheme(Brightness.light).extension<WsTheme>()!.palette);
