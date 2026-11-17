import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/workspace.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../whiteboard/board_controller.dart';
import '../whiteboard/board_surface.dart';
import '../whiteboard/board_tools.dart';
import 'anchored_menu.dart';
import 'page_views.dart';
import 'workspace_state.dart';

/// A Whiteboard's frame on the Page.
const whiteboardHeight = 360.0;

/// A Whiteboard Block on its Page (approved Whiteboard Block mockups, ADR-0306): a frame of
/// fixed height whose canvas has no edges. A click focuses it, and the tools draw on it in
/// place; hovering shows full screen and ⋯ with Delete. It follows only its own Block
/// (client architecture rule 6).
class WhiteboardBlock extends StatefulWidget {
  const WhiteboardBlock({super.key, required this.views, required this.view});

  final PageViews views;
  final BridgeView view;

  @override
  State<WhiteboardBlock> createState() => _WhiteboardBlockState();
}

class _WhiteboardBlockState extends State<WhiteboardBlock> {
  late final BoardController _board = widget.views.boardFor(widget.view);
  final _focus = FocusNode(debugLabel: 'Whiteboard');
  final _surface = GlobalKey<BoardSurfaceState>();
  final _more = OverlayPortalController();
  var _hovered = false;

  @override
  void initState() {
    super.initState();
    unawaited(_board.read());
    _focus.addListener(_changed);
    widget.views.focusBoard.addListener(_takeFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) => _takeFocus());
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  /// A new Whiteboard starts focused, with the pen.
  void _takeFocus() {
    if (!mounted || widget.views.focusBoard.value != widget.view.id) return;
    widget.views.focusBoard.value = null;
    boardTools.tool = BoardTool.pen;
    _focus.requestFocus();
  }

  @override
  void dispose() {
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
    return TapRegion(
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
            key: const Key('whiteboard'),
            height: whiteboardHeight,
            margin: const EdgeInsets.symmetric(vertical: WsMetrics.space1),
            decoration: BoxDecoration(
              color: ws.canvas,
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: focused ? ws.accent : ws.border,
                width: focused ? 1.5 : 1,
              ),
            ),
            clipBehavior: Clip.antiAlias,
            child: Stack(
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
                if (focused) ...[
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
                if (_hovered || _more.isShowing)
                  Positioned(top: 8, right: 8, child: _bar(context)),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _bar(BuildContext context) {
    final ws = context.ws;
    Widget button(IconData icon, Key key, String label, VoidCallback onTap) =>
        Tooltip(
          message: label,
          waitDuration: const Duration(milliseconds: 600),
          child: MouseRegion(
            cursor: SystemMouseCursors.click,
            child: GestureDetector(
              key: key,
              onTap: onTap,
              child: Padding(
                padding: const EdgeInsets.all(5),
                child: Icon(icon, size: 14, color: ws.textMuted),
              ),
            ),
          ),
        );
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
          button(
            LucideIcons.maximize2,
            const Key('whiteboard full screen'),
            'Full screen',
            () => widget.views.fullBoard.value = widget.view,
          ),
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
                key: const ValueKey('whiteboard Delete'),
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
            child: button(
              LucideIcons.ellipsis,
              const Key('whiteboard more'),
              'More',
              () => setState(_more.toggle),
            ),
          ),
        ],
      ),
    );
  }
}

/// A Whiteboard full screen over the Page's area, at the same place and zoom: Fit shows
/// everything drawn, esc or Done returns to the Page (approved mockups, section 6).
class WhiteboardFullScreen extends StatefulWidget {
  const WhiteboardFullScreen({
    super.key,
    required this.views,
    required this.view,
  });

  final PageViews views;
  final BridgeView view;

  @override
  State<WhiteboardFullScreen> createState() => _WhiteboardFullScreenState();
}

class _WhiteboardFullScreenState extends State<WhiteboardFullScreen> {
  late final BoardController _board = widget.views.boardFor(widget.view);
  final _focus = FocusNode(debugLabel: 'Whiteboard full screen');
  final _surface = GlobalKey<BoardSurfaceState>();
  final _canvas = GlobalKey();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    widget.views.releaseBoard(widget.view);
    _focus.dispose();
    super.dispose();
  }

  void _done() => widget.views.fullBoard.value = null;

  Size _size() => _canvas.currentContext?.size ?? Size.zero;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Focus(
      focusNode: _focus,
      // esc ends a text box, then clears the selection, then leaves full screen.
      onKeyEvent: (_, event) =>
          _surface.currentState?.key(event) ?? KeyEventResult.ignored,
      child: ColoredBox(
        key: const Key('whiteboard full'),
        color: ws.canvas,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 44,
              padding: const EdgeInsets.only(left: 20, right: 12),
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: ws.border)),
              ),
              child: Row(
                children: [
                  Icon(LucideIcons.presentation, size: 14, color: ws.textMuted),
                  const SizedBox(width: WsMetrics.space2),
                  Text(
                    'Whiteboard',
                    style: TextStyle(
                      fontSize: WsMetrics.fsCompact,
                      color: ws.text,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                  const Spacer(),
                  TextButton(
                    key: const Key('whiteboard fit'),
                    onPressed: () => _board.fit(_size()),
                    child: const Text('Fit'),
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
