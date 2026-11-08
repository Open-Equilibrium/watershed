import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
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

  static const _filterHeight = 34.0;
  static const _labelHeight = 22.0;
  static const _itemHeight = 30.0;

  /// The height the picker takes with `options` options and nothing typed.
  static double heightFor(int options) =>
      _filterHeight +
      2 * WsMetrics.space2 +
      _labelHeight +
      options * _itemHeight;

  @override
  State<SelectPicker> createState() => _SelectPickerState();
}

class _SelectPickerState extends State<SelectPicker> {
  TextEditingController get _text => widget.text;
  late final _focus = FocusNode(debugLabel: 'Select picker', onKeyEvent: _key);
  final _list = ScrollController();
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

  void _filtered() => _choose(0);

  /// Chooses the option at `index` and scrolls the list to keep it in view.
  void _choose(int index) {
    setState(() => _chosen = index);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || !_list.hasClients) return;
      final position = _list.position;
      final top = _chosen * SelectPicker._itemHeight;
      final bottom =
          top + SelectPicker._itemHeight - position.viewportDimension;
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
          height: SelectPicker._itemHeight,
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
              height: SelectPicker._filterHeight,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                color: ws.subtle,
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: TextField(
                controller: _text,
                focusNode: _focus,
                // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
                selectAllOnFocus: false,
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
                    Container(
                      height: SelectPicker._labelHeight,
                      padding: const EdgeInsets.fromLTRB(
                        WsMetrics.space3,
                        WsMetrics.space1,
                        WsMetrics.space3,
                        2,
                      ),
                      alignment: Alignment.centerLeft,
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
                        itemExtent: SelectPicker._itemHeight,
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

/// How a day shows in a cell and a date picker's field, in the person's region.
DateFormat shownDays(String locale) => DateFormat.yMMMd(locale);

/// How the core stores a day (ADR-0242).
final storedDays = DateFormat('yyyy-MM-dd', 'en_US');

/// The day typed in the person's region's short form, as cells show it or as the core
/// stores it, with a year of four digits; none for anything else, such as a year of two.
DateTime? parseDay(String typed, String locale) {
  for (final format in [
    DateFormat.yMd(locale),
    shownDays(locale),
    storedDays,
  ]) {
    try {
      final day = format.parseStrict(typed.trim());
      return day.year >= 1000 && day.year <= 9999 ? day : null;
    } on FormatException {
      continue;
    } on ArgumentError {
      continue;
    }
  }
  return null;
}

/// A date cell's picker (approved Table View mockups, section 4): its field holds the day
/// as typed, a calendar below shows its month. ↑↓ choose the day after or before, ⏎, ⇥ and
/// a click outside write what is typed, a click on a day writes it, and Clear empties the
/// cell; esc closes it and keeps the value.
class DatePicker extends StatefulWidget {
  const DatePicker({
    super.key,
    required this.text,
    required this.locale,
    required this.onDay,
    required this.onEnter,
    required this.onClear,
    required this.onClose,
    required this.onTab,
    this.onKeys,
  });

  /// What is typed, kept by the selection so keys typed before the picker takes them
  /// count too.
  final TextEditingController text;

  /// The person's region, as intl names it.
  final String locale;
  final ValueChanged<DateTime> onDay;
  final VoidCallback onEnter;
  final VoidCallback onClear;
  final VoidCallback onClose;

  /// ⇥ or, going back, ⇧⇥.
  final ValueChanged<bool> onTab;

  /// Takes the picker's keys while it is open, for keys that reach the cell first.
  final void Function(KeyEventResult Function(KeyEvent event)? keys)? onKeys;

  static const _fieldHeight = 30.0;
  static const _monthHeight = 30.0;
  static const _weekdaysHeight = 22.0;
  static const _weekHeight = 30.0;
  static const _clearHeight = 30.0;

  /// The height the picker takes at most, with a month of six weeks.
  static const height =
      2 * WsMetrics.space2 +
      _fieldHeight +
      6 +
      _monthHeight +
      _weekdaysHeight +
      6 * _weekHeight +
      6 +
      WsMetrics.space2 +
      1 +
      _clearHeight;

  @override
  State<DatePicker> createState() => _DatePickerState();
}

class _DatePickerState extends State<DatePicker> {
  TextEditingController get _text => widget.text;
  late final _focus = FocusNode(debugLabel: 'Date picker', onKeyEvent: _key);

  /// The first day of the month the calendar shows.
  late DateTime _month = _monthOf(_day ?? _today);

  DateTime get _today {
    final now = DateTime.now();
    return DateTime(now.year, now.month, now.day);
  }

  static DateTime _monthOf(DateTime day) => DateTime(day.year, day.month);

  DateTime? get _day => parseDay(_text.text, widget.locale);

  @override
  void initState() {
    super.initState();
    _text.addListener(_typed);
    widget.onKeys?.call((event) => _key(_focus, event));
    // The field takes the keys, also while the cell had them.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  /// The calendar follows a typed day to its month.
  void _typed() => setState(() {
    if (_day case final day?) _month = _monthOf(day);
  });

  @override
  void dispose() {
    _text.removeListener(_typed);
    widget.onKeys?.call(null);
    _focus.dispose();
    super.dispose();
  }

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent || !_text.value.composing.isCollapsed) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        final from = _day ?? _today;
        final shown = shownDays(widget.locale)
            .format(DateTime(from.year, from.month, from.day + by));
        _text.value = TextEditingValue(
          text: shown,
          selection: TextSelection.collapsed(offset: shown.length),
        );
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is! KeyRepeatEvent) widget.onEnter();
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
    final chosen = _day;
    final symbols = DateFormat(null, widget.locale).dateSymbols;
    // 0 is Sunday, as DateTime's weekday 7 is.
    final first = MaterialLocalizations.of(context).firstDayOfWeekIndex;
    final lead = (_month.weekday % 7 - first + 7) % 7;
    final length = DateTime(_month.year, _month.month + 1, 0).day;
    final weeks = ((lead + length) / 7).ceil();
    Widget step(Key key, IconData icon, int by) => GestureDetector(
      key: key,
      behavior: HitTestBehavior.opaque,
      onTap: () =>
          setState(() => _month = DateTime(_month.year, _month.month + by)),
      child: Padding(
        padding: const EdgeInsets.all(WsMetrics.space1),
        child: Icon(icon, size: 14, color: ws.textFaint),
      ),
    );
    Widget dayCell(DateTime day) {
      final on = day == chosen;
      return _Hovered(
        key: ValueKey('day ${storedDays.format(day)}'),
        onTap: () => widget.onDay(day),
        color: on ? ws.accent : null,
        child: Text(
          '${day.day}',
          style: TextStyle(
            fontSize: WsMetrics.fsSmall,
            color: on
                ? ws.onAccent
                : day.month == _month.month
                ? ws.text
                : ws.textFaint,
          ),
        ),
      );
    }

    return Container(
      key: const Key('date picker'),
      width: 244,
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      clipBehavior: Clip.antiAlias,
      // A window too low for the whole month scrolls it.
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(WsMetrics.space2),
        child: DefaultTextStyle.merge(
          style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Container(
                height: DatePicker._fieldHeight,
                margin: const EdgeInsets.fromLTRB(2, 2, 2, WsMetrics.space1),
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space2,
                ),
                alignment: Alignment.centerLeft,
                decoration: BoxDecoration(
                  color: ws.canvas,
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  border: Border.all(color: ws.accent, width: 1.5),
                ),
                child: TextField(
                  controller: _text,
                  focusNode: _focus,
                  // The caret stays where Liquid puts it; desktop Flutter would select all on focus.
                  selectAllOnFocus: false,
                  style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                  decoration: const InputDecoration.collapsed(hintText: null),
                ),
              ),
              SizedBox(
                height: DatePicker._monthHeight,
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space2,
                  ),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          DateFormat.yMMMM(widget.locale).format(_month),
                          style: const TextStyle(
                            fontSize: WsMetrics.fsCompact,
                            fontWeight: WsMetrics.fwMedium,
                          ),
                        ),
                      ),
                      step(
                        const Key('previous month'),
                        LucideIcons.chevronLeft,
                        -1,
                      ),
                      step(
                        const Key('next month'),
                        LucideIcons.chevronRight,
                        1,
                      ),
                    ],
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(6, 0, 6, 6),
                child: Column(
                  children: [
                    SizedBox(
                      height: DatePicker._weekdaysHeight,
                      child: Row(
                        children: [
                          for (var at = 0; at < 7; at++)
                            Expanded(
                              child: Center(
                                child: Text(
                                  _weekday(
                                    symbols.STANDALONESHORTWEEKDAYS[(first +
                                            at) %
                                        7],
                                  ),
                                  style: TextStyle(
                                    fontSize: WsMetrics.fsSmall,
                                    color: ws.textFaint,
                                  ),
                                ),
                              ),
                            ),
                        ],
                      ),
                    ),
                    for (var week = 0; week < weeks; week++)
                      SizedBox(
                        height: DatePicker._weekHeight,
                        child: Row(
                          children: [
                            for (var at = 0; at < 7; at++)
                              Expanded(
                                child: Padding(
                                  padding: const EdgeInsets.all(1),
                                  child: dayCell(
                                    DateTime(
                                      _month.year,
                                      _month.month,
                                      1 - lead + week * 7 + at,
                                    ),
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                  ],
                ),
              ),
              Divider(height: 1, thickness: 1, color: ws.border),
              const SizedBox(height: WsMetrics.space2),
              _Hovered(
                onTap: widget.onClear,
                alignment: Alignment.centerLeft,
                height: DatePicker._clearHeight,
                child: Text('Clear', style: TextStyle(color: ws.textMuted)),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// A weekday's short name, at most two letters, as the mockup shows them.
  static String _weekday(String name) {
    final letters = name.replaceAll('.', '');
    return letters.length > 2 ? letters.substring(0, 2) : letters;
  }
}

/// A picker's item, which shows the hover and takes a click.
class _Hovered extends StatefulWidget {
  const _Hovered({
    super.key,
    required this.onTap,
    required this.child,
    this.color,
    this.alignment = Alignment.center,
    this.height,
  });

  final VoidCallback onTap;
  final Widget child;
  final Color? color;
  final AlignmentGeometry alignment;
  final double? height;

  @override
  State<_Hovered> createState() => _HoveredState();
}

class _HoveredState extends State<_Hovered> {
  var _hovered = false;

  @override
  Widget build(BuildContext context) => MouseRegion(
    onEnter: (_) => setState(() => _hovered = true),
    onExit: (_) => setState(() => _hovered = false),
    child: GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: widget.onTap,
      child: Container(
        height: widget.height,
        padding: widget.alignment == Alignment.center
            ? null
            : const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        alignment: widget.alignment,
        decoration: BoxDecoration(
          color: widget.color ?? (_hovered ? context.ws.hover : null),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: widget.child,
      ),
    ),
  );
}
