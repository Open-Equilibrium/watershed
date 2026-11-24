import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/drawing.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'board.dart';
import 'rough.dart';
import 'board_controller.dart';
import 'board_painter.dart';

class BoardToolBars extends StatelessWidget {
  const BoardToolBars({super.key, required this.board});

  final BoardController board;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([boardTools, board]),
    builder: (context, _) {
      final options = _options(context);
      return Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (options.isNotEmpty) ...[
            _Bar(key: const Key('whiteboard options'), children: options),
            const SizedBox(height: 6),
          ],
          _Bar(
            key: const Key('whiteboard tools'),
            children: [
              for (final (tool, icon, name) in const [
                (BoardTool.lasso, LucideIcons.lasso, 'Lasso'),
                (BoardTool.pen, LucideIcons.pen, 'Pen'),
                (BoardTool.highlighter, LucideIcons.highlighter, 'Highlighter'),
                (BoardTool.eraser, LucideIcons.eraser, 'Eraser'),
                (BoardTool.shape, LucideIcons.shapes, 'Shape'),
                (BoardTool.text, LucideIcons.type, 'Text'),
              ])
                _IconButton(
                  key: ValueKey('tool ${tool.name}'),
                  icon: icon,
                  label: name,
                  on: boardTools.tool == tool,
                  onTap: () => boardTools.tool = tool,
                ),
              const _Separator(),
              _StyleSwitch(board: board),
            ],
          ),
        ],
      );
    },
  );

  List<Widget> _options(BuildContext context) {
    final tools = boardTools;
    final tool = tools.tool;
    final selection = board.selection;
    List<Widget> colors(
      BridgeInkColor on,
      ValueChanged<BridgeInkColor> pick, {
      List<BridgeInkColor> from = inkColors,
    }) => [
      for (final color in from)
        _Swatch(
          key: ValueKey('color ${color.name}'),
          color: inkColor(context.ws, color),
          on: color == on,
          onTap: () => pick(color),
        ),
    ];
    List<Widget> widths(int on, ValueChanged<int> pick, {bool dots = false}) =>
        [
          for (var width = 0; width < 3; width++)
            _Width(
              key: ValueKey('width $width'),
              width: width,
              dot: dots,
              on: width == on,
              onTap: () => pick(width),
            ),
        ];
    void set(BoardTool tool, {BridgeInkColor? color, int? width}) =>
        tools.update(() {
          if (color != null) tools.colors[tool] = color;
          if (width != null) tools.widths[tool] = width;
        });
    switch (tool) {
      case BoardTool.pen:
        return [
          _Segments(
            labels: const ['Ball', 'Fountain', 'Brush'],
            on: tools.pen.index,
            onTap: (at) => tools.update(() => tools.pen = BridgePen.values[at]),
          ),
          const _Separator(),
          ...colors(tools.colorOf(tool), (color) => set(tool, color: color)),
          const _Separator(),
          ...widths(tools.widthOf(tool), (width) => set(tool, width: width)),
        ];
      case BoardTool.shape:
        return [
          for (final (kind, icon, name) in const [
            (ShapeKind.snap, LucideIcons.lineSquiggle, 'Snap a drawn shape'),
            (ShapeKind.rectangle, LucideIcons.square, 'Rectangle'),
            (ShapeKind.ellipse, LucideIcons.circle, 'Ellipse'),
            (ShapeKind.line, LucideIcons.slash, 'Line'),
            (ShapeKind.arrow, LucideIcons.moveUpRight, 'Arrow'),
          ])
            _IconButton(
              key: ValueKey('shape ${kind.name}'),
              icon: icon,
              label: name,
              on: tools.shape == kind,
              onTap: () => tools.update(() => tools.shape = kind),
            ),
          const _Separator(),
          ...colors(tools.colorOf(tool), (color) => set(tool, color: color)),
          const _Separator(),
          ...widths(tools.widthOf(tool), (width) => set(tool, width: width)),
        ];
      case BoardTool.highlighter:
        return [
          ...colors(
            tools.colorOf(tool),
            (color) => set(tool, color: color),
            from: highlightColors,
          ),
          const _Separator(),
          ...widths(tools.widthOf(tool), (width) => set(tool, width: width)),
        ];
      case BoardTool.eraser:
        return [
          _Segments(
            labels: const ['Partial', 'Whole'],
            on: tools.wholeEraser ? 1 : 0,
            onTap: (at) => tools.update(() => tools.wholeEraser = at == 1),
          ),
          const _Separator(),
          _Segments(
            labels: const ['All ink', 'Highlighter only'],
            on: tools.highlighterOnly ? 1 : 0,
            onTap: (at) => tools.update(() => tools.highlighterOnly = at == 1),
          ),
          const _Separator(),
          ...widths(
            tools.widthOf(tool),
            (width) => set(tool, width: width),
            dots: true,
          ),
        ];
      case BoardTool.text:
        return [
          ...colors(tools.colorOf(tool), (color) => set(tool, color: color)),
          const _Separator(),
          _Segments(
            labels: const ['S', 'M', 'L'],
            on: tools.widthOf(tool),
            onTap: (width) => set(tool, width: width),
          ),
        ];
      case BoardTool.lasso when selection.isNotEmpty:
        final first = selection.first;
        final lines = selection.where(
          (element) =>
              element.kind == BridgeElementKind.shape &&
              element.shape == BridgeShape.line,
        );
        void change({
          BridgeInkColor? color,
          int? width,
          bool? arrowhead,
        }) => unawaited(
          board.edit(
            change: [
              for (final element in selection)
                changeOf(
                  element,
                  frame: width != null && element.kind == BridgeElementKind.text
                      ? Rect.fromLTWH(
                          element.x,
                          element.y,
                          element.w,
                          BoardPainter.textHeight(
                            element.text,
                            element.w,
                            width,
                            board.handDrawn,
                          ),
                        )
                      : null,
                  color: color,
                  width: width,
                  arrowhead:
                      element.kind == BridgeElementKind.shape &&
                          element.shape == BridgeShape.line
                      ? arrowhead
                      : null,
                ),
            ],
          ),
        );
        return [
          ...colors(first.color, (color) => change(color: color)),
          const _Separator(),
          ...widths(first.width, (width) => change(width: width)),
          if (lines.isNotEmpty) ...[
            const _Separator(),
            _Segments(
              labels: const ['No arrowhead', 'Arrowhead'],
              on: lines.first.arrowhead ? 1 : 0,
              onTap: (at) => change(arrowhead: at == 1),
            ),
          ],
          const _Separator(),
          _IconButton(
            key: const Key('whiteboard delete'),
            icon: LucideIcons.trash,
            label: 'Delete',
            on: false,
            onTap: () => unawaited(board.edit(remove: [...board.selected])),
          ),
        ];
      case BoardTool.lasso:
        return const [];
    }
  }
}

class ZoomBar extends StatelessWidget {
  const ZoomBar({super.key, required this.board, required this.size});

  final BoardController board;
  final Size Function() size;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: board,
    builder: (context, _) => _Bar(
      key: const Key('whiteboard zoom'),
      children: [
        _IconButton(
          key: const Key('zoom out'),
          icon: LucideIcons.minus,
          label: 'Zoom out',
          on: false,
          onTap: () => board.zoomTo(board.zoom / 1.25, size()),
        ),
        SizedBox(
          width: 44,
          child: Text(
            '${(board.zoom * 100).round()} %',
            textAlign: TextAlign.center,
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              color: context.ws.textMuted,
            ),
          ),
        ),
        _IconButton(
          key: const Key('zoom in'),
          icon: LucideIcons.plus,
          label: 'Zoom in',
          on: false,
          onTap: () => board.zoomTo(board.zoom * 1.25, size()),
        ),
      ],
    ),
  );
}

class _Bar extends StatelessWidget {
  const _Bar({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(4),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(10),
        boxShadow: ws.shadow,
      ),
      child: Row(mainAxisSize: MainAxisSize.min, children: children),
    );
  }
}

class _Separator extends StatelessWidget {
  const _Separator();

  @override
  Widget build(BuildContext context) => Container(
    width: 1,
    height: 20,
    margin: const EdgeInsets.symmetric(horizontal: 4),
    color: context.ws.border,
  );
}

class _Tap extends StatelessWidget {
  const _Tap({required this.label, required this.onTap, required this.child});

  final String label;
  final VoidCallback onTap;
  final Widget child;

  @override
  Widget build(BuildContext context) => Tooltip(
    message: label,
    waitDuration: const Duration(milliseconds: 600),
    child: Semantics(
      button: true,
      label: label,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: child,
        ),
      ),
    ),
  );
}

class _IconButton extends StatelessWidget {
  const _IconButton({
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
    return _Tap(
      label: label,
      onTap: onTap,
      child: Container(
        width: 30,
        height: 30,
        decoration: BoxDecoration(
          color: on ? ws.hover : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Icon(icon, size: 16, color: on ? ws.text : ws.textMuted),
      ),
    );
  }
}

class _Swatch extends StatelessWidget {
  const _Swatch({
    super.key,
    required this.color,
    required this.on,
    required this.onTap,
  });

  final Color color;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => _Tap(
    label: 'Color',
    onTap: onTap,
    child: Container(
      width: 24,
      height: 30,
      alignment: Alignment.center,
      child: Container(
        width: 16,
        height: 16,
        decoration: BoxDecoration(
          color: color,
          shape: BoxShape.circle,
          border: on
              ? Border.all(color: context.ws.accent, width: 2)
              : Border.all(color: const Color(0x1F000000)),
        ),
      ),
    ),
  );
}

class _Width extends StatelessWidget {
  const _Width({
    super.key,
    required this.width,
    required this.dot,
    required this.on,
    required this.onTap,
  });

  final int width;
  final bool dot;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final size = const [5.0, 9.0, 14.0][width];
    return _Tap(
      label: const ['Thin', 'Medium', 'Bold'][width],
      onTap: onTap,
      child: Container(
        width: 30,
        height: 30,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: on ? ws.hover : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Container(
          width: dot ? size : 14,
          height: dot ? size : const [1.5, 3.0, 5.0][width],
          decoration: BoxDecoration(
            color: ws.textMuted,
            borderRadius: BorderRadius.circular(dot ? size : 2),
          ),
        ),
      ),
    );
  }
}

class _Segments extends StatelessWidget {
  const _Segments({
    required this.labels,
    required this.on,
    required this.onTap,
  });

  final List<String> labels;
  final int on;
  final ValueChanged<int> onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(7),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (index, label) in labels.indexed)
            _Tap(
              label: label,
              onTap: () => onTap(index),
              child: Container(
                key: ValueKey('option $label'),
                padding: const EdgeInsets.symmetric(horizontal: 8),
                height: 22,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: index == on ? ws.canvas : null,
                  borderRadius: BorderRadius.circular(5),
                  border: index == on ? Border.all(color: ws.border) : null,
                ),
                child: Text(
                  label,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    color: index == on ? ws.text : ws.textMuted,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _StyleSwitch extends StatelessWidget {
  const _StyleSwitch({required this.board});

  final BoardController board;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget choice(bool hand) {
      final on = board.handDrawn == hand;
      final color = on ? ws.text : ws.textMuted;
      return _Tap(
        label: hand ? 'Hand-drawn' : 'Clean',
        onTap: () {
          if (!on) unawaited(board.toggleStyle());
        },
        child: Container(
          key: ValueKey(hand ? 'style hand-drawn' : 'style clean'),
          height: 26,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            color: on ? ws.canvas : null,
            borderRadius: BorderRadius.circular(6),
            border: on ? Border.all(color: ws.border) : null,
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              CustomPaint(
                size: const Size.square(15),
                painter: _Square(color: color, hand: hand),
              ),
              const SizedBox(width: 3),
              Text(
                'Aa',
                style: TextStyle(
                  fontSize: hand ? 15 : 13.5,
                  height: 16 / (hand ? 15 : 13.5),
                  color: color,
                  fontFamily: hand ? handFont : WsMetrics.fontUi.first,
                  fontFamilyFallback: WsMetrics.fontUi,
                ),
              ),
            ],
          ),
        ),
      );
    }

    return Container(
      key: const Key('whiteboard style'),
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [choice(true), const SizedBox(width: 2), choice(false)],
      ),
    );
  }
}

class _Square extends CustomPainter {
  const _Square({required this.color, required this.hand});

  final Color color;
  final bool hand;

  @override
  void paint(Canvas canvas, Size size) {
    final box = Rect.fromLTWH(2.5, 2.5, size.width - 5, size.height - 5);
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.4
      ..strokeCap = StrokeCap.round;
    canvas.drawPath(
      hand
          ? roughPolygon([
              box.topLeft,
              box.topRight,
              box.bottomRight,
              box.bottomLeft,
            ], const RoughOptions(seed: 5, maxRandomnessOffset: 0.7))
          : (Path()..addRRect(
              RRect.fromRectAndRadius(box, const Radius.circular(1.5)),
            )),
      paint,
    );
  }

  @override
  bool shouldRepaint(_Square old) => old.color != color || old.hand != hand;
}
