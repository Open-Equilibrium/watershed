import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../format.dart';
import '../session/session_figures.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'segment_bar.dart';
import 'statistics_chart.dart';

/// The text the Ask button puts in the input; the user completes it and sends it.
const askStart = 'About this session: ';

enum _View { cost, tokens, time }

enum _Breakdown { model, tool, turn }

/// The statistics panel of a session (ADR-0279): what its stored entries count to, by cost,
/// tokens or time, with a chart per turn and the breakdowns. It opens over the session card, and
/// Escape or ✕ closes it.
class StatisticsPanel extends StatefulWidget {
  const StatisticsPanel({
    super.key,
    required this.state,
    required this.onClose,
    required this.onAsk,
  });

  final SessionState state;
  final VoidCallback onClose;

  /// Called after the Ask button puts its text in the input, which then takes the keys.
  final VoidCallback onAsk;

  @override
  State<StatisticsPanel> createState() => _StatisticsPanelState();
}

class _StatisticsPanelState extends State<StatisticsPanel> {
  _View _view = _View.cost;
  _Breakdown _breakdown = _Breakdown.tool;

  /// Takes the keys once shown, so Escape closes the panel whatever held focus before.
  final _focus = FocusNode(debugLabel: 'statistics panel');

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  void _select(_View view) => setState(() {
    _view = view;
    _breakdown = switch (view) {
      _View.cost => _Breakdown.tool,
      _View.tokens => _Breakdown.model,
      _View.time => _Breakdown.turn,
    };
  });

  void _ask() {
    widget.state.draft.value = askStart;
    widget.onAsk();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Focus(
      focusNode: _focus,
      onKeyEvent: (node, event) {
        if (event is KeyDownEvent &&
            event.logicalKey == LogicalKeyboardKey.escape) {
          widget.onClose();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: DecoratedBox(
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
          boxShadow: ws.shadow,
        ),
        child: Material(
          key: const Key('statistics-panel'),
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
          clipBehavior: Clip.antiAlias,
          child: ListenableBuilder(
            listenable: Listenable.merge([
              widget.state.statistics,
              widget.state.status,
            ]),
            builder: (context, _) {
              final statistics = widget.state.statistics.value;
              final running = widget.state.status.value == RunStatus.running;
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _header(ws),
                  Expanded(
                    child: statistics == null
                        ? Padding(
                            padding: const EdgeInsets.all(20),
                            child: Text(
                              'Reading the statistics…',
                              style: TextStyle(
                                fontSize: WsMetrics.fsSmall,
                                color: ws.textFaint,
                              ),
                            ),
                          )
                        : SingleChildScrollView(
                            padding: const EdgeInsets.fromLTRB(20, 16, 20, 20),
                            child: _body(
                              ws,
                              SessionFigures(statistics, running: running),
                            ),
                          ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }

  Widget _header(WsPalette ws) => Container(
    height: 44,
    padding: const EdgeInsets.only(left: 20, right: 8),
    decoration: BoxDecoration(
      border: Border(bottom: BorderSide(color: ws.border)),
    ),
    child: Row(
      spacing: 10,
      children: [
        Text(
          'Session statistics',
          style: TextStyle(
            fontSize: WsMetrics.fsUi,
            fontWeight: WsMetrics.fwSemibold,
            color: ws.text,
          ),
        ),
        const Spacer(),
        _Segments<_View>(
          options: const [
            (_View.cost, 'Cost'),
            (_View.tokens, 'Tokens'),
            (_View.time, 'Time'),
          ],
          selected: _view,
          onSelect: _select,
        ),
        IconButton(
          tooltip: 'Close',
          onPressed: widget.onClose,
          icon: Icon(LucideIcons.x, size: 16, color: ws.textFaint),
        ),
      ],
    ),
  );

  Widget _body(WsPalette ws, SessionFigures figures) {
    final children = switch (_view) {
      _View.cost => _costView(ws, figures),
      _View.tokens => _tokensView(ws, figures),
      _View.time => _timeView(ws, figures),
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 20,
      children: [...children, _breakdownSection(ws, figures), _askRow(ws)],
    );
  }

  /// The headline and its per-model or per-part rows on the left, the chart on the right.
  Widget _top(
    WsPalette ws, {
    required String title,
    required String hero,
    required String caption,
    required List<Widget> who,
    required TurnChart chart,
  }) => Row(
    crossAxisAlignment: CrossAxisAlignment.start,
    spacing: 24,
    children: [
      SizedBox(
        width: 200,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: 16,
          children: [
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              spacing: 2,
              children: [
                Text(
                  hero,
                  style: TextStyle(
                    fontSize: 32,
                    height: 40 / 32,
                    fontWeight: WsMetrics.fwSemibold,
                    fontFeatures: const [FontFeature.tabularFigures()],
                    color: ws.text,
                  ),
                ),
                Text(
                  caption,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textFaint,
                  ),
                ),
              ],
            ),
            ...who,
          ],
        ),
      ),
      Expanded(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          spacing: 8,
          children: [_heading(ws, title), chart],
        ),
      ),
    ],
  );

  List<Widget> _costView(WsPalette ws, SessionFigures figures) {
    final models = figures.models;
    final kinds = figures.costByKind;
    return [
      _top(
        ws,
        title: 'Cost per turn',
        hero: '≈ ${formatCost(figures.cost)}',
        caption: _responsesAndTurns(figures),
        who: [
          for (final model in models)
            _who(
              ws,
              colour: _series(ws, model.series),
              name: model.name,
              value: '≈ ${formatCost(model.cost)}',
              caption:
                  '${formatShare(model.cost, figures.cost)} of cost · '
                  '${formatTokens(model.tokens)} tokens · '
                  '${plural(model.responses, 'response')}',
            ),
        ],
        chart: _chart(
          ws,
          figures,
          columnParts: (turn) =>
              turn.bySeries((model) => model.usage.costMicroUsd),
          colours: [..._seriesColours(ws), ws.textFaint],
          axis: (value) => value == 0 ? r'$0.00' : formatCost(value),
          rows: (turn) => [
            for (final model in turn.stats.models)
              if (model.usage.costMicroUsd > 0)
                ChartTipRow(
                  _series(ws, figures.seriesOf(model.model)),
                  modelName(model.model),
                  '≈ ${formatCost(model.usage.costMicroUsd)}',
                ),
          ],
          notes: (turn) => [
            turn.prompt,
            '${plural(turn.responses, 'response')} · ${formatTokens(turn.tokens)} tokens',
          ],
        ),
      ),
      _totals(ws, figures),
      _byType(
        ws,
        'Cost by type',
        input: kinds.input,
        cacheRead: kinds.cacheRead,
        cacheWrite: kinds.cacheWrite,
        output: kinds.output,
        format: formatCost,
      ),
    ];
  }

  /// A total's share bar and legend, split into input, cache read, cache write and output.
  Widget _byType(
    WsPalette ws,
    String title, {
    required int input,
    required int cacheRead,
    required int cacheWrite,
    required int output,
    required String Function(int) format,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    spacing: 8,
    children: [
      _heading(ws, title),
      SegmentBar(
        height: 8,
        segments: [
          BarSegment(input, ws.data1),
          BarSegment(cacheRead, ws.data2),
          BarSegment(cacheWrite, ws.textFaint),
          BarSegment(output, ws.data3),
        ],
      ),
      _legend(ws, [
        ('Input', ws.data1, format(input)),
        ('Cache read', ws.data2, format(cacheRead)),
        ('Cache write', ws.textFaint, format(cacheWrite)),
        ('Output', ws.data3, format(output)),
      ]),
    ],
  );

  List<Widget> _tokensView(WsPalette ws, SessionFigures figures) {
    final usage = figures.usage;
    return [
      _top(
        ws,
        title: 'Tokens per turn',
        hero: formatTokens(figures.tokens),
        caption: _responsesAndTurns(figures),
        who: [
          for (final model in figures.models)
            _who(
              ws,
              colour: _series(ws, model.series),
              name: model.name,
              value: formatTokens(model.tokens),
              caption:
                  '${formatShare(model.tokens, figures.tokens)} of tokens · '
                  '≈ ${formatCost(model.cost)} · '
                  '${plural(model.responses, 'response')}',
            ),
        ],
        chart: _chart(
          ws,
          figures,
          columnParts: (turn) => [
            turn.usage.input,
            turn.usage.cacheRead,
            turn.usage.cacheWrite,
            turn.usage.output,
          ],
          colours: [ws.data1, ws.data2, ws.textFaint, ws.data3],
          axis: (value) => value == 0 ? '0' : formatTokens(value),
          rows: (turn) => [
            ChartTipRow(ws.data1, 'Input', formatTokens(turn.usage.input)),
            ChartTipRow(
              ws.data2,
              'Cache read',
              formatTokens(turn.usage.cacheRead),
            ),
            ChartTipRow(
              ws.textFaint,
              'Cache write',
              formatTokens(turn.usage.cacheWrite),
            ),
            ChartTipRow(ws.data3, 'Output', formatTokens(turn.usage.output)),
          ],
          notes: (turn) => [turn.prompt],
        ),
      ),
      _totals(ws, figures),
      _byType(
        ws,
        'Tokens by type',
        input: usage.input,
        cacheRead: usage.cacheRead,
        cacheWrite: usage.cacheWrite,
        output: usage.output,
        format: formatTokens,
      ),
    ];
  }

  List<Widget> _timeView(WsPalette ws, SessionFigures figures) {
    final working = figures.workingMs;
    final (model, tool, other) = (
      figures.modelMs,
      figures.toolMs,
      figures.otherMs,
    );
    String share(int? part) =>
        part == null || working == 0 ? '—' : formatShare(part, working);
    return [
      _top(
        ws,
        title: 'Time per turn',
        hero: formatSpan(working),
        caption: [
          plural(figures.turns.length, 'turn'),
          if (figures.spanMs case final span?)
            '${formatSpan(span)} from first to last entry',
        ].join(' · '),
        who: [
          _who(
            ws,
            colour: ws.data1,
            name: 'Waiting for the model',
            value: model == null ? '—' : formatSpan(model),
            caption: model == null
                ? 'Not recorded for turns without timing entries'
                : '${share(model)} · ${plural(figures.responses, 'response')}'
                      ' · ${formatSpan(model ~/ (figures.responses == 0 ? 1 : figures.responses))}'
                      ' each on average',
          ),
          _who(
            ws,
            colour: ws.data2,
            name: 'Running tools',
            value: tool == null ? '—' : formatSpan(tool),
            caption: tool == null
                ? 'Not recorded for turns without timing entries'
                : '${share(tool)} · ${plural(figures.toolCalls, 'call')}',
          ),
          _who(
            ws,
            colour: ws.textFaint,
            name: 'Other',
            value: other == null ? '—' : formatSpan(other),
            caption: other == null
                ? 'Not recorded for turns without timing entries'
                : '${share(other)} · in the turn, neither model nor tool',
          ),
        ],
        chart: _chart(
          ws,
          figures,
          columnParts: (turn) => [
            turn.modelMs ?? 0,
            turn.toolMs ?? 0,
            turn.otherMs ?? 0,
          ],
          colours: [ws.data1, ws.data2, ws.textFaint],
          axis: (value) => value == 0
              ? '0'
              : value < 60000
              ? '${(value / 1000).round()} s'
              : '${(value / 60000).toStringAsFixed(1)} min',
          title: (turn) =>
              'Turn ${turn.index} · ${formatSpan(turn.workingMs ?? 0)}',
          rows: (turn) => [
            ChartTipRow(ws.data1, 'Model', formatSpan(turn.modelMs ?? 0)),
            ChartTipRow(ws.data2, 'Tools', formatSpan(turn.toolMs ?? 0)),
            ChartTipRow(ws.textFaint, 'Other', formatSpan(turn.otherMs ?? 0)),
          ],
          notes: (turn) => [
            turn.prompt,
            if (turn.modelMs == null) 'Timing not recorded for this turn',
          ],
        ),
      ),
    ];
  }

  String _responsesAndTurns(SessionFigures figures) =>
      '${plural(figures.responses, 'response')} · '
      '${plural(figures.turns.length, 'turn')} · as pi recorded it';

  /// The chart of the turns: one column per turn, its parts stacked as `columnParts` gives them.
  TurnChart _chart(
    WsPalette ws,
    SessionFigures figures, {
    required List<int> Function(TurnFigure turn) columnParts,
    required List<Color> colours,
    required String Function(int value) axis,
    required List<ChartTipRow> Function(TurnFigure turn) rows,
    required List<String> Function(TurnFigure turn) notes,
    String Function(TurnFigure turn)? title,
  }) => TurnChart(
    colors: colours,
    axis: axis,
    columns: [
      for (final turn in figures.turns)
        ChartColumn(
          parts: columnParts(turn),
          tip: ChartTip(
            title: title?.call(turn) ?? 'Turn ${turn.index}',
            rows: rows(turn),
            notes: notes(turn),
          ),
        ),
    ],
  );

  Widget _totals(WsPalette ws, SessionFigures figures) {
    final usage = figures.usage;
    final prompt = usage.input + usage.cacheRead;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 8,
      children: [
        _heading(ws, 'Totals'),
        Row(
          spacing: 16,
          children: [
            for (final (label, value) in [
              ('Input', formatTokens(usage.input)),
              ('Cache read', formatTokens(usage.cacheRead)),
              ('Cache write', formatTokens(usage.cacheWrite)),
              ('Output', formatTokens(usage.output)),
              ('Read from cache', formatShare(usage.cacheRead, prompt)),
            ])
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  spacing: 2,
                  children: [
                    Text(
                      label,
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.textFaint,
                      ),
                    ),
                    Text(
                      value,
                      style: TextStyle(
                        fontSize: WsMetrics.fsProse,
                        fontWeight: WsMetrics.fwMedium,
                        fontFeatures: const [FontFeature.tabularFigures()],
                        color: ws.text,
                      ),
                    ),
                  ],
                ),
              ),
          ],
        ),
      ],
    );
  }

  Widget _breakdownSection(WsPalette ws, SessionFigures figures) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    spacing: 4,
    children: [
      Row(
        children: [
          _heading(ws, 'Breakdown'),
          const Spacer(),
          _Segments<_Breakdown>(
            options: const [
              (_Breakdown.model, 'Model'),
              (_Breakdown.tool, 'Tool'),
              (_Breakdown.turn, 'Turn'),
            ],
            selected: _breakdown,
            onSelect: (next) => setState(() => _breakdown = next),
          ),
        ],
      ),
      switch (_breakdown) {
        _Breakdown.model => _modelTable(ws, figures),
        _Breakdown.tool => _toolTable(ws, figures),
        _Breakdown.turn => _turnTable(ws, figures),
      },
    ],
  );

  Widget _modelTable(WsPalette ws, SessionFigures figures) => _table(
    ws,
    headers: const ['#', 'Model', 'Responses', 'Tokens', 'Cost'],
    rows: [
      for (final (index, model) in figures.models.indexed)
        [
          _index(ws, index),
          _name(ws, model.name),
          _number(ws, '${model.responses}'),
          _number(ws, formatTokens(model.tokens)),
          _number(ws, '≈ ${formatCost(model.cost)}'),
        ],
    ],
  );

  Widget _toolTable(WsPalette ws, SessionFigures figures) {
    final tools = figures.statistics.tools;
    final largest = tools.fold(
      1,
      (most, tool) =>
          tool.succeeded + tool.failed + tool.cancelled + tool.open > most
          ? tool.succeeded + tool.failed + tool.cancelled + tool.open
          : most,
    );
    return _table(
      ws,
      headers: const ['#', 'Tool', 'Calls', 'Failed'],
      rows: [
        for (final (index, tool) in tools.indexed)
          [
            _index(ws, index),
            _name(
              ws,
              tool.name,
              mono: true,
              bar: _miniBar(
                ws,
                fraction:
                    (tool.succeeded +
                        tool.failed +
                        tool.cancelled +
                        tool.open) /
                    largest,
                segments: [
                  BarSegment(tool.succeeded, ws.textFaint),
                  BarSegment(tool.failed, ws.danger),
                  BarSegment(tool.cancelled, ws.borderStrong),
                  BarSegment(tool.open, ws.textFaint, hatched: figures.running),
                ],
              ),
            ),
            _number(
              ws,
              '${tool.succeeded + tool.failed + tool.cancelled + tool.open}',
            ),
            _number(
              ws,
              '${tool.failed}',
              color: tool.failed > 0 ? ws.danger : ws.textFaint,
            ),
          ],
      ],
    );
  }

  Widget _turnTable(WsPalette ws, SessionFigures figures) {
    final longest = figures.turns.fold(
      1,
      (most, turn) => (turn.workingMs ?? 0) > most ? turn.workingMs! : most,
    );
    return _table(
      ws,
      headers: const ['#', 'Turn', 'Time', 'Tokens', 'Cost'],
      rows: [
        for (final turn in figures.turns)
          [
            _index(ws, turn.index - 1),
            _name(
              ws,
              turn.prompt,
              bar: turn.workingMs == null
                  ? null
                  : _miniBar(
                      ws,
                      fraction: turn.workingMs! / longest,
                      segments: turn.modelMs == null
                          ? [BarSegment(turn.workingMs!, ws.textFaint)]
                          : [
                              BarSegment(turn.modelMs!, ws.data1),
                              BarSegment(turn.toolMs!, ws.data2),
                              BarSegment(turn.otherMs!, ws.textFaint),
                            ],
                    ),
              trailing: turn.failed > 0
                  ? Text(
                      '${turn.failed} failed',
                      style: TextStyle(
                        fontSize: WsMetrics.fsSmall,
                        color: ws.danger,
                      ),
                    )
                  : null,
            ),
            _number(
              ws,
              turn.workingMs == null ? '—' : formatSpan(turn.workingMs!),
            ),
            _number(ws, formatTokens(turn.tokens)),
            _number(ws, '≈ ${formatCost(turn.cost)}'),
          ],
      ],
    );
  }

  Widget _askRow(WsPalette ws) => Row(
    spacing: 16,
    children: [
      InkWell(
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        onTap: _ask,
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: 16),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            border: Border.all(color: ws.borderStrong),
          ),
          alignment: Alignment.center,
          child: Text(
            'Ask the agent about this session…',
            style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
          ),
        ),
      ),
      Flexible(
        child: Text(
          'puts a question in the input; nothing is sent until you send it',
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
        ),
      ),
    ],
  );

  Widget _heading(WsPalette ws, String text) => Text(
    text,
    style: TextStyle(
      fontSize: WsMetrics.fsCompact,
      fontWeight: WsMetrics.fwMedium,
      color: ws.text,
    ),
  );

  Widget _who(
    WsPalette ws, {
    required Color colour,
    required String name,
    required String value,
    required String caption,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    spacing: 2,
    children: [
      Row(
        children: [
          _dot(colour),
          const SizedBox(width: 7),
          Expanded(
            child: Text(
              name,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
            ),
          ),
          Text(
            value,
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontWeight: WsMetrics.fwMedium,
              fontFeatures: const [FontFeature.tabularFigures()],
              color: ws.text,
            ),
          ),
        ],
      ),
      Text(
        caption,
        style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
      ),
    ],
  );

  Widget _legend(WsPalette ws, List<(String, Color, String)> items) => Wrap(
    spacing: 14,
    runSpacing: 4,
    children: [
      for (final (label, colour, value) in items)
        Row(
          mainAxisSize: MainAxisSize.min,
          spacing: 5,
          children: [
            _dot(colour, size: 8, radius: 2),
            Text(
              label,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
            Text(
              value,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.text,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ],
        ),
    ],
  );

  static Widget _dot(Color colour, {double size = 8, double radius = 4}) =>
      Container(
        width: size,
        height: size,
        decoration: BoxDecoration(
          color: colour,
          borderRadius: BorderRadius.circular(radius),
        ),
      );

  Widget _index(WsPalette ws, int index) => Text(
    '${index + 1}',
    style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textFaint),
  );

  Widget _name(
    WsPalette ws,
    String text, {
    bool mono = false,
    Widget? bar,
    Widget? trailing,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    mainAxisSize: MainAxisSize.min,
    spacing: 5,
    children: [
      Row(
        spacing: 6,
        children: [
          Flexible(
            child: Text(
              text,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: mono
                  ? monoStyle(context).copyWith(fontSize: WsMetrics.fsSmall)
                  : TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
            ),
          ),
          ?trailing,
        ],
      ),
      ?bar,
    ],
  );

  Widget _miniBar(
    WsPalette ws, {
    required double fraction,
    required List<BarSegment> segments,
  }) => FractionallySizedBox(
    widthFactor: fraction.clamp(0, 1).toDouble(),
    alignment: Alignment.centerLeft,
    child: SegmentBar(segments: segments),
  );

  Widget _number(WsPalette ws, String text, {Color? color}) => Text(
    text,
    textAlign: TextAlign.right,
    style: TextStyle(
      fontSize: WsMetrics.fsCompact,
      color: color ?? ws.textMuted,
      fontFeatures: const [FontFeature.tabularFigures()],
    ),
  );

  /// A breakdown table: a `#` column, a name that takes the rest, then right-aligned figures.
  Widget _table(
    WsPalette ws, {
    required List<String> headers,
    required List<List<Widget>> rows,
  }) {
    Widget cell(int column, Widget child) => column == 0
        ? SizedBox(width: 22, child: child)
        : column == 1
        ? Expanded(
            child: Align(alignment: Alignment.centerLeft, child: child),
          )
        : SizedBox(
            width: 72,
            child: Align(alignment: Alignment.centerRight, child: child),
          );
    Widget row(List<Widget> cells, {required bool header}) => Container(
      padding: const EdgeInsets.symmetric(vertical: 7),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final (column, child) in cells.indexed) cell(column, child),
        ],
      ),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row([
          for (final label in headers)
            Text(
              label,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textFaint,
              ),
            ),
        ], header: true),
        for (final cells in rows) row(cells, header: false),
      ],
    );
  }

  static Color _series(WsPalette ws, int series) =>
      series >= otherSeries ? ws.textFaint : _seriesColours(ws)[series];

  static List<Color> _seriesColours(WsPalette ws) => [
    ws.data1,
    ws.data2,
    ws.data3,
  ];
}

/// The segmented control of the header and the breakdown: the chosen option is raised.
class _Segments<T> extends StatelessWidget {
  const _Segments({
    required this.options,
    required this.selected,
    required this.onSelect,
  });

  final List<(T, String)> options;
  final T selected;
  final ValueChanged<T> onSelect;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        border: Border.all(color: ws.border),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (value, label) in options)
            InkWell(
              borderRadius: BorderRadius.circular(4),
              onTap: () => onSelect(value),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: 2,
                ),
                decoration: BoxDecoration(
                  color: value == selected ? ws.canvas : null,
                  borderRadius: BorderRadius.circular(4),
                  border: value == selected
                      ? Border.all(color: ws.borderStrong)
                      : null,
                ),
                child: Text(
                  label,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: value == selected ? ws.text : ws.textMuted,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
