import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// Text Liquid assigns to an editable field goes through `renewController` (controls.dart), so ⌘Z
/// cannot bring back the text it replaced. Other writes to a text controller are a person's own
/// or a field's first text, and each one is listed here with its reason.
///
/// A regex over source lines, not a parser: it finds `x.text =`, `x.value =` and `x.clear()` where
/// `x` is declared in the same file as a TextEditingController (a field, parameter or getter). A
/// line starting with `//` is skipped.
void main() {
  test('every write to a text controller in lib/ is listed with a reason', () {
    final sources = {
      for (final file in Directory('lib').listSync(recursive: true))
        if (file is File &&
            file.path.endsWith('.dart') &&
            !file.path.contains('/rust/'))
          file.path: file.readAsLinesSync(),
    };
    final declared = RegExp(
      r'TextEditingController\??\s+(?:get\s+)?(\w+)|(\w+)\s*=\s*TextEditingController[.(]',
    );
    final write = RegExp(
      r'(\w+)\.(?:text\s*=(?![=>])|value\s*=(?![=>])|clear\(\))',
    );
    final found = <String>{};
    for (final MapEntry(key: file, value: source) in sources.entries) {
      final controllers = {
        for (final line in source)
          for (final match in declared.allMatches(line))
            (match[1] ?? match[2])!,
      };
      for (final line in source) {
        final code = line.trim();
        if (code.startsWith('//')) continue;
        if (write.allMatches(code).any((m) => controllers.contains(m[1]))) {
          found.add('$file: $code');
        }
      }
    }

    const allowed = <(String, String, String)>[
      (
        'lib/src/workspace/table_selection.dart',
        'pickText.value = TextEditingValue(',
        'Picker text, set when the picker opens (its field is built only while open) and by typing relays.',
      ),
      (
        'lib/src/workspace/table_selection.dart',
        'edit.value = TextEditingValue(',
        'Cell editor text, set when the editor opens (its field is built only while open) and by typing relays.',
      ),
      (
        'lib/src/workspace/page_view.dart',
        "_title.text = _page?.title ?? '';",
        'The first title, before its field is built.',
      ),
      (
        'lib/src/workspace/cell_pickers.dart',
        '_text.value = TextEditingValue(',
        'Date arrow keys: a person\'s own change, which stays undoable.',
      ),
      (
        'lib/src/trajectory/filter_bar.dart',
        'text.clear();',
        'The filter\'s clear button: a person\'s own change, which stays undoable.',
      ),
      (
        'lib/src/workspace/table_block.dart',
        '_name.value = TextEditingValue(',
        'The name taken from the line that became the Table, in its first frame; no earlier text is on its undo history.',
      ),
      (
        'lib/src/workspace/field_menu.dart',
        'name.value = TextEditingValue(',
        'The name shown when the menu opens; the menu\'s field is built on each open, so its history starts there.',
      ),
      (
        'lib/src/workspace/page_view.dart',
        '_first.clear();',
        'The first line once its Text Block takes over; the line is no longer built, so ⌘Z cannot reach it.',
      ),
      (
        'lib/src/agent/pull_request_writing.dart',
        '_titleField.text = _filledTitle = title;',
        'The first fill from the commit, before the form is built.',
      ),
      (
        'lib/src/agent/pull_request_writing.dart',
        '_bodyField.text = _filledBody = body;',
        'The first fill from the commit, before the form is built.',
      ),
      (
        'lib/src/controls.dart',
        'previous.value = value;',
        'renewController itself: the old history records the new text, which the fresh one starts from.',
      ),
      // Pending until their fix commits land.
      (
        'lib/src/agent/action_area.dart',
        'controller.clear();',
        'Pending: the composer clears after a send.',
      ),
      (
        'lib/src/agent/shell.dart',
        '_composer.value = TextEditingValue(',
        'Pending: text handed back to the composer.',
      ),
      (
        'lib/src/workspace/table_block.dart',
        'if (_name.text != name) _name.text = name;',
        'Pending: a name changed elsewhere.',
      ),
    ];
    final listed = {for (final (file, line, _) in allowed) '$file: $line'};
    expect(
      found.difference(listed),
      isEmpty,
      reason: 'renew these writes or list them with a reason',
    );
    expect(
      listed.difference(found),
      isEmpty,
      reason: 'remove allow-list entries that no longer match a write',
    );
  });
}
