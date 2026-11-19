import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/app.dart';
import 'package:liquid/src/launch.dart';
import 'package:liquid/src/workspace/workspace_state.dart';
import 'package:liquid/src/workspace/workspaces.dart';

import 'fake_workspace.dart';

void main() {
  late Map<String, FakeWorkspace> opened;

  Future<Workspaces> show(WidgetTester tester, {bool failNew = false}) async {
    await tester.binding.setSurfaceSize(const Size(1400, 900));
    opened = {};
    final workspaces = await Workspaces.load('/workspaces', (path) async {
      if (failNew && path.endsWith('/Workspace') == false) {
        return (null, 'The Workspace did not open: no key');
      }
      final fake = opened.putIfAbsent(path.split('/').last, FakeWorkspace.new);
      fake.isDisposed = false;
      final state = WorkspaceState(fake);
      await state.start();
      return (state, null);
    });
    await tester.pumpWidget(
      LiquidApp(
        launch: ValueNotifier(const Launch.problem(null)),
        workspaces: workspaces,
      ),
    );
    await tester.pumpAndSettle();
    return workspaces;
  }

  Future<void> tap(WidgetTester tester, Finder target) async {
    await tester.tap(target);
    for (var turn = 0; turn < 20; turn++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 2)),
      );
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.pumpAndSettle();
  }

  testWidgets('the Workspace you are in is renamed and given an icon', (
    tester,
  ) async {
    await show(tester);
    final fake = opened['Workspace']!;
    expect(find.text('Workspace'), findsOneWidget);
    await tap(tester, find.byKey(const Key('current workspace')));
    await tester.enterText(find.byKey(const Key('workspace name')), 'Home');
    await tester.pumpAndSettle();
    expect(fake.name, 'Home');
    await tap(
      tester,
      find.descendant(
        of: find.byKey(const Key('workspace menu')),
        matching: find.text('🏡'),
      ),
    );
    expect(fake.icon, '🏡');
    await tap(tester, find.text('Use the first letter'));
    expect(fake.icon, '');
  });

  testWidgets('＋ makes a Workspace to name; a click or ⌥⌘2 switches', (
    tester,
  ) async {
    final workspaces = await show(tester);
    await tap(tester, find.byKey(const Key('new workspace')));
    expect(workspaces.at, 1);
    expect(opened['Workspace']!.isDisposed, isTrue);
    expect(find.byKey(const Key('workspace menu')), findsOneWidget);
    expect(
      tester
          .widget<TextField>(find.byKey(const Key('workspace name')))
          .controller!
          .text,
      newWorkspaceName,
    );
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('workspace menu')), findsNothing);
    await tap(tester, find.byKey(const Key('workspace 0')));
    expect(workspaces.at, 0);
    expect(find.byKey(const Key('workspace menu')), findsNothing);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.meta);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit1);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.meta);
    await tester.pumpAndSettle();
    expect(workspaces.at, 0, reason: '⌘1 stays the Chat\'s');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.meta);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.alt);
    await tester.sendKeyEvent(LogicalKeyboardKey.digit2);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.alt);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.meta);
    for (var turn = 0; turn < 20; turn++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 2)),
      );
      await tester.pump(const Duration(milliseconds: 50));
    }
    expect(workspaces.at, 1);
  });

  testWidgets('a Workspace that did not open still lets another be chosen', (
    tester,
  ) async {
    final workspaces = await show(tester, failNew: true);
    await tap(tester, find.byKey(const Key('new workspace')));
    expect(workspaces.at, 1);
    expect(workspaces.current, isNull);
    expect(find.text('The Workspace did not open: no key'), findsOneWidget);
    await tap(tester, find.byKey(const Key('workspace 0')));
    expect(workspaces.at, 0);
    expect(workspaces.current, isNotNull);
  });
}
