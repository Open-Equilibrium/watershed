part of 'automation_editor.dart';

extension _ValueEditing on _AutomationEditorState {
  TextEditingController _text(String key, String stored) {
    final text = _texts.putIfAbsent(key, TextEditingController.new);
    if (!_focused.contains(key) && text.text != stored) {
      writeText(text, TextEditingValue(text: stored));
    }
    return text;
  }

  BridgeCell? _typed(BridgeField field, String typed) => switch (field.kind) {
    BridgeFieldKind.number => switch (double.tryParse(
      typed.trim().replaceAll(',', '.'),
    )) {
      final number? => _cell(BridgeCellKind.number, number: number),
      null => null,
    },
    BridgeFieldKind.date => switch (DateTime.tryParse(typed.trim())) {
      final day? when _day.format(day) == typed.trim() => _cell(
        BridgeCellKind.date,
        text: _day.format(day),
      ),
      _ => null,
    },
    _ => _cell(BridgeCellKind.text, text: typed),
  };

  String _cellText(BridgeCell? cell) => switch (cell?.kind) {
    BridgeCellKind.number when cell!.number == cell.number.truncateToDouble() =>
      cell.number.isFinite
          ? BigInt.from(cell.number).toString()
          : cell.number.toString(),
    BridgeCellKind.number => cell!.number.toString(),
    BridgeCellKind.text || BridgeCellKind.date => cell!.text ?? '',
    _ => '',
  };

  Widget _value(
    String key,
    BridgeField? field,
    BridgeCell? value,
    ValueChanged<BridgeCell> onSet, {
    bool clear = false,
  }) {
    if (field == null) return const SizedBox.shrink();
    switch (field.kind) {
      case BridgeFieldKind.select:
        return Token(
          name: key,
          label:
              field.options
                  .where((option) => option.id == value?.text)
                  .firstOrNull
                  ?.name ??
              (clear ? 'empty' : 'an option'),
          choices: [
            if (clear)
              (
                'empty',
                value?.kind == BridgeCellKind.empty,
                () => onSet(_empty),
              ),
            for (final option in field.options)
              (
                option.name,
                option.id == value?.text,
                () => onSet(_cell(BridgeCellKind.select, text: option.id)),
              ),
          ],
        );
      case BridgeFieldKind.checkbox:
        return Token(
          name: key,
          label: value?.checked ?? false ? 'checked' : 'not checked',
          choices: [
            (
              'checked',
              value?.checked ?? false,
              () => onSet(_cell(BridgeCellKind.checkbox, checked: true)),
            ),
            (
              'not checked',
              !(value?.checked ?? false),
              () => onSet(_cell(BridgeCellKind.checkbox, checked: false)),
            ),
          ],
        );
      default:
        final text = _text(key, _cellText(value));
        void submit(String typed) {
          if (typed == _cellText(value)) return;
          if (clear && typed.trim().isEmpty) return onSet(_empty);
          final cell = _typed(field, typed);
          if (cell == null) {
            _state.report(
              field.kind == BridgeFieldKind.date
                  ? 'Type a date as 2026-11-30.'
                  : 'Type a number.',
            );
            return;
          }
          onSet(cell);
        }

        return TokenField(
          key: Key(key),
          controller: text,
          hint: switch (field.kind) {
            BridgeFieldKind.number => '0',
            BridgeFieldKind.date => '2026-11-30',
            _ => clear ? 'empty' : 'text',
          },
          onFocus: (focused) {
            focused ? _focused.add(key) : _focused.remove(key);
            if (!focused) submit(text.text);
          },
          onSubmitted: submit,
        );
    }
  }
}
