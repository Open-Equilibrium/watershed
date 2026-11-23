import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

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
      r'(?:TextEditingController|MathText)\??\s+(?:get\s+)?(\w+)|(\w+)\s*=\s*(?:TextEditingController|MathText)[.(]',
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
        'lib/src/workspace/cell_pickers.dart',
        '_text.value = TextEditingValue(',
        'Date arrow keys: a person\'s own change, which stays undoable.',
      ),
      (
        'lib/src/workspace/stored_name.dart',
        '_controller.value = TextEditingValue(',
        'handOver: the name taken from the line that became the Table, in its first frame; no earlier text is on its undo history.',
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
        'lib/src/workspace/code_block.dart',
        '_text.value = value;',
        'Code Block ⏎, ⇥ and ⇧⇥: a person\'s own change, which stays undoable.',
      ),
      (
        'lib/src/workspace/text_block.dart',
        '_text.value = value.copyWith(',
        'Text Block ⌘V: pasted text, a person\'s own change, which stays undoable.',
      ),
      (
        'lib/src/whiteboard/board_surface.dart',
        '_typed.text = element.text;',
        'A Whiteboard text box\'s editor, filled as it opens; it is built only while open.',
      ),
      (
        'lib/src/workspace/code_block.dart',
        '_search.clear();',
        'The language search, emptied as its picker opens; the field is built only while open.',
      ),
      (
        'lib/src/workspace/row_popover.dart',
        '_pickText.text = typed;',
        'A row popover\'s picker text, emptied as the picker opens; the picker is built only while open.',
      ),
      (
        'lib/src/workspace/table_tabs.dart',
        '_name.value = TextEditingValue(',
        'The tab\'s name as Rename starts; the field is built only while renaming, so its history starts there.',
      ),
      (
        'lib/src/controls.dart',
        'controller.value = value;',
        'writeText itself: the one place Liquid writes text into a field.',
      ),
    ];
    final listed = {for (final (file, line, _) in allowed) '$file: $line'};
    expect(
      found.difference(listed),
      isEmpty,
      reason: 'route these writes through writeText or list them with a reason',
    );
    expect(
      listed.difference(found),
      isEmpty,
      reason: 'remove allow-list entries that no longer match a write',
    );
  });
}
