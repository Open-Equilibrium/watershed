import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/database.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'table_cells.dart';

/// The color a new option of `field` takes: the one after its last option's, wrapping
/// after the ten tag colors (approved Table View mockups, section 8).
int nextColor(BridgeField field) =>
    ((field.options.lastOrNull?.color ?? -1) + 1) % 10;

/// The options a select cell's picker offers for what is typed: those whose name holds it,
/// ignoring case.
List<BridgeOption> optionsFor(BridgeField field, String typed) => [
  for (final option in field.options)
    if (option.name.toLowerCase().contains(typed.trim().toLowerCase())) option,
];

/// A select cell's picker (approved Table View mockups, section 4): typing filters the
/// options, ↑↓ choose, ⏎ or a click picks one, or creates the typed name in the next color
/// when no option holds it; esc closes it and keeps the value, ⇥ keeps it and goes on.
class SelectPicker extends StatefulWidget {
  const SelectPicker({
    super.key,
    required this.field,
    required this.text,
    required this.onPick,
    required this.onCreate,
    required this.onClose,
    required this.onTab,
    this.onKeys,
  });

  final BridgeField field;

  /// What is typed to filter it, kept by the selection so keys typed before the picker
  /// takes them count too.
  final TextEditingController text;
  final ValueChanged<BridgeOption> onPick;
  final ValueChanged<String> onCreate;
  final VoidCallback onClose;

  /// ⇥ or, going back, ⇧⇥.
  final ValueChanged<bool> onTab;

  /// Takes the picker's keys while it is open, for keys that reach the cell first.
  final void Function(KeyEventResult Function(KeyEvent event)? keys)? onKeys;

  @override
  State<SelectPicker> createState() => _SelectPickerState();
}

class _SelectPickerState extends State<SelectPicker> {
  TextEditingController get _text => widget.text;
  late final _focus = FocusNode(debugLabel: 'Select picker', onKeyEvent: _key);
  var _chosen = 0;

  @override
  void initState() {
    super.initState();
    _text.addListener(_filtered);
    widget.onKeys?.call((event) => _key(_focus, event));
    // The filter takes the keys, also while the cell had them.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  void _filtered() => setState(() => _chosen = 0);

  @override
  void dispose() {
    _text.removeListener(_filtered);
    widget.onKeys?.call(null);
    _focus.dispose();
    super.dispose();
  }

  String get _typed => _text.text.trim();

  List<BridgeOption> get _options => optionsFor(widget.field, _typed);

  /// Whether the typed name is one no option has, to create.
  bool get _creates =>
      _typed.isNotEmpty &&
      !widget.field.options.any(
        (option) => option.name.toLowerCase() == _typed.toLowerCase(),
      );

  int get _count => _options.length + (_creates ? 1 : 0);

  void _take(int index) {
    final options = _options;
    if (index < options.length) {
      widget.onPick(options[index]);
    } else if (_creates) {
      widget.onCreate(_typed);
    }
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !_text.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        if (_count == 0) break;
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(() => _chosen = (_chosen + by) % _count);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is! KeyRepeatEvent) _take(_chosen);
      case LogicalKeyboardKey.escape:
        widget.onClose();
      case LogicalKeyboardKey.tab:
        widget.onTab(HardwareKeyboard.instance.isShiftPressed);
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final options = _options;
    Widget item(int index, Widget child) => MouseRegion(
      onEnter: (_) => setState(() => _chosen = index),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => _take(index),
        child: Container(
          height: 30,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          alignment: Alignment.centerLeft,
          decoration: BoxDecoration(
            color: index == _chosen ? ws.hover : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: child,
        ),
      ),
    );
    return Container(
      key: const Key('select picker'),
      width: 240,
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      clipBehavior: Clip.antiAlias,
      child: DefaultTextStyle.merge(
        style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              constraints: const BoxConstraints(minHeight: 34),
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.subtle,
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: TextField(
                controller: _text,
                focusNode: _focus,
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: const InputDecoration.collapsed(hintText: null),
              ),
            ),
            Padding(
              padding: const EdgeInsets.all(WsMetrics.space2),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.fromLTRB(
                      WsMetrics.space3,
                      WsMetrics.space1,
                      WsMetrics.space3,
                      2,
                    ),
                    child: Text(
                      'Choose an option or create one',
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.textFaint,
                      ),
                    ),
                  ),
                  for (var index = 0; index < options.length; index++)
                    item(
                      index,
                      Align(
                        alignment: Alignment.centerLeft,
                        child: OptionTag(options[index]),
                      ),
                    ),
                  if (_creates)
                    item(
                      options.length,
                      Row(
                        children: [
                          const Text('Create'),
                          const SizedBox(width: WsMetrics.space2),
                          Flexible(
                            child: OptionTag(
                              BridgeOption(
                                id: '',
                                name: _typed,
                                color: nextColor(widget.field),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
