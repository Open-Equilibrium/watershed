import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'cell_pickers.dart';
import 'field_panel.dart';
import 'table_cells.dart';

const _menuWidth = 280.0;
const _menuHeight = 360.0;
const _itemHeight = 30.0;

class QueryAnchor extends StatefulWidget {
  const QueryAnchor({super.key, required this.anchor, required this.menu});

  final Widget Function(VoidCallback toggle, bool open) anchor;
  final Widget Function(VoidCallback close) menu;

  @override
  State<QueryAnchor> createState() => _QueryAnchorState();
}

class _QueryAnchorState extends State<QueryAnchor> {
  final _menu = OverlayPortalController();

  void _close() {
    if (mounted && _menu.isShowing) setState(_menu.hide);
  }

  @override
  Widget build(BuildContext context) => AnchoredMenu(
    controller: _menu,
    height: _menuHeight,
    onTapOutside: (_) => _close(),
    menu: (_) => widget.menu(_close),
    child: widget.anchor(() => setState(_menu.toggle), _menu.isShowing),
  );
}

class QueryButton extends StatelessWidget {
  const QueryButton({
    super.key,
    required this.icon,
    required this.label,
    required this.on,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Tooltip(
      message: label,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          width: 28,
          height: 28,
          decoration: BoxDecoration(
            color: on ? ws.hover : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Icon(icon, size: 14, color: on ? ws.text : ws.textMuted),
        ),
      ),
    );
  }
}

class QueryChip extends StatelessWidget {
  const QueryChip({
    super.key,
    this.icon,
    required this.label,
    required this.onTap,
  });

  final IconData? icon;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        height: 24,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          color: ws.accentSubtle,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (icon case final icon?) ...[
              Icon(icon, size: 12, color: ws.accent),
              const SizedBox(width: WsMetrics.space2),
            ],
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 240),
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.accent),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

bool sameFilter(BridgeFilter? a, BridgeFilter? b) {
  if (a == null || b == null) return a == b;
  final (x, y) = (a.value, b.value);
  return a.field == b.field &&
      a.matches == b.matches &&
      x?.kind == y?.kind &&
      x?.text == y?.text &&
      x?.number == y?.number &&
      x?.checked == y?.checked;
}

String? filterLabel(
  BridgeFilter filter,
  List<BridgeField> fields,
  NumberFormat numbers,
  DateFormat days,
) {
  final field = fields.where((field) => field.id == filter.field).firstOrNull;
  if (field == null) return null;
  final value = filter.value;
  final name = field.name;
  return switch (filter.matches) {
    BridgeMatch.empty => '$name is empty',
    BridgeMatch.notEmpty => '$name is not empty',
    BridgeMatch.contains => '$name contains ${value?.text ?? ''}',
    BridgeMatch.between => '$name from ${value?.text ?? ''} to ${filter.until}',
    BridgeMatch.is_ => switch (value?.kind) {
      BridgeCellKind.number => '$name is ${numberText(numbers, value!.number)}',
      BridgeCellKind.checkbox =>
        value!.checked ? '$name is ticked' : '$name is not ticked',
      BridgeCellKind.date => switch (DateTime.tryParse(value!.text ?? '')) {
        final day? => '$name is ${days.format(day)}',
        null => '$name is ${value.text}',
      },
      BridgeCellKind.select =>
        '$name is ${field.options.where((option) => option.id == value!.text).firstOrNull?.name ?? 'a removed option'}',
      _ => '$name is ${value?.text ?? ''}',
    },
  };
}

class _Panel extends StatefulWidget {
  const _Panel({
    required this.onEscape,
    required this.child,
    this.takesKeys = true,
  });

  final VoidCallback onEscape;
  final Widget child;
  final bool takesKeys;

  @override
  State<_Panel> createState() => _PanelState();
}

class _PanelState extends State<_Panel> {
  final _focus = FocusNode(debugLabel: 'Table query menu');

  @override
  void initState() {
    super.initState();
    _takeKeys();
  }

  @override
  void didUpdateWidget(_Panel old) {
    super.didUpdateWidget(old);
    if (!old.takesKeys) _takeKeys();
  }

  void _takeKeys() => WidgetsBinding.instance.addPostFrameCallback((_) {
    if (mounted && widget.takesKeys) _focus.requestFocus();
  });

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Focus(
      focusNode: _focus,
      onKeyEvent: (_, event) {
        if (event is KeyDownEvent &&
            event.logicalKey == LogicalKeyboardKey.escape) {
          widget.onEscape();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: Container(
        key: const Key('table query menu'),
        width: _menuWidth,
        constraints: const BoxConstraints(maxHeight: _menuHeight),
        padding: const EdgeInsets.all(WsMetrics.space2),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: DefaultTextStyle.merge(
          style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [widget.child],
          ),
        ),
      ),
    );
  }
}

class _Item extends StatefulWidget {
  const _Item({
    this.icon,
    this.label,
    this.leading,
    this.checked = false,
    this.muted = false,
    required this.onTap,
  });

  final IconData? icon;
  final String? label;
  final Widget? leading;
  final bool checked;
  final bool muted;
  final VoidCallback onTap;

  @override
  State<_Item> createState() => _ItemState();
}

class _ItemState extends State<_Item> {
  var _hovered = false;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final color = widget.muted ? ws.textMuted : null;
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Container(
          height: _itemHeight,
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          decoration: BoxDecoration(
            color: _hovered ? ws.hover : null,
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          ),
          child: Row(
            children: [
              if (widget.icon case final icon?) ...[
                Icon(icon, size: 14, color: color ?? ws.textFaint),
                const SizedBox(width: 10),
              ],
              ?widget.leading,
              if (widget.label case final label?)
                Expanded(
                  child: Text(
                    label,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: color),
                  ),
                )
              else
                const Spacer(),
              if (widget.checked)
                Icon(LucideIcons.check, size: 14, color: ws.accent),
            ],
          ),
        ),
      ),
    );
  }
}

Widget _list(int count, IndexedWidgetBuilder item) => Flexible(
  child: ListView.builder(
    shrinkWrap: true,
    padding: EdgeInsets.zero,
    itemCount: count,
    itemExtent: _itemHeight,
    itemBuilder: item,
  ),
);

typedef Choice = ({
  IconData icon,
  String label,
  bool checked,
  VoidCallback onTap,
});

typedef Choices = ({String? label, List<Choice> choices});

class ChoiceMenu extends StatelessWidget {
  const ChoiceMenu({
    super.key,
    this.title,
    required this.sections,
    this.hint,
    required this.onClose,
  });

  final String? title;
  final List<Choices> sections;
  final String? hint;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget faint(String text) => Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space3,
        vertical: WsMetrics.space1,
      ),
      child: Text(
        text,
        style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
      ),
    );
    final divider = Divider(height: 9, thickness: 1, color: ws.border);
    return _Panel(
      onEscape: onClose,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ?switch (title) {
            final title? => faint(title),
            null => null,
          },
          for (final (index, section) in sections.indexed) ...[
            if (section.label case final label?)
              faint(label)
            else if (index > 0)
              divider,
            for (final choice in section.choices)
              _Item(
                icon: choice.icon,
                label: choice.label,
                checked: choice.checked,
                onTap: () {
                  onClose();
                  choice.onTap();
                },
              ),
          ],
          if (hint case final hint?) ...[divider, faint(hint)],
        ],
      ),
    );
  }
}

class SortMenu extends StatelessWidget {
  const SortMenu({
    super.key,
    required this.fields,
    required this.sort,
    required this.onSet,
    required this.onClose,
  });

  final List<BridgeField> fields;
  final BridgeSort? sort;
  final ValueChanged<BridgeSort?> onSet;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final sortable = [
      for (final field in fields)
        if (field.kind != BridgeFieldKind.file &&
            field.kind != BridgeFieldKind.formula)
          field,
    ];
    final sort = this.sort;
    return _Panel(
      onEscape: onClose,
      child: Flexible(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _list(sortable.length, (context, index) {
              final field = sortable[index];
              return _Item(
                icon: fieldKindIcon(field.kind),
                label: field.name,
                checked: field.id == sort?.field,
                onTap: () => onSet(
                  BridgeSort(
                    field: field.id,
                    descending: sort?.descending ?? false,
                  ),
                ),
              );
            }),
            if (sort != null) ...[
              Divider(height: 9, thickness: 1, color: ws.border),
              for (final (descending, icon, label) in const [
                (false, LucideIcons.arrowUp, 'Ascending'),
                (true, LucideIcons.arrowDown, 'Descending'),
              ])
                _Item(
                  icon: icon,
                  label: label,
                  checked: sort.descending == descending,
                  onTap: () => onSet(
                    BridgeSort(field: sort.field, descending: descending),
                  ),
                ),
              Divider(height: 9, thickness: 1, color: ws.border),
              _Item(
                icon: LucideIcons.trash,
                label: 'Remove the sort',
                muted: true,
                onTap: () {
                  onSet(null);
                  onClose();
                },
              ),
            ],
          ],
        ),
      ),
    );
  }
}

enum _Choosing { value, field, match }

class FilterMenu extends StatefulWidget {
  const FilterMenu({
    super.key,
    required this.fields,
    required this.filter,
    required this.numbers,
    required this.locale,
    required this.onSet,
    required this.onClose,
  });

  final List<BridgeField> fields;
  final BridgeFilter? filter;
  final NumberFormat numbers;

  final String locale;
  final ValueChanged<BridgeFilter?> onSet;
  final VoidCallback onClose;

  @override
  State<FilterMenu> createState() => _FilterMenuState();
}

class _FilterMenuState extends State<FilterMenu> {
  late BridgeField _field;
  late BridgeMatch _match;
  BridgeCell? _value;
  var _typed = TextEditingController();

  var _replaced = false;
  final _typing = FocusNode(debugLabel: 'Filter value');
  var _choosing = _Choosing.value;
  var _refused = false;
  var _dropTyped = false;

  static List<BridgeMatch> _matchesOf(BridgeFieldKind kind) => switch (kind) {
    BridgeFieldKind.text => const [
      BridgeMatch.contains,
      BridgeMatch.is_,
      BridgeMatch.empty,
      BridgeMatch.notEmpty,
    ],
    BridgeFieldKind.file => const [BridgeMatch.notEmpty, BridgeMatch.empty],
    _ => const [BridgeMatch.is_, BridgeMatch.empty, BridgeMatch.notEmpty],
  };

  static String _matchLabel(BridgeMatch match) => switch (match) {
    BridgeMatch.contains => 'contains',
    BridgeMatch.is_ => 'is',
    BridgeMatch.empty => 'is empty',
    BridgeMatch.notEmpty => 'is not empty',
    BridgeMatch.between => 'is between',
  };

  bool get _types =>
      _match == BridgeMatch.contains ||
      (_match == BridgeMatch.is_ &&
          switch (_field.kind) {
            BridgeFieldKind.text ||
            BridgeFieldKind.number ||
            BridgeFieldKind.date => true,
            _ => false,
          });

  @override
  void initState() {
    super.initState();
    _load();
    _focusValue();
  }

  void _focusValue() => WidgetsBinding.instance.addPostFrameCallback((_) {
    if (mounted && _types && _choosing == _Choosing.value) {
      _typing.requestFocus();
    }
  });

  void _load() {
    final filter = widget.filter;
    final field = widget.fields
        .where((field) => field.id == filter?.field)
        .firstOrNull;
    if (filter == null || field == null) {
      _pick(widget.fields.first);
      return;
    }
    _field = field;
    _match = filter.matches;
    _value = filter.value;
    final value = filter.value;
    _show(switch (value?.kind) {
      BridgeCellKind.number => numberText(widget.numbers, value!.number),
      BridgeCellKind.date => switch (DateTime.tryParse(value!.text ?? '')) {
        final day? => shownDays(widget.locale).format(day),
        null => value.text ?? '',
      },
      BridgeCellKind.text => value!.text ?? '',
      _ => '',
    });
  }

  void _show(String text) {
    if (writeText(_typed, TextEditingValue(text: text))) _replaced = true;
  }

  void _pick(BridgeField field) {
    _field = field;
    _match = _matchesOf(field.kind).first;
    _value = null;
    _show('');
    _refused = false;
  }

  @override
  void didUpdateWidget(FilterMenu old) {
    super.didUpdateWidget(old);
    final filter = widget.filter;
    if (filter != null &&
        !sameFilter(filter, old.filter) &&
        !sameFilter(filter, _built())) {
      setState(_load);
    }
  }

  @override
  void dispose() {
    if (_types && !_dropTyped) _writeTyped();
    _typed.dispose();
    _typing.dispose();
    super.dispose();
  }

  BridgeCell _cell(
    BridgeCellKind kind, {
    String? text,
    double number = 0,
    bool checked = false,
  }) => BridgeCell(
    kind: kind,
    text: text,
    number: number,
    checked: checked,
    files: const [],
  );

  BridgeCell? _parsed() {
    final typed = _typed.text.trim();
    if (typed.isEmpty) return null;
    if (_match == BridgeMatch.contains) {
      return _cell(BridgeCellKind.text, text: typed);
    }
    switch (_field.kind) {
      case BridgeFieldKind.number:
        try {
          final number = widget.numbers.parse(typed).toDouble();
          return number.isFinite
              ? _cell(BridgeCellKind.number, number: number)
              : null;
        } on FormatException {
          return null;
        }
      case BridgeFieldKind.date:
        final day = parseDay(typed, widget.locale);
        return day == null
            ? null
            : _cell(BridgeCellKind.date, text: storedDays.format(day));
      default:
        return _cell(BridgeCellKind.text, text: typed);
    }
  }

  BridgeFilter? _built() {
    final needsValue =
        _match == BridgeMatch.is_ || _match == BridgeMatch.contains;
    final value = _types ? _parsed() : _value;
    if (needsValue && value == null) return null;
    return BridgeFilter(
      field: _field.id,
      matches: _match,
      value: needsValue ? value : null,
    );
  }

  void _commit() {
    final built = _built();
    if (built != null && !sameFilter(built, widget.filter)) {
      widget.onSet(built);
    }
  }

  void _writeTyped() {
    if (_typed.text.trim().isNotEmpty) return _commit();
    final saved = widget.filter;
    if (saved != null &&
        saved.value != null &&
        saved.field == _field.id &&
        saved.matches == _match) {
      widget.onSet(null);
    }
  }

  void _submit() {
    final typed = _typed.text.trim().isNotEmpty;
    setState(() => _refused = typed && _built() == null);
    _writeTyped();
  }

  Widget _dropdown(Key key, String label, _Choosing choosing) {
    final ws = context.ws;
    return GestureDetector(
      key: key,
      behavior: HitTestBehavior.opaque,
      onTap: () {
        setState(
          () => _choosing = _choosing == choosing ? _Choosing.value : choosing,
        );
        _focusValue();
      },
      child: Container(
        height: 26,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
        decoration: BoxDecoration(
          border: Border.all(color: ws.border),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            const SizedBox(width: WsMetrics.space1),
            Icon(LucideIcons.chevronDown, size: 12, color: ws.textFaint),
          ],
        ),
      ),
    );
  }

  Widget _values() {
    final ws = context.ws;
    if (_types) {
      return Padding(
        padding: const EdgeInsets.all(WsMetrics.space1),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              height: 28,
              alignment: Alignment.centerLeft,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              decoration: BoxDecoration(
                color: ws.canvas,
                border: Border.all(color: ws.borderStrong),
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: TextField(
                key: const Key('filter value'),
                controller: _typed,
                focusNode: _typing,
                textInputAction: TextInputAction.done,
                onSubmitted: (_) => _submit(),
                onChanged: (_) {
                  if (_refused) setState(() => _refused = false);
                },
                style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
                decoration: InputDecoration.collapsed(
                  hintText: 'Value',
                  hintStyle: TextStyle(color: ws.textFaint),
                ),
              ),
            ),
            if (_refused)
              Padding(
                padding: const EdgeInsets.only(top: WsMetrics.space1),
                child: Text(
                  _field.kind == BridgeFieldKind.date
                      ? 'This is not a date.'
                      : 'This is not a number.',
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.danger,
                  ),
                ),
              ),
          ],
        ),
      );
    }
    if (_match != BridgeMatch.is_) return const SizedBox.shrink();
    void choose(BridgeCell value) {
      setState(() => _value = value);
      _commit();
    }

    if (_field.kind == BridgeFieldKind.checkbox) {
      return Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (checked, label) in const [
            (true, 'Ticked'),
            (false, 'Not ticked'),
          ])
            _Item(
              label: label,
              checked: _value?.checked == checked && _value != null,
              onTap: () =>
                  choose(_cell(BridgeCellKind.checkbox, checked: checked)),
            ),
        ],
      );
    }
    final options = _field.options;
    return _list(options.length, (context, index) {
      final option = options[index];
      return _Item(
        leading: OptionTag(option),
        checked: _value?.text == option.id,
        onTap: () => choose(_cell(BridgeCellKind.select, text: option.id)),
      );
    });
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (_replaced) {
      _replaced = false;
      _typed = adoptController(_typed);
    }
    final matches = _matchesOf(_field.kind);
    return _Panel(
      takesKeys: !_types || _choosing != _Choosing.value,
      onEscape: () {
        _dropTyped = true;
        widget.onClose();
      },
      child: Flexible(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.all(WsMetrics.space1),
              child: Row(
                children: [
                  Flexible(
                    child: _dropdown(
                      const Key('filter field'),
                      _field.name,
                      _Choosing.field,
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  Flexible(
                    child: _dropdown(
                      const Key('filter match'),
                      _matchLabel(_match),
                      _Choosing.match,
                    ),
                  ),
                ],
              ),
            ),
            switch (_choosing) {
              _Choosing.field => _list(widget.fields.length, (context, index) {
                final field = widget.fields[index];
                return _Item(
                  icon: fieldKindIcon(field.kind),
                  label: field.name,
                  checked: field.id == _field.id,
                  onTap: () {
                    setState(() {
                      if (field.id != _field.id) _pick(field);
                      _choosing = _Choosing.value;
                    });
                    _focusValue();
                    _commit();
                  },
                );
              }),
              _Choosing.match => _list(matches.length, (context, index) {
                final match = matches[index];
                return _Item(
                  label: _matchLabel(match),
                  checked: match == _match,
                  onTap: () {
                    setState(() {
                      _match = match;
                      _choosing = _Choosing.value;
                    });
                    _focusValue();
                    _commit();
                  },
                );
              }),
              _Choosing.value => _values(),
            },
            if (widget.filter != null) ...[
              Divider(height: 9, thickness: 1, color: ws.border),
              _Item(
                icon: LucideIcons.trash,
                label: 'Remove the filter',
                muted: true,
                onTap: () {
                  _dropTyped = true;
                  widget.onSet(null);
                  widget.onClose();
                },
              ),
            ],
          ],
        ),
      ),
    );
  }
}
