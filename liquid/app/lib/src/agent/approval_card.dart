import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show ScrollCacheExtent;

import '../controls.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'read_delay.dart';

/// A tool call waiting for approval: Enter allows once, Cmd+Enter for the session, Esc rejects.
class ApprovalCard extends StatefulWidget {
  const ApprovalCard({super.key, required this.request, required this.state});

  final BridgeRequest request;
  final SessionState state;

  @override
  State<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends State<ApprovalCard> with ReadDelay {
  final _focus = FocusNode(debugLabel: 'approval');
  final _code = ScrollController();

  /// The call's lines, worked out once, since a write can be long (ADR-0212): a file change's
  /// path and changed lines, any other call as its text.
  late final _lines = switch (widget.request.approval!) {
    BridgeApproval(:final edit?) => _editLines(edit),
    final approval => [
      (kind: _LineKind.call, text: approval.text, breaks: false),
    ],
  };

  @override
  void initState() {
    super.initState();
    // The card takes the keyboard as soon as it appears, so the decision needs no mouse.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _focus.dispose();
    _code.dispose();
    super.dispose();
  }

  /// Moves the call by a line, or by a page of its view.
  void _scroll(int direction, {bool page = false}) {
    if (!_code.hasClients) return;
    final position = _code.position;
    final step = page
        ? position.viewportDimension
        : _lineHeight(monoStyle(context));
    _code.jumpTo(
      (position.pixels + direction * step).clamp(
        position.minScrollExtent,
        position.maxScrollExtent,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    final id = widget.request.id;
    void decide(BridgeDecision decision) =>
        decided(() => state.decide(id, decision));
    final approval = widget.request.approval!;
    final ws = context.ws;
    final title = switch (approval.kind) {
      BridgeRequestKind.commandExecution => 'Run a shell command?',
      BridgeRequestKind.fileChange => 'Change files?',
      BridgeRequestKind.tool => 'Call ${approval.tool}?',
    };
    final keys = Keymap.of(context);
    final decisions = {
      Command.allowOnce: () => decide(BridgeDecision.allowOnce),
      Command.allowSession: () => decide(BridgeDecision.allowSession),
      Command.reject: () => decide(BridgeDecision.reject),
    };
    return CallbackShortcuts(
      bindings: {
        ...keys.bind(decisions),
        ...Fixed.scrollCall.bind([
          () => _scroll(-1),
          () => _scroll(1),
          () => _scroll(-1, page: true),
          () => _scroll(1, page: true),
        ]),
      },
      // A key pressed before the card was read decides nothing (see [ReadDelay]); a held key
      // never does.
      child: Focus(
        focusNode: _focus,
        child: InputCard(
          ring: ws.warningBorder,
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  StatusBadge(
                    text: '!',
                    color: ws.warning,
                    background: ws.warningSubtle,
                  ),
                  const SizedBox(width: WsMetrics.space4),
                  // A long tool name wraps rather than lose a part.
                  Expanded(
                    child: Text(
                      title,
                      style: Theme.of(context).textTheme.labelMedium,
                    ),
                  ),
                  // Any other tool is named in the title already.
                  if (approval.kind != BridgeRequestKind.tool) ...[
                    const SizedBox(width: WsMetrics.space4),
                    Text(
                      approval.tool,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ],
                ],
              ),
              const SizedBox(height: WsMetrics.space3),
              Expanded(
                child: _ScrollingCode(controller: _code, lines: _lines),
              ),
              const SizedBox(height: WsMetrics.space3),
              LayoutBuilder(
                builder: (context, row) {
                  final session = SecondaryButton(
                    label: _sessionLabel(approval),
                    shortcut: keys.label(Command.allowSession),
                    onPressed: () => decide(BridgeDecision.allowSession),
                  );
                  final hint = TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  );
                  // Short of room, the hint gives way before what the button covers.
                  final hinted =
                      row.maxWidth -
                          [
                            'Allow once',
                            _sessionLabel(approval),
                            'Reject',
                          ].fold(
                            0.0,
                            (sum, label) => sum + buttonWidth(context, label),
                          ) -
                          3 * WsMetrics.space3 >=
                      measureText('your draft is kept', hint).width;
                  return Row(
                    children: [
                      PrimaryButton(
                        label: 'Allow once',
                        shortcut: keys.label(Command.allowOnce),
                        onPressed: () => decide(BridgeDecision.allowOnce),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      if (hinted) ...[
                        session,
                        Expanded(
                          child: Text(
                            'your draft is kept',
                            textAlign: TextAlign.end,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: hint,
                          ),
                        ),
                      ] else
                        Expanded(
                          child: Align(
                            alignment: Alignment.centerLeft,
                            child: session,
                          ),
                        ),
                      const SizedBox(width: WsMetrics.space3),
                      SecondaryButton(
                        label: 'Reject',
                        shortcut: keys.label(Command.reject),
                        color: ws.danger,
                        onPressed: () => decide(BridgeDecision.reject),
                      ),
                    ],
                  );
                },
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// What allowing a call for the session covers: a shell command only as shown, any other tool
/// wholly (ADR-0209). A long tool name is cut short; the title names it in full.
String _sessionLabel(BridgeApproval approval) {
  if (approval.kind == BridgeRequestKind.commandExecution) {
    return 'Allow this command for this session';
  }
  final tool = approval.tool.characters;
  final shown = tool.length > 24 ? '${tool.take(23)}…' : '$tool';
  return 'Allow $shown for this session';
}

/// Code that scrolls inside a fixed height and says how many lines are below, so nothing is
/// hidden without a hint (approved approval mockup).
class _ScrollingCode extends StatefulWidget {
  const _ScrollingCode({required this.lines, required this.controller});

  /// The call's lines; their pieces are built only once they scroll into view.
  final List<_CodeLine> lines;
  final ScrollController controller;

  @override
  State<_ScrollingCode> createState() => _ScrollingCodeState();
}

/// Part of a line, at most [_pieceChars] long, since a text without spaces lays out in time
/// that grows with the square of its length. Its height is known when its line had to be cut.
typedef _Piece = ({int line, String text, bool last, double? height});

const _pieceChars = 2000;

class _ScrollingCodeState extends State<_ScrollingCode> {
  int _below = 0;
  int _last = 0;
  bool _counting = false;

  /// The pieces for the width, style and text scale they were cut at.
  List<_Piece> _pieces = const [];
  Object? _cutFor;

  @override
  void didUpdateWidget(_ScrollingCode old) {
    super.didUpdateWidget(old);
    if (!identical(old.lines, widget.lines)) _cutFor = null;
  }

  /// Counts what lies below once a layout is done. Lines not yet reached count by the call's
  /// lines, since lines below are built only as they scroll into view; within the last line,
  /// which may wrap, by the lines it shows as.
  void _laidOut(int last) {
    _last = last;
    if (_counting) return;
    _counting = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _counting = false;
      if (!mounted || !widget.controller.hasClients) return;
      final after = widget.controller.position.extentAfter;
      final line = _pieces[_last].line;
      final lines = widget.lines.length;
      final below = line < lines - 1
          ? lines - 1 - line
          : (after / _lineHeight(monoStyle(context))).ceil();
      if (below != _below) setState(() => _below = below);
    });
  }

  /// Cuts each line into pieces that wrap as the whole line would at `width` (approved
  /// mockup). A long line is laid out a piece at a time and cut before the last row the piece
  /// fills, since only that row could change with the characters after it.
  void _cut(BuildContext context, double width) {
    final base = DefaultTextStyle.of(context).style;
    final scaler = MediaQuery.textScalerOf(context);
    final key = (width, base.merge(monoStyle(context)), scaler);
    if (key == _cutFor) return;
    _cutFor = key;
    final painter = TextPainter(
      textDirection: TextDirection.ltr,
      textScaler: scaler,
    );
    _pieces = [];
    for (final (index, line) in widget.lines.indexed) {
      final text = _shown(line);
      if (text.length <= _pieceChars) {
        _pieces.add((line: index, text: text, last: true, height: null));
        continue;
      }
      final style = base.merge(_style(context, line));
      var start = 0;
      while (true) {
        var end = math.min(start + _pieceChars, text.length);
        final last = end == text.length;
        // A piece never ends inside a character's pair of UTF-16 units.
        if (!last && _leadSurrogate(text.codeUnitAt(end - 1))) end--;
        painter
          ..text = TextSpan(
            text: text.substring(start, end) + (last && line.breaks ? '↵' : ''),
            style: style,
          )
          ..layout(maxWidth: width - _inset(line));
        if (last) {
          _pieces.add((
            line: index,
            text: text.substring(start),
            last: true,
            height: painter.height,
          ));
          break;
        }
        final rows = painter.computeLineMetrics();
        var cut = painter
            .getLineBoundary(TextPosition(offset: end - start))
            .start;
        var kept = rows.length - 1;
        // A piece one row long stays whole.
        if (cut == 0) (cut, kept) = (end - start, rows.length);
        final piece = text.substring(start, start + cut);
        _pieces.add((
          line: index,
          // A row that ends with a line break shows as a piece without it.
          text: piece.endsWith('\n')
              ? piece.substring(0, piece.length - 1)
              : piece,
          last: false,
          height: rows
              .take(kept)
              .fold<double>(0, (sum, row) => sum + row.height),
        ));
        start += cut;
      }
    }
    painter.dispose();
  }

  Widget _piece(BuildContext context, int index) {
    final piece = _pieces[index];
    final line = widget.lines[piece.line];
    final ws = context.ws;
    final style = _style(context, line);
    Widget marked(Color background) => Container(
      color: background,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
      child: Text.rich(
        TextSpan(
          text: piece.text,
          children: [
            if (line.breaks && piece.last)
              TextSpan(
                text: '↵',
                style: TextStyle(color: ws.textFaint),
              ),
          ],
        ),
        style: style,
      ),
    );
    return switch (line.kind) {
      _LineKind.removed => marked(ws.diffDel),
      _LineKind.added => marked(ws.diffAdd),
      _ => Text(piece.text, style: style),
    };
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      decoration: BoxDecoration(
        color: ws.code,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      // The hint sits below the code, so it never covers a line of the call.
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: LayoutBuilder(
              builder: (context, box) {
                _cut(context, box.maxWidth - 2 * WsMetrics.space4);
                // No pieces are built ahead, so the last one laid out is the last one in view.
                return ListView.custom(
                  controller: widget.controller,
                  scrollCacheExtent: const ScrollCacheExtent.pixels(0),
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                    vertical: WsMetrics.space2,
                  ),
                  childrenDelegate: _Pieces(
                    _piece,
                    pieces: _pieces,
                    laidOut: _laidOut,
                  ),
                );
              },
            ),
          ),
          if (_below > 0)
            Padding(
              padding: const EdgeInsets.only(
                right: WsMetrics.space3,
                bottom: WsMetrics.space1,
              ),
              child: Text(
                '↓ $_below more line${_below == 1 ? '' : 's'}',
                textAlign: TextAlign.end,
                style: TextStyle(
                  color: ws.textFaint,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Pieces built on demand that report the last one each layout reached. The pieces of a line
/// that was cut have known heights, so below the last one reached, the extent is exact.
class _Pieces extends SliverChildBuilderDelegate {
  _Pieces(super.builder, {required this.pieces, required this.laidOut})
    : super(childCount: pieces.length);

  final List<_Piece> pieces;
  final void Function(int last) laidOut;

  @override
  void didFinishLayout(int firstIndex, int lastIndex) => laidOut(lastIndex);

  @override
  double? estimateMaxScrollOffset(
    int firstIndex,
    int lastIndex,
    double leadingScrollOffset,
    double trailingScrollOffset,
  ) {
    var extent = trailingScrollOffset;
    for (final piece in pieces.skip(lastIndex + 1)) {
      final height = piece.height;
      if (height == null) return null;
      extent += height;
    }
    return extent;
  }
}

bool _leadSurrogate(int unit) => unit >= 0xD800 && unit <= 0xDBFF;

double _lineHeight(TextStyle style) => measureText(' ', style).height;

/// The lines of `text`; a final line break ends the last line instead of starting another.
List<String> _lines(String text) => text.isEmpty
    ? const []
    : (text.endsWith('\n') ? text.substring(0, text.length - 1) : text).split(
        '\n',
      );

enum _LineKind { call, path, writesTo, note, removed, added }

/// A line of the call; `breaks` marks a line break that ends only its side of a replacement.
typedef _CodeLine = ({_LineKind kind, String text, bool breaks});

/// A file change as the Diff tab shows one: the path, then each replacement's removed and added
/// lines (approved mockup, ADR-0212). A path that does not say plainly which file pi writes is
/// followed by that file; a write says that it replaces whatever the file holds.
List<_CodeLine> _editLines(BridgeEdit edit) => [
  (kind: _LineKind.path, text: edit.path, breaks: false),
  if (edit.writesTo case final file?)
    (kind: _LineKind.writesTo, text: 'writes to $file', breaks: false),
  if (edit.wholeFile)
    (
      kind: _LineKind.note,
      text: edit.changes.every((change) => change.added.isEmpty)
          ? 'Empties the whole file'
          : 'Replaces the whole file',
      breaks: false,
    ),
  for (final change in edit.changes) ...[
    ..._side(_LineKind.removed, change.removed, change.added, edit.wholeFile),
    ..._side(_LineKind.added, change.added, change.removed, edit.wholeFile),
  ],
];

/// One side of a replacement. Unless a write replaces the whole file, its last line marks a line
/// break that ends only this side, so adding or removing one never looks like no change
/// (approved mockup).
List<_CodeLine> _side(_LineKind kind, String text, String other, bool write) {
  final lines = _lines(text);
  final breaks = !write && text.endsWith('\n') && !other.endsWith('\n');
  return [
    for (final (index, line) in lines.indexed)
      (kind: kind, text: line, breaks: breaks && index == lines.length - 1),
  ];
}

/// A line as drawn, with the sign that marks a changed one.
String _shown(_CodeLine line) => switch (line.kind) {
  _LineKind.removed => '− ${line.text}',
  _LineKind.added => '+ ${line.text}',
  _ => line.text,
};

TextStyle _style(BuildContext context, _CodeLine line) {
  final style = monoStyle(context);
  return switch (line.kind) {
    _LineKind.path => style.copyWith(fontWeight: WsMetrics.fwSemibold),
    _LineKind.writesTo => style.copyWith(color: context.ws.textFaint),
    _LineKind.note => style.copyWith(color: context.ws.warning),
    _ => style,
  };
}

/// The room a changed line's background takes beside its text.
double _inset(_CodeLine line) =>
    line.kind == _LineKind.removed || line.kind == _LineKind.added
    ? 2 * WsMetrics.space1
    : 0;
