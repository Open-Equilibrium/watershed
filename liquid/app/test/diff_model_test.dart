import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/diff/diff_model.dart';
import 'package:liquid/src/rust/api/events.dart';

BridgeFileChange change(
  String path,
  BridgeChangeStatus status,
  int added,
  int removed,
) => BridgeFileChange(
  path: path,
  status: status,
  added: added,
  removed: removed,
);

void main() {
  test('groups changed files into folders, folders first', () {
    final rows = fileTree([
      change('test/login.test.ts', BridgeChangeStatus.modified, 12, 3),
      change('src/routes/login.ts', BridgeChangeStatus.modified, 9, 2),
      change('README.md', BridgeChangeStatus.modified, 1, 0),
      change('src/middleware/rateLimit.ts', BridgeChangeStatus.added, 38, 0),
    ]);
    expect(
      rows.map(
        (row) =>
            '${'  ' * row.depth}${row.name}${row.change == null ? '/' : ''}',
      ),
      [
        'src/',
        '  middleware/',
        '    rateLimit.ts',
        '  routes/',
        '    login.ts',
        'test/',
        '  login.test.ts',
        'README.md',
      ],
    );
    expect(rows[2].change!.path, 'src/middleware/rateLimit.ts');
  });

  test('numbers the lines of a unified diff', () {
    final lines = parsePatch('''diff --git a/a.ts b/a.ts
index 1..2 100644
--- a/a.ts
+++ b/a.ts
@@ -1,3 +1,4 @@ export
 import x
-old
+new
+added
 same
''');
    expect(
      lines.map(
        (line) => (line.kind, line.oldNumber, line.newNumber, line.text),
      ),
      [
        (DiffKind.hunk, null, null, '@@ -1,3 +1,4 @@ export'),
        (DiffKind.context, 1, 1, 'import x'),
        (DiffKind.removed, 2, null, 'old'),
        (DiffKind.added, null, 2, 'new'),
        (DiffKind.added, null, 3, 'added'),
        (DiffKind.context, 3, 4, 'same'),
      ],
    );
  });
}
