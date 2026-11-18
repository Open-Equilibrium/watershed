import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../highlight.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';

({Color? background, Widget number, Widget sign}) _lineParts(
  BuildContext context,
  DiffLine line,
  int? value,
) {
  final ws = context.ws;
  final (background, gutter, sign) = switch (line.kind) {
    DiffKind.added => (ws.diffAdd, ws.diffAddStrong, '+'),
    DiffKind.removed => (ws.diffDel, ws.diffDelStrong, '−'),
    DiffKind.hunk => (ws.subtle, ws.subtle, ''),
    DiffKind.context => (null, null, ''),
  };
  return (
    background: background,
    number: Container(
      width: 44,
      color: gutter,
      padding: const EdgeInsets.only(right: WsMetrics.space4),
      alignment: Alignment.centerRight,
      child: Text(
        value?.toString() ?? '',
        style: TextStyle(
          color: ws.textMuted,
          fontSize: WsMetrics.fsSmall,
          fontFeatures: const [FontFeature.tabularFigures()],
        ),
      ),
    ),
    sign: SizedBox(
      width: 20,
      child: Text(
        sign,
        textAlign: TextAlign.center,
        style: TextStyle(
          color: line.kind == DiffKind.added ? ws.success : ws.danger,
        ),
      ),
    ),
  );
}

Widget _code(BuildContext context, DiffLine line) {
  final faint = context.ws.textFaint;
  if (line.kind == DiffKind.hunk) {
    return Text(
      line.text,
      style: monoStyle(
        context,
        color: faint,
      ).copyWith(fontSize: WsMetrics.fsSmall),
    );
  }
  final (:text, :cut) = shownText(line.text);
  final crlf = !cut && text.endsWith('\r');
  final end = cut
      ? '…'
      : crlf
      ? '␍'
      : null;
  final shown = highlight(
    context,
    crlf ? text.substring(0, text.length - 1) : text,
  );
  return Text.rich(
    end == null
        ? shown
        : TextSpan(
            style: shown.style,
            children: [
              ...?shown.children,
              TextSpan(
                text: end,
                style: TextStyle(color: faint),
              ),
            ],
          ),
    maxLines: 1,
    overflow: TextOverflow.clip,
    softWrap: false,
  );
}

class UnifiedLineRow extends StatelessWidget {
  const UnifiedLineRow({super.key, required this.line});

  static const height = 22.0;

  static double heightIn(TextScaler scaler) => math.max(
    height,
    scaler.scale(WsMetrics.fsMono) * WsMetrics.lhMono / WsMetrics.fsMono,
  );

  static const gutter = 44.0 + 44.0 + 20.0;

  static const side = 44.0 + 20.0;

  final DiffLine line;

  @override
  Widget build(BuildContext context) {
    final old = _lineParts(context, line, line.oldNumber);
    final now = _lineParts(context, line, line.newNumber);
    return Container(
      color: old.background,
      child: Row(
        children: [
          old.number,
          now.number,
          old.sign,
          Expanded(child: _code(context, line)),
        ],
      ),
    );
  }
}

class SplitLineRow extends StatelessWidget {
  const SplitLineRow({super.key, required this.row});

  final SplitRow row;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    if (row.hunk case final hunk?) {
      return Container(
        color: ws.subtle,
        padding: const EdgeInsets.only(left: UnifiedLineRow.side),
        alignment: Alignment.centerLeft,
        child: _code(context, hunk),
      );
    }
    Widget side(DiffLine? line, {required bool before}) {
      if (line == null) return Expanded(child: ColoredBox(color: ws.subtle));
      final parts = _lineParts(
        context,
        line,
        before ? line.oldNumber : line.newNumber,
      );
      return Expanded(
        child: Container(
          color: parts.background,
          child: Row(
            children: [
              parts.number,
              parts.sign,
              Expanded(child: _code(context, line)),
            ],
          ),
        ),
      );
    }

    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        side(row.left, before: true),
        VerticalDivider(width: 1, color: ws.border),
        side(row.right, before: false),
      ],
    );
  }
}
