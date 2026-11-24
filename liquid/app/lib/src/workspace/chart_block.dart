import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:intl/intl.dart' show NumberFormat;

import '../controls.dart';
import '../rust/api/chart.dart';
import '../rust/api/database.dart';
import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'anchored_menu.dart';
import 'block_title_field.dart';
import 'explorer.dart' show untitled;
import 'follow.dart';
import 'page_views.dart';
import 'stored_name.dart';
import '../rust/api/source.dart';
import 'workspace_state.dart';

const chartHeight = 180.0;

String chartKindName(BridgeChartKind kind) => switch (kind) {
  BridgeChartKind.bar => 'Bar',
  BridgeChartKind.line => 'Line',
  BridgeChartKind.pie => 'Pie',
};

String chartSummary(BridgeChart chart, NumberFormat numbers) =>
    '${chartKindName(chart.setup.kind)} chart: ${[for (final point in chart.points) '${point.label} ${numbers.format(point.value)}'].join('; ')}';

class ChartSourceChoice {
  const ChartSourceChoice(
    this.name,
    this.detail, {
    this.table,
    this.block,
    this.source,
  });

  final String name;
  final String detail;
  final String? table;
  final String? block;
  final String? source;
}

class ChartBlock extends StatefulWidget {
  const ChartBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<ChartBlock> createState() => _ChartBlockState();
}

class _ChartBlockState extends State<ChartBlock> {
  BridgeChart? _chart;
  BridgeDatabase? _database;
  List<BridgeColumn> _columns = const [];
  String? _watched;
  StreamSubscription<void>? _watch;
  Timer? _settle;
  List<ChartSourceChoice> _sources = const [];
  int _reads = 0;
  StreamSubscription<BridgeChange>? _followed;
  StreamSubscription<String>? _located;
  late final FollowedSources _sourcesFollowed = FollowedSources(
    _state.workspace,
    changed: () => unawaited(_read()),
    failed: _report,
  );
  final _panel = OverlayPortalController();
  final _kinds = OverlayPortalController();

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
  final _nameFocus = FocusNode(debugLabel: 'Chart title');

  @override
  void initState() {
    super.initState();
    _state.finishing.add(_name.typed);
    _nameFocus.addListener(() {
      if (!_nameFocus.hasFocus) _name.write();
    });
    _followed = follow(
      _state.workspace,
      BridgeResource(
        kind: BridgeResourceKind.block,
        page: _page,
        block: _block,
      ),
    ).listen((_) => unawaited(_read()), onError: _report);
    _located = _state.located.listen((source) {
      if (source != _watched) return;
      _follow(source, again: true);
      unawaited(_read());
    });
  }

  @override
  void didUpdateWidget(ChartBlock old) {
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
    unawaited(_watch?.cancel());
    unawaited(_located?.cancel());
    _settle?.cancel();
    _sourcesFollowed.dispose();
    _name.dispose();
    _nameFocus.dispose();
    super.dispose();
  }

  void _report(Object error) {
    if (mounted) _state.readFailed(error);
  }

  Future<void> _read() async {
    final read = ++_reads;
    try {
      final answer = await _state.read(
        query(BridgeQueryKind.chart, page: _page, block: _block),
      );
      final chart = answer.chart;
      final table = chart?.setup.table;
      final database = table == null
          ? null
          : (await _state.read(query(BridgeQueryKind.database, block: table)))
                .database;
      final source = chart?.setup.source;
      final columns = chart?.columns ?? const <BridgeColumn>[];
      if (!mounted || read != _reads) return;
      _sourcesFollowed.sync(answer.follows);
      _follow(source);
      setState(() {
        _chart = chart;
        _database = database;
        _columns = columns;
      });
    } on Object catch (error) {
      _report(error);
    }
  }

  void _follow(String? source, {bool again = false}) {
    if (source == _watched && !again) return;
    _watched = source;
    unawaited(_watch?.cancel());
    _watch = source == null
        ? null
        : _state.watchSource(source).listen((_) {
            _settle?.cancel();
            _settle = Timer(
              const Duration(milliseconds: 200),
              () => unawaited(_read()),
            );
          });
  }

  Future<void> _loadSources() async {
    try {
      final tables = (await _state.read(query(BridgeQueryKind.tables))).tables;
      final views = (await _state.read(
        query(BridgeQueryKind.views, page: _page),
      )).views;
      if (!mounted) return;
      setState(
        () => _sources = [
          for (final table in tables)
            ChartSourceChoice(table.name, 'Table', table: table.id),
          for (final view in views)
            if (view.kind == BridgeBlockKind.calculation)
              ChartSourceChoice(
                view.title.isEmpty ? untitled : view.title,
                'Calculation',
                block: view.block,
              ),
          for (final area in _state.areas.value)
            for (final source in _state.sources(area.id).value)
              if (source.kind == BridgeSourceKind.file &&
                  RegExp(
                    r'\.(csv|tsv)$',
                    caseSensitive: false,
                  ).hasMatch(source.name))
                ChartSourceChoice(source.name, 'Source', source: source.id),
        ],
      );
    } on Object catch (error) {
      _report(error);
    }
  }

  void _set(BridgeChartSetup setup) {
    unawaited(
      widget.views
          .write(
            mutation(
              BridgeMutationKind.setChart,
              page: _page,
              block: _block,
              chart: setup,
            ),
            [_block],
          )
          .then((_) {}, onError: (Object _) {}),
    );
  }

  BridgeChartSetup get _setup =>
      _chart?.setup ??
      const BridgeChartSetup(
        kind: BridgeChartKind.bar,
        value: BridgeChartValue.count,
      );

  BridgeChartSetup _with({
    BridgeChartKind? kind,
    (String?, String?, String?, String?)? source,
    (String?,)? group,
    (BridgeChartValue, String?)? value,
  }) {
    final setup = _setup;
    final (table, page, block, file) =
        source ?? (setup.table, setup.page, setup.block, setup.source);
    final (measure, field) = value ?? (setup.value, setup.field);
    return BridgeChartSetup(
      kind: kind ?? setup.kind,
      table: table,
      page: page,
      block: block,
      source: file,
      group: group == null ? setup.group : group.$1,
      value: measure,
      field: field,
    );
  }

  void _togglePanel() {
    if (_panel.isShowing) {
      setState(_panel.hide);
      return;
    }
    unawaited(_loadSources());
    setState(_panel.show);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final chart = _chart;
    final numbers = NumberFormat.decimalPattern()..maximumFractionDigits = 2;
    final faint = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    return AnchoredMenu(
      controller: _panel,
      group: _panel,
      beside: chartPanelWidth,
      minLeft: WsMetrics.widthSide,
      height: 600,
      onTapOutside: (_) {
        if (_panel.isShowing) setState(_panel.hide);
      },
      menu: (_) => ChartPanel(
        group: _panel,
        title: widget.view.title,
        chart: chart,
        database: _database,
        columns: _columns,
        sources: _sources,
        onClose: _togglePanel,
        onSource: (source) => _set(
          _with(
            source: (
              source?.table,
              source?.block == null ? null : _page,
              source?.block,
              source?.source,
            ),
            group: (null,),
            value: (BridgeChartValue.count, null),
          ),
        ),
        onGroup: (group) => _set(_with(group: (group,))),
        onValue: (value, field) => _set(_with(value: (value, field))),
        onKind: (kind) => _set(_with(kind: kind)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: 32,
            child: Row(
              children: [
                Icon(LucideIcons.barChart3, size: 16, color: ws.textFaint),
                const SizedBox(width: WsMetrics.space3),
                Expanded(
                  child: BlockTitleField(
                    controller: _name.take(),
                    focusNode: _nameFocus,
                  ),
                ),
                AnchoredMenu(
                  controller: _kinds,
                  onTapOutside: (_) => setState(_kinds.hide),
                  menu: (_) => ChoiceList(
                    choices: [
                      for (final kind in BridgeChartKind.values)
                        (
                          chartKindName(kind),
                          kind == _setup.kind,
                          () {
                            setState(_kinds.hide);
                            _set(_with(kind: kind));
                          },
                        ),
                    ],
                  ),
                  child: _Chip(
                    key: const Key('chart kind'),
                    label: chartKindName(_setup.kind),
                    onTap: () => setState(_kinds.toggle),
                  ),
                ),
                const SizedBox(width: WsMetrics.space3),
                GestureDetector(
                  key: const Key('chart data'),
                  behavior: HitTestBehavior.opaque,
                  onTap: _togglePanel,
                  child: Text(switch (chart?.source) {
                    final source? =>
                      'from ${source.isEmpty ? untitled : source}',
                    null => 'Choose data',
                  }, style: faint),
                ),
              ],
            ),
          ),
          SizedBox(
            height: chartHeight,
            child: switch (chart) {
              null => const SizedBox.shrink(),
              BridgeChart(source: null) => Center(
                child: Text(
                  'Choose its data from a Table, a Calculation or a Source',
                  style: faint,
                ),
              ),
              BridgeChart(:final points) when points.isEmpty => Center(
                child: Text('Nothing to draw yet', style: faint),
              ),
              BridgeChart(
                :final points,
                setup: BridgeChartSetup(kind: BridgeChartKind.line),
              )
                  when points.length < 2 =>
                Center(child: Text('A line needs two points', style: faint)),
              final chart => Semantics(
                label: chartSummary(chart, numbers),
                child: CustomPaint(
                  painter: ChartPainter(
                    points: chart.points,
                    kind: chart.setup.kind,
                    colors: [ws.data1, ws.data2, ws.data3],
                    text: DefaultTextStyle.of(context).style.copyWith(
                      fontSize: WsMetrics.fsCaption,
                      color: ws.textFaint,
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                    line: ws.border,
                    numbers: numbers,
                  ),
                  size: Size.infinite,
                ),
              ),
            },
          ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({super.key, required this.label, required this.onTap});

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
          color: ws.subtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              label,
              style: TextStyle(
                fontSize: WsMetrics.fsCompact,
                color: ws.textMuted,
              ),
            ),
            const SizedBox(width: WsMetrics.space1),
            Icon(LucideIcons.chevronDown, size: 12, color: ws.textFaint),
          ],
        ),
      ),
    );
  }
}

class ChoiceList extends StatelessWidget {
  const ChoiceList({super.key, required this.choices});

  final List<(String, bool, VoidCallback)> choices;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return FloatingPanel(
      width: 220,
      constraints: const BoxConstraints(maxHeight: 320),
      padding: const EdgeInsets.all(WsMetrics.space1),
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (label, chosen, onTap) in choices)
              InkWell(
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                onTap: onTap,
                child: Container(
                  height: 30,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          label,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: WsMetrics.fsUi,
                            color: ws.text,
                          ),
                        ),
                      ),
                      if (chosen)
                        Icon(LucideIcons.check, size: 14, color: ws.accent),
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

const chartPanelWidth = 320.0;

class ChartPanel extends StatelessWidget {
  const ChartPanel({
    super.key,
    required this.group,
    required this.title,
    required this.chart,
    required this.database,
    this.columns = const [],
    required this.sources,
    required this.onClose,
    required this.onSource,
    required this.onGroup,
    required this.onValue,
    required this.onKind,
  });

  final Object group;
  final String title;
  final BridgeChart? chart;
  final BridgeDatabase? database;
  final List<BridgeColumn> columns;
  final List<ChartSourceChoice> sources;
  final VoidCallback onClose;
  final ValueChanged<ChartSourceChoice?> onSource;
  final ValueChanged<String?> onGroup;
  final void Function(BridgeChartValue value, String? field) onValue;
  final ValueChanged<BridgeChartKind> onKind;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final setup = chart?.setup;
    final fields = database?.fields ?? const <BridgeField>[];
    String? nameOf(String? field) =>
        fields.where((known) => known.id == field).firstOrNull?.name ??
        columns.where((known) => known.id == field).firstOrNull?.name;
    final groups = <({String id, String name})>[
      for (final field in fields)
        if (field.kind == BridgeFieldKind.select ||
            field.kind == BridgeFieldKind.checkbox ||
            field.kind == BridgeFieldKind.text)
          (id: field.id, name: field.name),
      for (final column in columns) (id: column.id, name: column.name),
    ];
    final numbers = <({String id, String name})>[
      for (final field in fields)
        if (field.kind == BridgeFieldKind.number ||
            field.kind == BridgeFieldKind.formula)
          (id: field.id, name: field.name),
      for (final column in columns)
        if (column.number) (id: column.id, name: column.name),
    ];
    String valueName(BridgeChartValue value, String? field) => switch (value) {
      BridgeChartValue.count => 'Count of rows',
      BridgeChartValue.sum => 'Sum of ${nameOf(field) ?? '…'}',
      BridgeChartValue.average => 'Average of ${nameOf(field) ?? '…'}',
    };
    final heading = TextStyle(
      fontSize: WsMetrics.fsSmall,
      fontWeight: WsMetrics.fwMedium,
      color: ws.textFaint,
    );
    final caption = TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint);
    final table = setup?.table != null;
    final file = setup?.source != null;
    return FloatingPanel(
      key: const Key('chart panel'),
      width: chartPanelWidth,
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space5,
        vertical: WsMetrics.space4,
      ),
      child: DefaultTextStyle.merge(
        style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            spacing: WsMetrics.space4,
            children: [
              Row(
                children: [
                  Icon(LucideIcons.barChart3, size: 16, color: ws.textFaint),
                  const SizedBox(width: WsMetrics.space3),
                  Expanded(
                    child: Text(
                      title.isEmpty ? untitled : title,
                      style: const TextStyle(
                        fontSize: WsMetrics.fsUi,
                        fontWeight: WsMetrics.fwSemibold,
                      ),
                    ),
                  ),
                  GestureDetector(
                    onTap: onClose,
                    child: Icon(LucideIcons.x, size: 16, color: ws.textFaint),
                  ),
                ],
              ),
              Text('Data', style: heading),
              _Field(
                group: group,
                label: 'From',
                value: chart?.source ?? 'None',
                choices: [
                  (
                    'None',
                    setup?.table == null &&
                        setup?.block == null &&
                        setup?.source == null,
                    () => onSource(null),
                  ),
                  for (final source in sources)
                    (
                      '${source.name} · ${source.detail}',
                      switch (source) {
                        ChartSourceChoice(:final table?) =>
                          table == setup?.table,
                        ChartSourceChoice(:final source?) =>
                          source == setup?.source,
                        _ => source.block == setup?.block,
                      },
                      () => onSource(source),
                    ),
                ],
              ),
              if (table || file) ...[
                _Field(
                  group: group,
                  label: 'Group by',
                  value: nameOf(setup?.group) ?? 'None',
                  choices: [
                    ('None', setup?.group == null, () => onGroup(null)),
                    for (final field in groups)
                      (
                        field.name,
                        field.id == setup?.group,
                        () => onGroup(field.id),
                      ),
                  ],
                ),
                _Field(
                  group: group,
                  label: 'Value',
                  value: valueName(
                    setup?.value ?? BridgeChartValue.count,
                    setup?.field,
                  ),
                  choices: [
                    (
                      'Count of rows',
                      setup?.value == BridgeChartValue.count,
                      () => onValue(BridgeChartValue.count, null),
                    ),
                    for (final measure in [
                      BridgeChartValue.sum,
                      BridgeChartValue.average,
                    ])
                      for (final field in numbers)
                        (
                          valueName(measure, field.id),
                          setup?.value == measure && setup?.field == field.id,
                          () => onValue(measure, field.id),
                        ),
                  ],
                ),
              ],
              _Field(
                group: group,
                label: 'Kind',
                value: chartKindName(setup?.kind ?? BridgeChartKind.bar),
                choices: [
                  for (final kind in BridgeChartKind.values)
                    (
                      chartKindName(kind),
                      kind == setup?.kind,
                      () => onKind(kind),
                    ),
                ],
              ),
              Text('Uses', style: heading),
              switch (chart?.source) {
                final source? => Container(
                  height: 32,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  decoration: BoxDecoration(
                    color: ws.subtle,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                  ),
                  child: Row(
                    children: [
                      Icon(
                        table
                            ? LucideIcons.table
                            : file
                            ? LucideIcons.fileSpreadsheet
                            : LucideIcons.calculator,
                        size: 14,
                        color: ws.textFaint,
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      Expanded(
                        child: Text(source, overflow: TextOverflow.ellipsis),
                      ),
                      Text(
                        table
                            ? 'Table'
                            : file
                            ? 'Source'
                            : 'Calculation',
                        style: caption,
                      ),
                    ],
                  ),
                ),
                null => Text('Nothing yet.', style: caption),
              },
              Text('Used by', style: heading),
              Text(
                'Nothing yet. A formula or Automation that reads this Chart appears here.',
                style: caption,
              ),
              Text(
                'The data list offers Tables, Calculation Blocks and CSV Sources you may read. '
                'Removing a Connection leaves both Blocks in place.',
                style: caption,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Field extends StatefulWidget {
  const _Field({
    required this.group,
    required this.label,
    required this.value,
    required this.choices,
  });

  final Object group;
  final String label;
  final String value;
  final List<(String, bool, VoidCallback)> choices;

  @override
  State<_Field> createState() => _FieldState();
}

class _FieldState extends State<_Field> {
  final _menu = OverlayPortalController();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Row(
      children: [
        SizedBox(
          width: 88,
          child: Text(widget.label, style: TextStyle(color: ws.textMuted)),
        ),
        Expanded(
          child: AnchoredMenu(
            controller: _menu,
            group: widget.group,
            onTapOutside: (_) => setState(_menu.hide),
            menu: (_) => ChoiceList(
              choices: [
                for (final (label, chosen, onTap) in widget.choices)
                  (
                    label,
                    chosen,
                    () {
                      setState(_menu.hide);
                      onTap();
                    },
                  ),
              ],
            ),
            child: GestureDetector(
              key: Key('chart ${widget.label}'),
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(_menu.toggle),
              child: Container(
                height: 28,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  border: Border.all(color: ws.borderStrong),
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        widget.value,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    Icon(
                      LucideIcons.chevronDown,
                      size: 12,
                      color: ws.textFaint,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class ChartPainter extends CustomPainter {
  ChartPainter({
    required this.points,
    required this.kind,
    required this.colors,
    required this.text,
    required this.line,
    required this.numbers,
  });

  final List<BridgeChartPoint> points;
  final BridgeChartKind kind;
  final List<Color> colors;
  final TextStyle text;
  final Color line;
  final NumberFormat numbers;

  static const _labels = 18.0;
  static const _values = 16.0;

  TextPainter _text(String shown, double width) => TextPainter(
    text: TextSpan(text: shown, style: text),
    maxLines: 1,
    ellipsis: '…',
    textDirection: TextDirection.ltr,
  )..layout(maxWidth: math.max(0, width));

  void _centered(
    Canvas canvas,
    String shown,
    double x,
    double y,
    double width,
  ) {
    final painter = _text(shown, width);
    painter.paint(canvas, Offset(x - painter.width / 2, y));
  }

  @override
  void paint(Canvas canvas, Size size) {
    switch (kind) {
      case BridgeChartKind.pie:
        _pie(canvas, size);
      case BridgeChartKind.bar || BridgeChartKind.line:
        _axes(canvas, size);
    }
  }

  void _axes(Canvas canvas, Size size) {
    final top = points.map((point) => point.value).fold(0.0, math.max);
    final low = points.map((point) => point.value).fold(0.0, math.min);
    final span = top - low == 0 ? 1.0 : top - low;
    final plot = size.height - _labels - _values;
    final step = size.width / points.length;
    final base = _values + plot * top / span;
    canvas.drawLine(
      Offset(0, base),
      Offset(size.width, base),
      Paint()..color = line,
    );
    double y(double value) => _values + plot * (top - value) / span;
    final path = Path();
    for (final (at, point) in points.indexed) {
      final x = step * at + step / 2;
      if (kind == BridgeChartKind.bar) {
        final width = math.min(step * 0.7, 64.0);
        final rect = Rect.fromLTRB(
          x - width / 2,
          math.min(y(point.value), base),
          x + width / 2,
          math.max(y(point.value), base),
        );
        canvas.drawRRect(
          RRect.fromRectAndCorners(
            rect,
            topLeft: const Radius.circular(3),
            topRight: const Radius.circular(3),
          ),
          Paint()..color = colors.first,
        );
      } else {
        at == 0
            ? path.moveTo(x, y(point.value))
            : path.lineTo(x, y(point.value));
        canvas.drawCircle(
          Offset(x, y(point.value)),
          3,
          Paint()..color = colors.first,
        );
      }
      _centered(
        canvas,
        numbers.format(point.value),
        x,
        y(math.max(point.value, 0)) -
            _values +
            (kind == BridgeChartKind.line ? -4 : 2),
        step,
      );
      _centered(canvas, point.label, x, size.height - _labels + 4, step);
    }
    if (kind == BridgeChartKind.line) {
      canvas.drawPath(
        path,
        Paint()
          ..color = colors.first
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
    }
  }

  void _pie(Canvas canvas, Size size) {
    final shown = points.where((point) => point.value > 0).toList();
    final sum = shown.fold(0.0, (total, point) => total + point.value);
    if (sum == 0) return;
    final radius = size.height / 2 - 4;
    final center = Offset(radius + 4, size.height / 2);
    var start = -math.pi / 2;
    for (final (at, point) in shown.indexed) {
      final sweep = point.value / sum * 2 * math.pi;
      final color = colors[at % colors.length].withValues(
        alpha: 1 - 0.3 * (at ~/ colors.length % 3),
      );
      canvas.drawArc(
        Rect.fromCircle(center: center, radius: radius),
        start,
        sweep,
        true,
        Paint()..color = color,
      );
      final legend = Offset(center.dx + radius + 24, 8.0 + at * 20);
      if (legend.dy + 14 < size.height) {
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(legend.dx, legend.dy + 2, 10, 10),
            const Radius.circular(2),
          ),
          Paint()..color = color,
        );
        _text(
          '${point.label}  ${numbers.format(point.value)}',
          size.width - legend.dx - 16,
        ).paint(canvas, legend.translate(16, 0));
      }
      start += sweep;
    }
  }

  @override
  bool shouldRepaint(ChartPainter old) =>
      old.points != points || old.kind != kind || old.colors != colors;
}
