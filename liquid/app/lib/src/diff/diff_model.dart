import 'dart:async';
import 'dart:isolate';
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

/// The characters of a line the Diff panel shows; a longer one, such as a minified bundle's,
/// shows them and an ellipsis, so laying out one line stays cheap.
const shownChars = 1000;

/// The start of `text` the Diff panel shows, without a split character pair, and whether it is
/// cut.
({String text, bool cut}) shownText(String text) {
  if (text.length <= shownChars) return (text: text, cut: false);
  final end = text.codeUnitAt(shownChars - 1) & 0xFC00 == 0xD800
      ? shownChars - 1
      : shownChars;
  return (text: text.substring(0, end), cut: true);
}

/// The shown text of the lines likely widest: the longest by a count that takes every
/// non-ASCII character as two wide, so wide characters such as Chinese ones count as the
/// width they take. The caller lays out only these few.
List<String> widestLines(List<DiffLine> lines, {int count = 8}) {
  int columns(String text) {
    var sum = 0;
    for (var index = 0; index < text.length; index++) {
      sum += text.codeUnitAt(index) < 0x80 ? 1 : 2;
    }
    return sum;
  }

  final widest = <({String text, int columns})>[];
  for (final line in lines) {
    final shown = shownText(line.text);
    final text = shown.cut ? '${shown.text}…' : shown.text;
    final width = columns(text);
    if (widest.length == count && width <= widest.last.columns) continue;
    final at = widest.indexWhere((wide) => wide.columns < width);
    widest.insert(at < 0 ? widest.length : at, (text: text, columns: width));
    if (widest.length > count) widest.removeLast();
  }
  return [for (final wide in widest) wide.text];
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

/// Reads a patch into lines. The window passes [parsePatchInBackground]; tests pass
/// [parsePatch] itself, as fake time does not run isolates.
typedef PatchParser = FutureOr<List<DiffLine>> Function(String patch);

/// Parses [patch] on a background isolate, so a large patch (up to 4 MiB) does not stall the
/// window.
Future<List<DiffLine>> parsePatchInBackground(String patch) =>
    Isolate.run(() => parsePatch(patch));

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
