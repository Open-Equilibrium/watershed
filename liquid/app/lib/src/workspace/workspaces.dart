import 'dart:async';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../settings_file.dart';
import 'workspace_state.dart';

typedef WorkspaceEntry = ({String dir, String name, String icon});

typedef WorkspaceOpener = Future<(WorkspaceState?, String?)> Function(
  String dir,
);

const newWorkspaceName = 'New Workspace';

final _folder = RegExp(r'^Workspace(-[0-9a-f]{32})?$');

class Workspaces extends ChangeNotifier {
  Workspaces._(this.root, this._file, this._open, this._entries);

  final String root;
  final SettingsFile? _file;
  final WorkspaceOpener _open;

  List<WorkspaceEntry> _entries;
  List<WorkspaceEntry> get entries => List.unmodifiable(_entries);

  int _at = 0;
  int get at => _at;

  WorkspaceState? _current;
  WorkspaceState? get current => _current;

  String? _problem;
  String? get problem => _problem;

  Future<void> _work = Future.value();

  bool _fresh = false;

  bool takeFresh() {
    final fresh = _fresh;
    _fresh = false;
    return fresh;
  }

  static Future<Workspaces> load(
    String root,
    WorkspaceOpener open, {
    SettingsFile? file,
  }) async {
    final json = await file?.load();
    final entries = <WorkspaceEntry>[];
    if (json?['list'] case final List<Object?> list) {
      for (final item in list) {
        if (item
            case {
              'dir': final String dir,
              'name': final String name,
              'icon': final String icon,
            }
            when _folder.hasMatch(dir) && !entries.any((e) => e.dir == dir)) {
          entries.add((dir: dir, name: name, icon: icon));
        }
      }
    }
    if (entries.isEmpty) entries.add((dir: 'Workspace', name: '', icon: ''));
    final opened = json?['open'];
    final at = entries.indexWhere((entry) => entry.dir == opened);
    final workspaces = Workspaces._(root, file, open, entries);
    await workspaces._openAt(at < 0 ? 0 : at);
    return workspaces;
  }

  Future<void> switchTo(int index) => _queued(() async {
    if (index == _at || index < 0 || index >= _entries.length) return;
    await _close();
    await _openAt(index);
  });

  Future<void> create() => _queued(() async {
    final bytes = List.generate(16, (_) => Random.secure().nextInt(256));
    final dir =
        'Workspace-${bytes.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join()}';
    _entries = [..._entries, (dir: dir, name: newWorkspaceName, icon: '')];
    _fresh = true;
    await _close();
    await _openAt(_entries.length - 1);
    final state = _current;
    if (state != null) {
      try {
        await state.renameWorkspace(newWorkspaceName);
      } on Object catch (_) {}
    }
  });

  Future<void> _queued(Future<void> Function() step) {
    final next = _work.then((_) => step());
    _work = next.then<void>((_) {}, onError: (Object _) {});
    return next;
  }

  Future<void> _openAt(int index) async {
    final (state, problem) = await _open('$root/${_entries[index].dir}');
    _at = index;
    _current = state;
    _problem = problem;
    state?.named.addListener(_named);
    notifyListeners();
    _save();
  }

  Future<void> _close() async {
    final state = _current;
    if (state == null) return;
    state.named.removeListener(_named);
    _current = null;
    notifyListeners();
    await state.drain();
    await state.dispose();
    state.workspace.dispose();
  }

  void _named() {
    final named = _current?.named.value;
    if (named == null) return;
    final entry = _entries[_at];
    if (entry.name == named.name && entry.icon == named.icon) return;
    _entries = [
      for (final (index, shown) in _entries.indexed)
        index == _at
            ? (dir: shown.dir, name: named.name, icon: named.icon)
            : shown,
    ];
    notifyListeners();
    _save();
  }

  void _save() => unawaited(
    _file
        ?.save({
          'open': _entries[_at].dir,
          'list': [
            for (final entry in _entries)
              {'dir': entry.dir, 'name': entry.name, 'icon': entry.icon},
          ],
        })
        .then<void>((_) {}, onError: (Object _) {}),
  );
}
