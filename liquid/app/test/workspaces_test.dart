import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/settings_file.dart';
import 'package:liquid/src/workspace/workspace_state.dart';
import 'package:liquid/src/workspace/workspaces.dart';

import 'fake_workspace.dart';

void main() {
  late Directory dir;
  late Map<String, FakeWorkspace> opened;

  setUp(() async {
    dir = await Directory.systemTemp.createTemp('liquid-workspaces');
    opened = {};
  });
  tearDown(() => dir.delete(recursive: true));

  SettingsFile file() => SettingsFile(File('${dir.path}/workspaces.json'));

  Future<(WorkspaceState?, String?)> open(String path) async {
    final fake = opened.putIfAbsent(path.split('/').last, FakeWorkspace.new);
    fake.isDisposed = false;
    final state = WorkspaceState(fake);
    await state.start();
    return (state, null);
  }

  test('the first start opens the one Workspace there is', () async {
    final workspaces = await Workspaces.load(dir.path, open, file: file());
    expect(workspaces.entries.map((entry) => entry.dir), ['Workspace']);
    expect(workspaces.current?.workspace, opened['Workspace']);
  });

  test('a new Workspace is named, opened, and the old one closed', () async {
    final workspaces = await Workspaces.load(dir.path, open, file: file());
    final first = opened['Workspace']!;
    await workspaces.create();
    expect(first.isDisposed, isTrue);
    expect(workspaces.entries, hasLength(2));
    expect(workspaces.at, 1);
    final made = workspaces.entries[1];
    expect(made.dir, matches(RegExp(r'^Workspace-[0-9a-f]{32}$')));
    expect(opened[made.dir]!.name, newWorkspaceName);
    await pumpEventQueue();
    expect(workspaces.entries[1].name, newWorkspaceName);
  });

  test('switching opens the other, and a restart opens the last one', () async {
    var workspaces = await Workspaces.load(dir.path, open, file: file());
    await workspaces.create();
    await workspaces.current!.setWorkspaceIcon('🏡');
    await pumpEventQueue();
    await workspaces.switchTo(0);
    expect(workspaces.current?.workspace, opened['Workspace']);
    expect(opened[workspaces.entries[1].dir]!.isDisposed, isTrue);
    expect(workspaces.entries[1].icon, '🏡');
    await workspaces.switchTo(1);
    await pumpEventQueue();
    workspaces = await Workspaces.load(dir.path, open, file: file());
    expect(workspaces.at, 1);
    expect(workspaces.entries.map((entry) => (entry.name, entry.icon)), [
      ('', ''),
      (newWorkspaceName, '🏡'),
    ]);
  });

  test('a listed folder that is no Workspace folder is ignored', () async {
    await file().save({
      'open': '../elsewhere',
      'list': [
        {'dir': '../elsewhere', 'name': 'x', 'icon': ''},
      ],
    });
    final workspaces = await Workspaces.load(dir.path, open, file: file());
    expect(workspaces.entries.map((entry) => entry.dir), ['Workspace']);
  });
}
