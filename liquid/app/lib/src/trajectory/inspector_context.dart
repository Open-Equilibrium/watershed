import 'package:flutter/material.dart';

import '../format.dart';
import '../rust/api/history.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'context_track.dart';
import 'inspector_parts.dart';
import 'track_cell.dart';

/// The request that read a step: its size against the window, its parts
/// with the step's own hatched, the largest parts and then every part.
class ContextTab extends StatelessWidget {
  const ContextTab({
    super.key,
    required this.records,
    required this.index,
    required this.request,
    required this.detail,
    required this.unread,
    required this.windowOf,
  });

  final List<BridgeRecord> records;
  final int index;
  final int? request;
  final BridgeContextDetail? detail;
  final String? unread;
  final int? Function(String model) windowOf;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final request = this.request;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final read = request == null ? null : records[request];
    if (read == null || read.usage == null || read.context == null) {
      return Center(
        child: Text(
          read == null
              ? 'No request read this step yet.'
              : 'This request reported no tokens.',
          style: faint,
        ),
      );
    }
    final record = records[index];
    final message = record.kind == BridgeRecordKind.message;
    final detail = this.detail;
    final stepChars = message
        ? [
            for (final part
                in detail?.messages ?? const <BridgeContextMessage>[])
              if (part.fresh && part.inContext) part.chars,
          ].fold(0, (sum, chars) => sum + chars)
        : record.addedChars;
    final parts = requestParts(read, detail: detail, stepChars: stepChars);
    final number = requestNumber(records, request!);
    final relation = switch (record.kind) {
      BridgeRecordKind.message => 'what this response read',
      BridgeRecordKind.tool => 'the request that read this result',
      BridgeRecordKind.user => 'the request this prompt went into',
    };
    // The window of the model that read it, which an earlier model switch may change.
    final window = windowOf(read.model);
    final compaction = window == null ? null : compactionPoint(window);
    final stepLabel = message ? 'New since the last request' : 'This step';
    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _header(ws, read, number, relation, parts, window, compaction),
          const SizedBox(height: WsMetrics.space3),
          _Gauge(
            parts: parts,
            scale: (window ?? parts.total).toDouble(),
            compaction: compaction,
          ),
          const SizedBox(height: WsMetrics.space3),
          _legends(ws, parts, stepLabel),
          if (parts.largest.isNotEmpty) ...[
            const SizedBox(height: WsMetrics.space2),
            Text(
              'Largest in request #$number: '
              '${parts.largest.map((part) => '${part.$1} ≈${formatCompact(part.$2)}').join(' · ')}',
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: faint,
            ),
          ],
          const SizedBox(height: WsMetrics.space4),
          if (detail != null)
            _ContextList(record: read, detail: detail, unread: unread)
          else
            Text(switch (unread) {
              final reason? => 'Could not read what the model saw: $reason',
              null => 'Reading what the model saw…',
            }, style: faint),
        ],
      ),
    );
  }

  /// The request's number, its reader and model, and its size against the window.
  Widget _header(
    WsPalette ws,
    BridgeRecord read,
    int number,
    String relation,
    RequestParts parts,
    int? window,
    int? compaction,
  ) {
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Row(
      children: [
        Expanded(
          child: Text(
            [
              'Request #$number',
              relation,
              if (read.model.isNotEmpty) read.model,
            ].join(' · '),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: WsMetrics.fsSmall),
          ),
        ),
        const SizedBox(width: WsMetrics.space4),
        Text(
          [
            window == null
                ? '${formatCompact(parts.total)} tokens'
                : '${formatCompact(parts.total)} of ${formatWindow(window)}',
            if (window != null) '${(parts.total * 100 / window).round()} %',
            if (compaction != null)
              'pi compacts at ${formatCompact(compaction)}',
          ].join(' · '),
          style: faint.copyWith(
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
      ],
    );
  }

  /// The request's parts by color, with the inspected step hatched.
  Widget _legends(WsPalette ws, RequestParts parts, String stepLabel) {
    return Wrap(
      spacing: WsMetrics.space5,
      runSpacing: WsMetrics.space1,
      children: [
        _Legend(ws.data2, 'System prompt', parts.system),
        if (parts.projectContext > 0)
          _Legend(ws.warning, 'AGENTS.md', parts.projectContext),
        _Legend(ws.data3, 'Tools', parts.tools),
        _Legend(ws.data1, 'Carried over', parts.carried),
        _Legend(
          ws.accent,
          stepLabel,
          parts.step,
          hatched: true,
          share: parts.total == 0
              ? null
              : '(${(parts.step * 100 / parts.total).round()} % of the request)',
        ),
      ],
    );
  }
}

/// A request's parts against the window, the inspected step hatched, and pi's compaction point.
class _Gauge extends StatelessWidget {
  const _Gauge({
    required this.parts,
    required this.scale,
    required this.compaction,
  });

  final RequestParts parts;
  final double scale;
  final int? compaction;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.maxWidth;
        double x(int tokens) => scale <= 0 ? 0 : tokens / scale * width;
        Widget part(int tokens, Color color) =>
            Container(width: x(tokens), color: color);
        return SizedBox(
          height: 14,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(
                child: Container(
                  decoration: BoxDecoration(
                    color: ws.surface,
                    borderRadius: BorderRadius.circular(4),
                    border: Border.all(color: ws.borderStrong),
                  ),
                ),
              ),
              ClipRRect(
                borderRadius: BorderRadius.circular(4),
                child: Row(
                  children: [
                    part(parts.system, ws.data2),
                    part(parts.projectContext, ws.warning),
                    part(parts.tools, ws.data3),
                    part(parts.carried, ws.data1),
                    SizedBox(
                      width: x(parts.step),
                      child: CustomPaint(
                        painter: _Hatch(ws.accent),
                        size: Size.infinite,
                      ),
                    ),
                  ],
                ),
              ),
              if (compaction case final compaction?)
                Positioned(
                  left: x(compaction) - 1,
                  top: -2,
                  bottom: -2,
                  child: Container(width: 2, color: ws.warning),
                ),
            ],
          ),
        );
      },
    );
  }
}

class _Hatch extends CustomPainter {
  _Hatch(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(
      Offset.zero & size,
      Paint()..color = color.withValues(alpha: 0.45),
    );
    final stripe = Paint()
      ..color = color
      ..strokeWidth = 2;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    for (var x = -size.height; x < size.width; x += 6) {
      canvas.drawLine(
        Offset(x, size.height),
        Offset(x + size.height, 0),
        stripe,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_Hatch old) => old.color != color;
}

/// A colored square and a part's estimated tokens, with its share of the request if given.
class _Legend extends StatelessWidget {
  const _Legend(
    this.color,
    this.label,
    this.tokens, {
    this.hatched = false,
    this.share,
  });

  final Color color;
  final String label;
  final int tokens;
  final bool hatched;
  final String? share;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      SizedBox(
        width: 9,
        height: 9,
        child: hatched
            ? CustomPaint(painter: _Hatch(color))
            : DecoratedBox(
                decoration: BoxDecoration(
                  color: color,
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
      ),
      const SizedBox(width: WsMetrics.space2),
      Flexible(
        child: Text.rich(
          TextSpan(
            text: '$label ≈${formatCompact(tokens)}',
            children: [
              if (share case final share?)
                TextSpan(
                  text: ' $share',
                  style: TextStyle(color: context.ws.textFaint),
                ),
            ],
          ),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            color: context.ws.textMuted,
            fontSize: WsMetrics.fsSmall,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
      ),
    ],
  );
}

/// Every part of what the model saw for one request.
class _ContextList extends StatelessWidget {
  const _ContextList({required this.record, required this.detail, this.unread});

  final BridgeRecord record;
  final BridgeContextDetail? detail;
  final String? unread;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final detail = this.detail;
    final saw = record.context;
    if (detail == null || saw == null) {
      return Center(
        child: Text(switch (unread) {
          final reason? => 'Could not read what the model saw: $reason',
          null => 'Reading what the model saw…',
        }, style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall)),
      );
    }
    final split = tokenSplit(record);
    int total(Iterable<int> chars) =>
        chars.fold(0, (sum, value) => sum + value);
    final sectionChars = total(detail.sections.map((part) => part.chars));
    final toolChars = total(detail.tools.map((part) => part.chars));
    final messageChars = total([
      for (final message in detail.messages)
        if (message.inContext) message.chars,
    ]);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _ContextGroup('System prompt', split[0]),
        for (final section in detail.sections)
          _ContextPart(
            record: record,
            label: section.name,
            chars: section.chars,
            groupChars: sectionChars,
            color: ws.data2,
          ),
        _ContextGroup('Tools · ${detail.tools.length}', split[1]),
        for (final tool in detail.tools)
          _ContextPart(
            record: record,
            label: tool.name,
            chars: tool.chars,
            groupChars: toolChars,
            color: ws.data3,
          ),
        _ContextGroup('Messages · ${saw.messages}', split[2]),
        for (final (index, message) in detail.messages.indexed)
          _ContextPart(
            record: record,
            label: '${index + 1} · ${message.label}',
            chars: message.chars,
            groupChars: messageChars,
            color: ws.data1,
            fresh: message.fresh,
            gone: !message.inContext,
          ),
        const SizedBox(height: WsMetrics.space3),
        Text(
          'Shares estimated from characters; the total is the provider\'s measured input. '
          'Compacted or removed entries appear crossed out.',
          style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
        ),
      ],
    );
  }
}

/// A group's heading with the group's estimated tokens.
class _ContextGroup extends StatelessWidget {
  const _ContextGroup(this.title, this.tokens);

  final String title;
  final int tokens;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: WsMetrics.space2),
    child: Row(
      children: [
        Expanded(
          child: Text(
            title,
            style: const TextStyle(
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwSemibold,
            ),
          ),
        ),
        Text(
          '≈ ${formatCount(tokens)}',
          style: const TextStyle(
            fontSize: WsMetrics.fsSmall,
            fontWeight: WsMetrics.fwSemibold,
            fontFeatures: [FontFeature.tabularFigures()],
          ),
        ),
      ],
    ),
  );
}

/// One part of a request: its name, its bar against the group's total, and its estimated tokens.
class _ContextPart extends StatelessWidget {
  const _ContextPart({
    required this.record,
    required this.label,
    required this.chars,
    required this.groupChars,
    required this.color,
    this.fresh = false,
    this.gone = false,
  });

  final BridgeRecord record;
  final String label;
  final int chars;
  final int groupChars;
  final Color color;
  final bool fresh;
  final bool gone;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return SizedBox(
      height: 18,
      child: Row(
        children: [
          Expanded(
            child: Row(
              children: [
                Flexible(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: gone ? ws.textFaint : ws.textMuted,
                      fontSize: WsMetrics.fsSmall,
                      decoration: gone ? TextDecoration.lineThrough : null,
                    ),
                  ),
                ),
                if (fresh) ...[
                  const SizedBox(width: WsMetrics.space1),
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 4),
                    decoration: BoxDecoration(
                      color: ws.accentSubtle,
                      borderRadius: BorderRadius.circular(3),
                    ),
                    child: Text(
                      'new',
                      style: TextStyle(
                        color: ws.accent,
                        fontSize: WsMetrics.fsCaption,
                      ),
                    ),
                  ),
                ],
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space4),
          Container(
            width: 120,
            height: 5,
            decoration: BoxDecoration(
              color: ws.hover,
              borderRadius: BorderRadius.circular(3),
            ),
            alignment: Alignment.centerLeft,
            child: FractionallySizedBox(
              widthFactor: groupChars == 0 || gone ? 0 : chars / groupChars,
              child: Container(
                decoration: BoxDecoration(
                  color: color,
                  borderRadius: BorderRadius.circular(3),
                ),
              ),
            ),
          ),
          SizedBox(
            width: 64,
            child: Text(
              gone ? '–' : '≈ ${formatCount(tokensOf(record, chars))}',
              textAlign: TextAlign.right,
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsSmall,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
