import 'dart:math' as math;

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
  final _list = ScrollController();
  var _chosen = 0;

  static const _itemHeight = 30.0;

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

  void _filtered() => _choose(0);

  /// Chooses the option at `index` and scrolls the list to keep it in view.
  void _choose(int index) {
    setState(() => _chosen = index);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_list.hasClients) return;
      final position = _list.position;
      final top = _chosen * _itemHeight;
      final bottom = top + _itemHeight - position.viewportDimension;
      if (top < position.pixels) position.jumpTo(top);
      if (bottom > position.pixels) position.jumpTo(bottom);
    });
  }

  @override
  void dispose() {
    _text.removeListener(_filtered);
    widget.onKeys?.call(null);
    _focus.dispose();
    _list.dispose();
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
        _choose((_chosen + by) % _count);
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
          height: _itemHeight,
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
            // The options scroll in the height the picker has.
            Flexible(
              child: Padding(
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
                    Flexible(
                      child: ListView.builder(
                        controller: _list,
                        shrinkWrap: true,
                        itemExtent: _itemHeight,
                        itemCount: _count,
                        itemBuilder: (context, index) => index < options.length
                            ? item(
                                index,
                                Align(
                                  alignment: Alignment.centerLeft,
                                  child: OptionTag(options[index]),
                                ),
                              )
                            : item(
                                index,
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
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Places a select cell's picker under the cell, or above it when only there it fits,
/// in the window's height; the picker's list scrolls in what it gets.
class PickerLayout extends SingleChildLayoutDelegate {
  PickerLayout(this.cell);

  /// The cell, where the picker's overlay lays it out.
  final Rect cell;

  static const _gap = WsMetrics.space1;
  static const _margin = WsMetrics.space2;

  double _below(Size window) => window.height - cell.bottom - _gap - _margin;
  double get _above => cell.top - _gap - _margin;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      BoxConstraints.loose(
        Size(
          constraints.maxWidth,
          math.max(0, math.max(_below(constraints.biggest), _above)),
        ),
      );

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    final below = _below(size);
    final above = childSize.height > below && _above > below;
    return Offset(
      math.max(0, math.min(cell.left, size.width - childSize.width)),
      above ? cell.top - _gap - childSize.height : cell.bottom + _gap,
    );
  }

  @override
  bool shouldRelayout(PickerLayout old) => old.cell != cell;
}
