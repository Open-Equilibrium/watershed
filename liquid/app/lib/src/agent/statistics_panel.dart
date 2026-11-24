import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../format.dart';
import '../session/session_statistics.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'segment_bar.dart';
import 'session_card_statistics.dart';
import 'statistics_chart.dart';
import 'statistics_table.dart';

const askStart = 'About this session: ';

enum _View { cost, tokens, time }

enum _Breakdown { model, tool, turn }

class StatisticsPanel extends StatefulWidget {
  const StatisticsPanel({
    super.key,
    required this.state,
    required this.onClose,
    required this.onAsk,
  });

  final SessionState state;
  final VoidCallback onClose;

  final VoidCallback onAsk;

  @override
  State<StatisticsPanel> createState() => _StatisticsPanelState();
}

class _StatisticsPanelState extends State<StatisticsPanel> {
  _View _view = _View.cost;
  _Breakdown _breakdown = _Breakdown.tool;

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
              widget.state.statisticsProblem,
              widget.state.status,
              widget.state.turns,
            ]),
            builder: (context, _) {
              final stored = widget.state.statistics.value;
              final running = turnRuns(widget.state);
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _header(ws),
                  Expanded(
                    child: stored == null
                        ? _unread(ws, widget.state.statisticsProblem.value)
                        : SingleChildScrollView(
                            padding: const EdgeInsets.fromLTRB(20, 16, 20, 20),
                            child: _body(
                              ws,
                              SessionStatistics(stored, running: running),
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

  Widget _unread(WsPalette ws, ({String title, String text})? problem) =>
      Padding(
        padding: const EdgeInsets.all(20),
        child: problem == null
            ? Text(
                'Reading the statistics…',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textFaint,
                ),
              )
            : Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 4,
                children: [
                  _heading(ws, problem.title),
                  Text(
                    problem.text,
                    style: TextStyle(
                      fontSize: WsMetrics.fsSmall,
                      color: ws.textMuted,
                    ),
                  ),
                ],
              ),
      );

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
        Segments<_View>(
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

  Widget _body(WsPalette ws, SessionStatistics statistics) {
    final children = switch (_view) {
      _View.cost => _costView(ws),
      _View.tokens => _tokensView(ws, statistics),
      _View.time => _timeView(ws, statistics),
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 20,
      children: [...children, _breakdownSection(ws, statistics), _askRow(ws)],
    );
  }

  List<Widget> _costView(WsPalette ws) => [
    Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      spacing: 4,
      children: [
        _heading(ws, 'T3 Code reports no cost'),
        Text(
          'Tokens and times are counted from the session and are on their views.',
          style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted),
        ),
      ],
    ),
  ];

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
                    fontSize: WsMetrics.fsH1,
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

  Widget _byType(
    WsPalette ws,
    String title, {
    required int input,
    required int cacheRead,
    required int cacheWrite,
    required int output,
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
      StatisticsLegend([
        ('Input', ws.data1, formatTokens(input)),
        ('Cache read', ws.data2, formatTokens(cacheRead)),
        ('Cache write', ws.textFaint, formatTokens(cacheWrite)),
        ('Output', ws.data3, formatTokens(output)),
      ]),
    ],
  );

  List<Widget> _tokensView(WsPalette ws, SessionStatistics statistics) {
    final usage = statistics.usage;
    return [
      _top(
        ws,
        title: 'Tokens per turn',
        hero: formatTokens(statistics.tokens),
        caption: _responsesAndTurns(statistics),
        who: [
          for (final model in statistics.models)
            _who(
              ws,
              colour: _series(ws, model.series),
              name: model.name,
              value: formatTokens(model.tokens),
              caption:
                  '${formatShare(model.tokens, statistics.tokens)} of tokens · '
                  '${plural(model.responses, 'response')}',
            ),
        ],
        chart: _chart(
          ws,
          statistics,
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
      _totals(ws, statistics),
      _byType(
        ws,
        'Tokens by type',
        input: usage.input,
        cacheRead: usage.cacheRead,
        cacheWrite: usage.cacheWrite,
        output: usage.output,
      ),
    ];
  }

  List<Widget> _timeView(WsPalette ws, SessionStatistics statistics) {
    final working = statistics.workingMs;
    final tool = statistics.toolMs;
    String share(int part) => working == 0 ? '—' : formatShare(part, working);
    return [
      _top(
        ws,
        title: 'Time per turn',
        hero: formatDuration(working),
        caption: [
          plural(statistics.turns.length, 'turn'),
          if (statistics.spanMs case final span?)
            '${formatDuration(span)} from first to last entry',
        ].join(' · '),
        who: [
          _who(
            ws,
            colour: ws.data1,
            name: 'Model',
            value: '—',
            caption: 'T3 times no single model request',
          ),
          _who(
            ws,
            colour: ws.data2,
            name: 'Running tools',
            value: tool == null ? '—' : formatDuration(tool),
            caption: tool == null
                ? 'Not recorded for turns without timing entries'
                : '${share(tool)} · ${plural(statistics.toolCalls, 'call')}',
          ),
        ],
        chart: _chart(
          ws,
          statistics,
          columnParts: (turn) => [turn.workingMs ?? 0],
          colours: [ws.data1],
          axis: formatDuration,
          title: (turn) =>
              'Turn ${turn.index} · ${formatDuration(turn.workingMs ?? 0)}',
          rows: (turn) => [
            ChartTipRow(ws.data2, 'Tools', formatDuration(turn.toolMs ?? 0)),
          ],
          notes: (turn) => [
            turn.prompt,
            if (turn.toolMs == null) 'Timing not recorded for this turn',
          ],
        ),
      ),
    ];
  }

  String _responsesAndTurns(SessionStatistics statistics) =>
      '${plural(statistics.responses, 'response')} · '
      '${plural(statistics.turns.length, 'turn')} · as T3 Code reports it';

  TurnChart _chart(
    WsPalette ws,
    SessionStatistics statistics, {
    required List<int> Function(TurnStatistics turn) columnParts,
    required List<Color> colours,
    required String Function(int value) axis,
    required List<ChartTipRow> Function(TurnStatistics turn) rows,
    required List<String> Function(TurnStatistics turn) notes,
    String Function(TurnStatistics turn)? title,
  }) => TurnChart(
    colors: colours,
    axis: axis,
    columns: [
      for (final turn in statistics.turns)
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

  Widget _totals(WsPalette ws, SessionStatistics statistics) {
    final usage = statistics.usage;
    final prompt = promptTokens(usage);
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

  Widget _breakdownSection(WsPalette ws, SessionStatistics statistics) =>
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: 4,
        children: [
          Row(
            children: [
              _heading(ws, 'Breakdown'),
              const Spacer(),
              Segments<_Breakdown>(
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
            _Breakdown.model => _modelTable(ws, statistics),
            _Breakdown.tool => _toolTable(ws, statistics),
            _Breakdown.turn => _turnTable(ws, statistics),
          },
        ],
      );

  Widget _modelTable(WsPalette ws, SessionStatistics statistics) =>
      StatisticsTable(
        headers: const ['#', 'Model', 'Responses', 'Tokens'],
        rows: [
          for (final (index, model) in statistics.models.indexed)
            [
              _index(ws, index),
              _name(ws, model.name),
              _number(ws, '${model.responses}'),
              _number(ws, formatTokens(model.tokens)),
            ],
        ],
      );

  Widget _toolTable(WsPalette ws, SessionStatistics statistics) {
    final tools = statistics.tools;
    final largest = tools.fold(
      1,
      (most, tool) => tool.total > most ? tool.total : most,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 8,
      children: [
        StatisticsTable(
          headers: const ['#', 'Tool', 'Calls', 'Failed', 'Time'],
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
                    fraction: tool.total / largest,
                    segments: [
                      BarSegment(tool.succeeded, ws.textFaint),
                      BarSegment(tool.failed, ws.danger),
                      BarSegment(tool.cancelled, ws.borderStrong),
                      BarSegment(tool.running, ws.textFaint, hatched: true),
                    ],
                  ),
                ),
                _number(ws, '${tool.total}'),
                _number(
                  ws,
                  '${tool.failed}',
                  color: tool.failed > 0 ? ws.danger : ws.textFaint,
                ),
                _number(
                  ws,
                  tool.durationMs == null
                      ? '—'
                      : formatDuration(tool.durationMs!),
                ),
              ],
          ],
        ),
        StatisticsLegend([
          ('Succeeded', ws.textFaint, ''),
          ('Failed', ws.danger, ''),
          ('Cancelled', ws.borderStrong, ''),
        ]),
      ],
    );
  }

  void _selectTurn(TurnStatistics turn) {
    widget.state.scrollChatTo.value = turn.stats.number;
    widget.onClose();
  }

  Widget _turnTable(WsPalette ws, SessionStatistics statistics) {
    final longest = statistics.turns.fold(
      1,
      (most, turn) => (turn.workingMs ?? 0) > most ? turn.workingMs! : most,
    );
    return StatisticsTable(
      headers: const ['#', 'Turn', 'Time', 'Tokens'],
      onSelect: (index) => _selectTurn(statistics.turns[index]),
      rows: [
        for (final turn in statistics.turns)
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
                      segments: [BarSegment(turn.workingMs!, ws.data1)],
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
              turn.workingMs == null ? '—' : formatDuration(turn.workingMs!),
            ),
            _number(ws, formatTokens(turn.tokens)),
          ],
      ],
    );
  }

  Widget _askRow(WsPalette ws) => Row(
    spacing: 16,
    children: [
      SecondaryButton(
        label: 'Ask the agent about this session…',
        onPressed: _ask,
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
          ColourDot(colour),
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

  static Color _series(WsPalette ws, int series) =>
      series >= otherSeries ? ws.textFaint : _seriesColours(ws)[series];

  static List<Color> _seriesColours(WsPalette ws) => [
    ws.data1,
    ws.data2,
    ws.data3,
  ];
}
