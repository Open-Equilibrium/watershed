import 'package:flutter/material.dart';

import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';

/// The colors, sign and number of one side of a line.
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
      // Muted, as faint lacks contrast on the strong diff colors.
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

/// A line's code, colored lightly, with a faint ␍ for a Windows line end and a faint … where
/// a long line is cut; a hunk header in the faint small style.
Widget _code(BuildContext context, DiffLine line) {
  final faint = context.ws.textFaint;
  final (:text, :cut) = shownText(line.text);
  if (line.kind == DiffKind.hunk) {
    return Text(
      cut ? '$text…' : text,
      style: monoStyle(
        context,
        color: faint,
      ).copyWith(fontSize: WsMetrics.fsSmall),
    );
  }
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

/// One line of the unified view: its old and new numbers, sign and code.
class UnifiedLineRow extends StatelessWidget {
  const UnifiedLineRow({super.key, required this.line});

  static const height = 22.0;

  /// Width of the two line numbers and the sign before the code.
  static const gutter = 44.0 + 44.0 + 20.0;

  /// Width of one side's line number and sign in the split view.
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

/// One row of the split view: the line before the run on the left, after it on the right.
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
      // A side without a line, beside one only the other side has.
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

final _token = RegExp(
  r'(//.*$)|("(?:[^"\\]|\\.)*"|'
  r"'(?:[^'\\]|\\.)*')"
  r'|\b(import|from|export|const|let|var|async|await|return|if|else|function|fn|pub|use|mut|impl|struct|enum|match|class|new)\b|\b(\d[\d_.]*)\b',
);

/// Light syntax colors for common languages; unknown text stays plain.
TextSpan highlight(BuildContext context, String code) {
  final ws = context.ws;
  final base = monoStyle(context);
  final spans = <TextSpan>[];
  var start = 0;
  for (final match in _token.allMatches(code)) {
    if (match.start > start) {
      spans.add(TextSpan(text: code.substring(start, match.start)));
    }
    final color = match.group(1) != null
        ? ws.synCom
        : match.group(2) != null
        ? ws.synStr
        : match.group(3) != null
        ? ws.synKw
        : ws.synNum;
    spans.add(
      TextSpan(
        text: match.group(0),
        style: TextStyle(color: color),
      ),
    );
    start = match.end;
  }
  if (start < code.length) spans.add(TextSpan(text: code.substring(start)));
  return TextSpan(style: base, children: spans);
}
