import 'dart:math' as math;

import '../rust/api/events.dart';

/// One row of the changed-files tree: a folder or a changed file.
class FileRow {
  const FileRow({
    required this.name,
    required this.depth,
    required this.folder,
    this.change,
  });

  final String name;
  final int depth;

  /// The folder's path, used to collapse it.
  final String folder;
  final BridgeFileChange? change;
}

/// Lists changed files as a folder tree, folders before files, each sorted by name.
List<FileRow> fileTree(List<BridgeFileChange> changes) {
  final root = _Folder('');
  for (final change in changes) {
    final parts = change.shown.split('/');
    var folder = root;
    for (final part in parts.take(parts.length - 1)) {
      folder = folder.folders.putIfAbsent(
        part,
        () => _Folder(folder.path.isEmpty ? part : '${folder.path}/$part'),
      );
    }
    folder.files[parts.last] = change;
  }
  final rows = <FileRow>[];
  void visit(_Folder folder, int depth) {
    for (final name in folder.folders.keys.toList()..sort()) {
      final child = folder.folders[name]!;
      rows.add(FileRow(name: name, depth: depth, folder: child.path));
      visit(child, depth + 1);
    }
    for (final name in folder.files.keys.toList()..sort()) {
      rows.add(
        FileRow(
          name: name,
          depth: depth,
          folder: folder.path,
          change: folder.files[name],
        ),
      );
    }
  }

  visit(root, 0);
  return rows;
}

class _Folder {
  _Folder(this.path);

  final String path;
  final folders = <String, _Folder>{};
  final files = <String, BridgeFileChange>{};
}

enum DiffKind { hunk, context, added, removed }

/// One displayed line of a patch.
class DiffLine {
  const DiffLine(this.kind, this.text, {this.oldNumber, this.newNumber});

  final DiffKind kind;
  final String text;
  final int? oldNumber;
  final int? newNumber;
}

final _hunk = RegExp(r'^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@');

/// Turns a unified diff into numbered lines, skipping the file headers.
List<DiffLine> parsePatch(String patch) {
  final lines = <DiffLine>[];
  var oldNumber = 0;
  var newNumber = 0;
  var inHunk = false;
  for (final line in patch.split('\n')) {
    // Git writes some changes of one file, such as a file becoming a link, as two sections.
    if (line.startsWith('diff --git ')) {
      inHunk = false;
      continue;
    }
    final hunk = _hunk.firstMatch(line);
    if (hunk != null) {
      inHunk = true;
      oldNumber = int.parse(hunk.group(1)!);
      newNumber = int.parse(hunk.group(2)!);
      lines.add(DiffLine(DiffKind.hunk, line));
    } else if (!inHunk || line.isEmpty || line.startsWith(r'\')) {
      continue;
    } else if (line.startsWith('+')) {
      lines.add(
        DiffLine(DiffKind.added, line.substring(1), newNumber: newNumber++),
      );
    } else if (line.startsWith('-')) {
      lines.add(
        DiffLine(DiffKind.removed, line.substring(1), oldNumber: oldNumber++),
      );
    } else {
      lines.add(
        DiffLine(
          DiffKind.context,
          line.substring(1),
          oldNumber: oldNumber++,
          newNumber: newNumber++,
        ),
      );
    }
  }
  return lines;
}

/// One row of the split view: a hunk header across both sides, or a line of the file before
/// the run beside its line after it.
class SplitRow {
  const SplitRow({this.hunk, this.left, this.right});

  final DiffLine? hunk;
  final DiffLine? left;
  final DiffLine? right;
}

/// Sets a patch's lines side by side: unchanged lines on both sides, and each run of removed
/// lines beside the added lines that follow it.
List<SplitRow> splitRows(List<DiffLine> lines) {
  final rows = <SplitRow>[];
  final removed = <DiffLine>[];
  final added = <DiffLine>[];
  void pair() {
    for (
      var index = 0;
      index < math.max(removed.length, added.length);
      index++
    ) {
      rows.add(
        SplitRow(
          left: index < removed.length ? removed[index] : null,
          right: index < added.length ? added[index] : null,
        ),
      );
    }
    removed.clear();
    added.clear();
  }

  for (final line in lines) {
    switch (line.kind) {
      case DiffKind.removed:
        if (added.isNotEmpty) pair();
        removed.add(line);
      case DiffKind.added:
        added.add(line);
      case DiffKind.hunk:
        pair();
        rows.add(SplitRow(hunk: line));
      case DiffKind.context:
        pair();
        rows.add(SplitRow(left: line, right: line));
    }
  }
  pair();
  return rows;
}
