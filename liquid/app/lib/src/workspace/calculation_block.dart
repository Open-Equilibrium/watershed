import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/calculation.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'block_title_field.dart';
import 'follow.dart';
import 'formula_field.dart';
import 'formula_text.dart';
import 'page_views.dart';
import 'stored_name.dart';
import 'table_view_parts.dart' show MoreMenu, touchFirst;
import 'workspace_state.dart';

const lineHeight = 30.0;
const valueWidth = 120.0;

class CalculationBlock extends StatefulWidget {
  const CalculationBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<CalculationBlock> createState() => _CalculationBlockState();
}

class _CalculationBlockState extends State<CalculationBlock> {
  List<BridgeLine>? _lines;
  int _reads = 0;
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _settled;
  String? _editing;
  String? _labelled;
  List<Suggestion> _tables = const [];
  late final FollowedSources _sourcesFollowed = FollowedSources(
    _state.workspace,
    changed: () => unawaited(_read()),
    failed: _report,
  );

  WorkspaceState get _state => widget.views.state;
  String get _page => widget.views.page;
  String get _block => widget.view.block;

  late final _name = StoredName(
    widget.view.title,
    rename: (title) => widget.views.write(
      mutation(
        BridgeMutationKind.renameView,
        page: _page,
        view: widget.view.id,
        text: title,
      ),
      [widget.view.id],
    ),
    restore: () async => mounted ? widget.view.title : null,
    quitting: () => _state.quitting,
    replaced: () {
      if (mounted) setState(() {});
    },
  );
  final _nameFocus = FocusNode(debugLabel: 'Calculation title');
  Future<void> _writes = Future.value();

  @override
  void initState() {
    super.initState();
    _state.finishing.add(_name.typed);
    _nameFocus.addListener(_leftName);
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
    _settled = widget.views.settled
        .where((settled) => settled == _block)
        .listen((_) => unawaited(_read()));
  }

  @override
  void didUpdateWidget(CalculationBlock old) {
    super.didUpdateWidget(old);
    if (widget.view.title != old.view.title &&
        !_nameFocus.hasFocus &&
        widget.views.pending(widget.view.id) == 0) {
      _name.show(widget.view.title);
    }
  }

  @override
  void dispose() {
    _state.finishing.remove(_name.typed);
    unawaited(_followed?.cancel());
    unawaited(_settled?.cancel());
    _sourcesFollowed.dispose();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _leftName() {
    if (_nameFocus.hasFocus) return;
    _name.write();
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  Future<void> _read() async {
    final read = ++_reads;
    final BridgeAnswer answer;
    try {
      answer = await _state.read(
        query(BridgeQueryKind.calculation, page: _page, block: _block),
      );
    } on Object catch (error) {
      _report(error);
      return;
    }
    if (!mounted || read != _reads) return;
    _sourcesFollowed.sync(answer.follows);
    setState(() => _lines = answer.lines);
  }

  Future<BridgeApplied> _write(BridgeMutation change) =>
      widget.views.write(change, [_block]);

  Future<void> _set(
    BridgeLine line, {
    String? label,
    String? formula,
    BridgeLineStyle? style,
  }) {
    final run = _writes.then((_) async {
      final stored = await _state.read(
        query(BridgeQueryKind.calculation, page: _page, block: _block),
      );
      final current =
          stored.lines.where((kept) => kept.id == line.id).firstOrNull ?? line;
      await _write(
        mutation(
          BridgeMutationKind.setLine,
          page: _page,
          block: _block,
          line: line.id,
          lineSetup: BridgeLineSetup(
            label: label ?? current.label,
            formula: formula ?? current.formula,
            style: style ?? current.style,
          ),
        ),
      );
    });
    _writes = run.then<void>((_) {}, onError: (Object _) {});
    return run;
  }

  Future<void> _add() async {
    final lines = _lines;
    if (lines == null) return;
    try {
      final added = await _write(
        mutation(
          BridgeMutationKind.addLine,
          page: _page,
          block: _block,
          after: lines.isEmpty ? null : lines.last.id,
          lineSetup: const BridgeLineSetup(
            label: '',
            formula: '',
            style: BridgeLineStyle.plain,
          ),
        ),
      );
      if (mounted) setState(() => _labelled = added.line);
    } on Object catch (_) {}
  }

  Future<void> _remove(BridgeLine line) => _write(
    mutation(
      BridgeMutationKind.removeLine,
      page: _page,
      block: _block,
      line: line.id,
    ),
  ).then((_) {}, onError: (Object _) {});

  Future<void> _loadTables() async {
    try {
      final tables = (await _state.read(query(BridgeQueryKind.tables))).tables;
      final found = <Suggestion>[];
      for (final table in tables.take(50)) {
        final database = (await _state.read(
          query(BridgeQueryKind.database, block: table.id),
        )).database;
        for (final field in database?.fields ?? const []) {
          final name = '${table.name}.${field.name}';
          found.add(
            Suggestion(
              name,
              'Field',
              reference: Reference('{${table.id}.${field.id}}', name),
            ),
          );
        }
      }
      if (mounted) setState(() => _tables = found);
    } on Object catch (_) {}
  }

  List<Suggestion> _known(BridgeLine editing) => [
    for (final line in _lines ?? const <BridgeLine>[])
      if (line.id != editing.id &&
          line.label.isNotEmpty &&
          line.style != BridgeLineStyle.heading)
        Suggestion(
          line.label,
          'Line',
          reference: Reference('{$_page.$_block.${line.id}}', line.label),
        ),
    ..._tables,
    const Suggestion('ABOVE', 'The lines above, up to a heading'),
    for (final name in formulaFunctions) Suggestion(name, 'Function'),
  ];

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final lines = _lines;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          height: 32,
          child: Row(
            children: [
              Icon(LucideIcons.calculator, size: 16, color: ws.textFaint),
              const SizedBox(width: WsMetrics.space3),
              Expanded(
                child: BlockTitleField(
                  controller: _name.take(),
                  focusNode: _nameFocus,
                ),
              ),
            ],
          ),
        ),
        if (lines != null) ...[
          for (final line in lines)
            _Line(
              key: ValueKey(line.id),
              line: line,
              finishing: _state.finishing,
              editing: _editing == line.id,
              focusLabel: _labelled == line.id,
              known: () => _known(line),
              onEdit: () {
                if (_tables.isEmpty) unawaited(_loadTables());
                setState(() => _editing = line.id);
              },
              onDone: () => setState(() {
                if (_editing == line.id) _editing = null;
              }),
              onLabelled: () => _labelled = null,
              set: ({label, formula, style}) =>
                  _set(line, label: label, formula: formula, style: style),
              remove: () => unawaited(_remove(line)),
            ),
          _AddLine(onAdd: () => unawaited(_add())),
        ],
      ],
    );
  }
}

typedef _SetLine = Future<void> Function({
  String? label,
  String? formula,
  BridgeLineStyle? style,
});

class _Line extends StatefulWidget {
  const _Line({
    super.key,
    required this.line,
    required this.finishing,
    required this.editing,
    required this.focusLabel,
    required this.known,
    required this.onEdit,
    required this.onDone,
    required this.onLabelled,
    required this.set,
    required this.remove,
  });

  final BridgeLine line;
  final Set<VoidCallback> finishing;
  final bool editing;
  final bool focusLabel;
  final List<Suggestion> Function() known;
  final VoidCallback onEdit;
  final VoidCallback onDone;
  final VoidCallback onLabelled;
  final _SetLine set;
  final VoidCallback remove;

  @override
  State<_Line> createState() => _LineState();
}

class _LineState extends State<_Line> {
  late final _label = TextEditingController(text: widget.line.label);
  final _labelFocus = FocusNode(debugLabel: 'Line label');
  bool _hovered = false;

  BridgeLine get _line => widget.line;

  @override
  void initState() {
    super.initState();
    _labelFocus.addListener(_leftLabel);
    widget.finishing.add(_saveLabel);
    _takeLabel();
  }

  @override
  void didUpdateWidget(_Line old) {
    super.didUpdateWidget(old);
    if (!_labelFocus.hasFocus && _label.text != _line.label) {
      writeText(_label, TextEditingValue(text: _line.label));
    }
    _takeLabel();
  }

  @override
  void dispose() {
    widget.finishing.remove(_saveLabel);
    _saveLabel();
    _label.dispose();
    _labelFocus.dispose();
    super.dispose();
  }

  void _takeLabel() {
    if (!widget.focusLabel) return;
    widget.onLabelled();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _labelFocus.requestFocus();
    });
  }

  void _leftLabel() {
    if (_labelFocus.hasFocus) return;
    _saveLabel();
  }

  void _saveLabel() {
    if (_label.text == _line.label) return;
    unawaited(widget.set(label: _label.text).then((_) {}, onError: (_) {}));
  }

  void _restyle(BridgeLineStyle style) {
    if (style == _line.style) return;
    unawaited(widget.set(style: style).then((_) {}, onError: (_) {}));
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = _line.style;
    final heading = style == BridgeLineStyle.heading;
    final total = style == BridgeLineStyle.total;
    final subtotal = style == BridgeLineStyle.subtotal;
    final text = TextStyle(
      fontSize: heading
          ? WsMetrics.fsSmall
          : total
          ? 15.0
          : WsMetrics.fsUi,
      fontWeight: total
          ? WsMetrics.fwBold
          : subtotal
          ? WsMetrics.fwSemibold
          : WsMetrics.fwRegular,
      color: heading ? ws.textFaint : ws.text,
    );
    final menu = MoreMenu(
      label: 'Line actions',
      visible: _hovered || touchFirst,
      plain: [
        (LucideIcons.text, 'Plain line', () => _restyle(BridgeLineStyle.plain)),
        (
          LucideIcons.heading3,
          'Heading',
          () => _restyle(BridgeLineStyle.heading),
        ),
        (
          LucideIcons.sigma,
          'Subtotal',
          () => _restyle(BridgeLineStyle.subtotal),
        ),
        (LucideIcons.equal, 'Total', () => _restyle(BridgeLineStyle.total)),
      ],
      items: [(LucideIcons.trash, 'Remove line', widget.remove)],
    );
    final label = TextField(
      controller: _label,
      focusNode: _labelFocus,
      style: text,
      onSubmitted: (_) => heading ? null : widget.onEdit(),
      decoration: InputDecoration.collapsed(
        hintText: heading ? 'Heading' : 'Label',
        hintStyle: text.copyWith(color: ws.textFaint),
      ),
    );
    return MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Container(
        height: heading ? 28 : lineHeight,
        padding: EdgeInsets.only(
          left: WsMetrics.space3,
          top: heading ? WsMetrics.space3 : 0,
        ),
        decoration: BoxDecoration(
          color: subtotal ? ws.subtle : null,
          border: heading
              ? null
              : total
              ? Border(top: BorderSide(color: ws.text, width: 2))
              : Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(
          children: [
            Expanded(child: label),
            if (!heading) ...[
              if (widget.editing)
                Expanded(
                  child: FormulaField(
                    formula: _line.formula,
                    names: _line.references,
                    known: widget.known,
                    onCommit: (formula) => widget.set(formula: formula),
                    onDone: widget.onDone,
                  ),
                )
              else ...[
                if (_hovered && isComputed(_line.formula))
                  Flexible(
                    child: Text(
                      '= ${namedFormula(_line.formula, _line.references)}',
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: WsMetrics.fontMono.first,
                        fontSize: WsMetrics.fsCaption,
                        color: ws.textFaint,
                      ),
                    ),
                  ),
                const SizedBox(width: 10),
                _value(ws, text),
              ],
            ],
            menu,
          ],
        ),
      ),
    );
  }

  Widget _value(WsPalette ws, TextStyle text) {
    final empty = _line.value.isEmpty;
    return GestureDetector(
      key: ValueKey('value ${_line.id}'),
      behavior: HitTestBehavior.opaque,
      onTap: widget.onEdit,
      child: SizedBox(
        width: valueWidth,
        child: Text(
          empty && _hovered ? 'Value' : _line.value,
          textAlign: TextAlign.right,
          overflow: TextOverflow.ellipsis,
          style: text.copyWith(
            color: _line.kind == BridgeValueKind.error
                ? ws.danger
                : empty
                ? ws.textFaint
                : null,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
      ),
    );
  }
}

class _AddLine extends StatelessWidget {
  const _AddLine({required this.onAdd});

  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      button: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onAdd,
        child: SizedBox(
          height: lineHeight,
          child: Row(
            children: [
              const SizedBox(width: WsMetrics.space3),
              Icon(LucideIcons.plus, size: 14, color: ws.textFaint),
              const SizedBox(width: WsMetrics.space2),
              Text(
                'Add a line',
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: ws.textFaint,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
