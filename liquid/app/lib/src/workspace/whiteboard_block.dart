import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../whiteboard/board_controller.dart';
import '../whiteboard/board_surface.dart';
import '../whiteboard/board_tools.dart';
import '../whiteboard/document_pages.dart';
import 'anchored_menu.dart';
import 'page_views.dart';
import 'workspace_state.dart';

const whiteboardHeight = 360.0;

const documentHeight = 560.0;

class WhiteboardBlock extends StatelessWidget {
  const WhiteboardBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  Widget build(BuildContext context) => DrawingFrame(
    views: views,
    view: view,
    name: 'whiteboard',
    height: whiteboardHeight,
  );
}

class DrawingFrame extends StatefulWidget {
  const DrawingFrame({
    super.key,
    required this.views,
    required this.view,
    required this.name,
    required this.height,
    this.buttons = const [],
    this.framed = true,
    this.bar = true,
  });

  final PageViews views;
  final BridgeView view;

  final String name;
  final double height;

  final List<Widget> buttons;

  final bool framed;

  final bool bar;

  @override
  State<DrawingFrame> createState() => _DrawingFrameState();
}

class _DrawingFrameState extends State<DrawingFrame> {
  late final BoardController _board = widget.views.boardFor(widget.view);
  final _focus = FocusNode(debugLabel: 'Drawing');
  final _surface = GlobalKey<BoardSurfaceState>();
  final _more = OverlayPortalController();
  final _frame = GlobalKey(debugLabel: 'Drawing frame');
  var _hovered = false;

  Size _size() => switch (_frame.currentContext?.findRenderObject()) {
    final RenderBox box when box.hasSize => box.size,
    _ => Size.zero,
  };

  @override
  void initState() {
    super.initState();
    unawaited(_board.read());
    _focus.addListener(_changed);
    _board.addListener(_followPages);
    widget.views.focusBoard.addListener(_takeFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) => _takeFocus());
  }

  final _below = OverlayPortalController();

  void _changed() {
    if (!mounted) return;
    if (!widget.framed) {
      _focus.hasFocus ? _below.show() : _below.hide();
    }
    setState(() {});
  }

  void _takeFocus() {
    if (!mounted || widget.views.focusBoard.value != widget.view.id) return;
    widget.views.focusBoard.value = null;
    boardTools.tool = BoardTool.pen;
    _focus.requestFocus();
  }

  void _followPages() {
    if (!mounted || !_board.paged) return;
    followPages(_board, _size());
  }

  @override
  void dispose() {
    _board.removeListener(_followPages);
    widget.views.focusBoard.removeListener(_takeFocus);
    widget.views.releaseBoard(widget.view);
    _focus.dispose();
    super.dispose();
  }

  void _delete() {
    _more.hide();
    unawaited(
      widget.views
          .write(
            mutation(
              BridgeMutationKind.removeView,
              page: widget.views.page,
              view: widget.view.id,
            ),
            [widget.view.block],
          )
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final focused = _focus.hasFocus;
    final name = widget.name;
    final frame = TapRegion(
      groupId: _focus,
      onTapOutside: (_) {
        if (_focus.hasFocus) _focus.unfocus();
      },
      child: Focus(
        focusNode: _focus,
        onKeyEvent: (_, event) =>
            _surface.currentState?.key(event) ?? KeyEventResult.ignored,
        child: MouseRegion(
          onEnter: (_) => setState(() => _hovered = true),
          onExit: (_) => setState(() => _hovered = false),
          child: Container(
            key: _frame,
            height: widget.height,
            margin: widget.framed
                ? const EdgeInsets.symmetric(vertical: WsMetrics.space1)
                : null,
            decoration: widget.framed
                ? BoxDecoration(
                    color: _board.paged ? ws.subtle : ws.canvas,
                    borderRadius: BorderRadius.circular(10),
                    border: Border.all(
                      color: focused ? ws.accent : ws.border,
                      width: focused ? 1.5 : 1,
                    ),
                  )
                : BoxDecoration(
                    border: focused
                        ? Border.all(color: ws.accent, width: 1.5)
                        : null,
                  ),
            clipBehavior: widget.framed ? Clip.antiAlias : Clip.none,
            child: Stack(
              key: Key(name),
              clipBehavior: Clip.none,
              children: [
                Positioned.fill(
                  child: BoardSurface(
                    key: _surface,
                    board: _board,
                    focused: focused,
                    onFocus: _focus.requestFocus,
                    onLeave: _focus.unfocus,
                  ),
                ),
                if (_board.paged && _board.sheets.length > 1)
                  Positioned(
                    left: 12,
                    bottom: 12,
                    child: PageCounter(board: _board, size: _size),
                  ),
                if (focused && widget.framed) ...[
                  Positioned(
                    left: 0,
                    right: 0,
                    bottom: 12,
                    child: Center(child: BoardToolBars(board: _board)),
                  ),
                  Positioned(
                    right: 10,
                    bottom: 12,
                    child: ZoomBar(
                      board: _board,
                      size: () => context.size ?? Size.zero,
                    ),
                  ),
                ],
                if (widget.bar && (_hovered || _more.isShowing))
                  Positioned(
                    key: ValueKey('$name bar'),
                    top: 8,
                    right: 8,
                    child: _bar(context),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    if (widget.framed) return frame;
    return OverlayPortal.overlayChildLayoutBuilder(
      controller: _below,
      overlayChildBuilder: (context, info) {
        final anchor = MatrixUtils.transformRect(
          info.childPaintTransform,
          Offset.zero & info.childSize,
        );
        return Positioned(
          left: anchor.left,
          top: anchor.bottom + 8,
          child: TapRegion(
            groupId: _focus,
            child: BoardToolBars(board: _board),
          ),
        );
      },
      child: frame,
    );
  }

  Widget _bar(BuildContext context) {
    final ws = context.ws;
    final name = widget.name;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.surface,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        boxShadow: ws.shadow,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          BarButton(
            icon: LucideIcons.maximize2,
            key: Key('$name full screen'),
            label: 'Full screen',
            onTap: () => widget.views.fullBoard.value = widget.view,
          ),
          ...widget.buttons,
          AnchoredMenu(
            controller: _more,
            onTapOutside: (_) => setState(_more.hide),
            menu: (context) => Container(
              width: 150,
              padding: const EdgeInsets.all(WsMetrics.space1 + 2),
              decoration: BoxDecoration(
                color: ws.surface,
                borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
                boxShadow: ws.shadow,
              ),
              child: GestureDetector(
                key: ValueKey('$name Delete'),
                onTap: _delete,
                child: Container(
                  height: 30,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space2,
                  ),
                  child: Row(
                    children: [
                      Icon(LucideIcons.trash, size: 14, color: ws.danger),
                      const SizedBox(width: WsMetrics.space2),
                      Text(
                        'Delete',
                        style: TextStyle(
                          fontSize: WsMetrics.fsUi,
                          color: ws.danger,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            child: BarButton(
              icon: LucideIcons.ellipsis,
              key: Key('$name more'),
              label: 'More',
              onTap: () => setState(_more.toggle),
            ),
          ),
        ],
      ),
    );
  }
}

class BarButton extends StatelessWidget {
  const BarButton({
    super.key,
    required this.icon,
    required this.label,
    required this.onTap,
    this.showLabel = false,
  });

  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final bool showLabel;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Tooltip(
      message: label,
      waitDuration: const Duration(milliseconds: 600),
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.all(5),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(icon, size: 14, color: ws.textMuted),
                if (showLabel) ...[
                  const SizedBox(width: WsMetrics.space1),
                  Text(
                    label,
                    style: TextStyle(
                      fontSize: WsMetrics.fsCompact,
                      color: ws.textMuted,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

void followPages(BoardController board, Size size) {
  final view = Rect.fromLTWH(
    board.origin.dx,
    board.origin.dy,
    size.width / board.zoom,
    size.height / board.zoom,
  );
  final shown = [
    for (final (index, sheet) in board.sheets.indexed)
      if (sheet.overlaps(view)) index,
  ];
  if (shown.isEmpty) return;
  board.showPages(shown.first, shown.last + 3 - shown.first);
}

class PageCounter extends StatelessWidget {
  const PageCounter({super.key, required this.board, required this.size});

  final BoardController board;

  final Size Function() size;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ListenableBuilder(
      listenable: board,
      builder: (context, _) {
        final middle = board.toBoard(size().center(Offset.zero));
        final page = board.pages?.pageNear(middle) ?? 0;
        return Container(
          key: const Key('page counter'),
          padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3),
          decoration: BoxDecoration(
            color: ws.surface,
            borderRadius: BorderRadius.circular(7),
            boxShadow: ws.shadow,
          ),
          child: Text(
            '${page + 1} / ${board.sheets.length}',
            style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted),
          ),
        );
      },
    );
  }
}

class DrawingFullScreen extends StatefulWidget {
  const DrawingFullScreen({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<DrawingFullScreen> createState() => _DrawingFullScreenState();
}

class _DrawingFullScreenState extends State<DrawingFullScreen> {
  late final BoardController _board = widget.views.boardFor(widget.view);
  final _focus = FocusNode(debugLabel: 'Drawing full screen');
  final _surface = GlobalKey<BoardSurfaceState>();
  final _canvas = GlobalKey();

  @override
  void initState() {
    super.initState();
    _board.addListener(_followPages);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  void _followPages() {
    if (_board.paged) followPages(_board, _size());
  }

  @override
  void dispose() {
    _board.removeListener(_followPages);
    widget.views.releaseBoard(widget.view);
    _focus.dispose();
    super.dispose();
  }

  void _done() => widget.views.fullBoard.value = null;

  Size _size() => switch (_canvas.currentContext?.findRenderObject()) {
    final RenderBox box when box.hasSize => box.size,
    _ => Size.zero,
  };

  int _page() =>
      _board.pages?.pageNear(_board.toBoard(_size().center(Offset.zero))) ?? 0;

  void _show(int index) {
    final sheets = _board.sheets;
    if (sheets.isEmpty) return;
    final sheet = sheets[index.clamp(0, sheets.length - 1)];
    final size = _size();
    _board
      ..zoom = (size.width / (sheet.width + 2 * DocumentPages.gap)).clamp(
        BoardController.minZoom,
        BoardController.maxZoom,
      )
      ..origin = Offset(
        sheet.center.dx - size.width / 2 / _board.zoom,
        sheet.top - DocumentPages.gap,
      )
      ..notifyListeners();
  }

  KeyEventResult _key(KeyEvent event) {
    if (_board.paged && event is! KeyUpEvent) {
      switch (event.logicalKey) {
        case LogicalKeyboardKey.arrowDown:
          _show(_page() + 1);
          return KeyEventResult.handled;
        case LogicalKeyboardKey.arrowUp:
          _show(_page() - 1);
          return KeyEventResult.handled;
      }
    }
    return _surface.currentState?.key(event) ?? KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final paged = _board.paged;
    return Focus(
      focusNode: _focus,
      onKeyEvent: (_, event) => _key(event),
      child: ColoredBox(
        key: const Key('whiteboard full'),
        color: paged ? ws.subtle : ws.canvas,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 44,
              padding: const EdgeInsets.only(left: 20, right: 12),
              decoration: BoxDecoration(
                color: ws.canvas,
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: Row(
                children: [
                  Icon(
                    paged ? LucideIcons.file : LucideIcons.presentation,
                    size: 14,
                    color: ws.textMuted,
                  ),
                  const SizedBox(width: WsMetrics.space2),
                  Flexible(
                    child: Text(
                      _board.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        color: ws.text,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                  ),
                  const Spacer(),
                  if (paged && _board.sheets.length > 1) ...[
                    PageCounter(board: _board, size: _size),
                    const SizedBox(width: WsMetrics.space2),
                  ],
                  TextButton(
                    key: const Key('whiteboard fit'),
                    onPressed: () =>
                        paged ? _show(_page()) : _board.fit(_size()),
                    child: Text(paged ? 'Fit width' : 'Fit'),
                  ),
                  const SizedBox(width: WsMetrics.space1),
                  TextButton.icon(
                    key: const Key('whiteboard done'),
                    onPressed: _done,
                    icon: const Icon(LucideIcons.minimize2, size: 14),
                    label: const Text('Done'),
                  ),
                ],
              ),
            ),
            Expanded(
              child: Stack(
                children: [
                  Positioned.fill(
                    key: _canvas,
                    child: BoardSurface(
                      key: _surface,
                      board: _board,
                      focused: true,
                      onFocus: _focus.requestFocus,
                      onLeave: _done,
                    ),
                  ),
                  Positioned(
                    left: 0,
                    right: 0,
                    bottom: 12,
                    child: Center(child: BoardToolBars(board: _board)),
                  ),
                  Positioned(
                    right: 10,
                    bottom: 12,
                    child: ZoomBar(board: _board, size: _size),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
