import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

const rowHandleWidth = 46.0;

class RowHandle extends StatelessWidget {
  const RowHandle({
    super.key,
    required this.height,
    required this.onInsert,
    required this.menu,
    required this.menuOpen,
    required this.onMenu,
  });

  final double height;

  final ValueChanged<bool> onInsert;

  final LayerLink menu;
  final bool menuOpen;
  final VoidCallback onMenu;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget button(Key key, IconData icon, VoidCallback onTap, bool on) =>
        GestureDetector(
          key: key,
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Container(
            width: 20,
            height: 24,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: on ? ws.hover : null,
              borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
            ),
            child: Icon(
              icon,
              size: 14,
              color: on ? ws.textMuted : ws.textFaint,
            ),
          ),
        );
    return Container(
      key: const Key('row handle'),
      width: rowHandleWidth,
      height: height,
      padding: const EdgeInsets.only(right: WsMetrics.space1),
      alignment: Alignment.centerRight,
      color: Colors.transparent,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: 2,
        children: [
          button(
            const Key('insert row'),
            LucideIcons.plus,
            () => onInsert(HardwareKeyboard.instance.isAltPressed),
            false,
          ),
          CompositedTransformTarget(
            link: menu,
            child: button(
              const Key('row menu button'),
              LucideIcons.gripVertical,
              onMenu,
              menuOpen,
            ),
          ),
        ],
      ),
    );
  }
}

class RowMenu extends StatefulWidget {
  const RowMenu({
    super.key,
    required this.onInsert,
    required this.onDelete,
    required this.onClose,
  });

  final ValueChanged<bool> onInsert;
  final VoidCallback onDelete;
  final VoidCallback onClose;

  @override
  State<RowMenu> createState() => _RowMenuState();
}

class _RowMenuState extends State<RowMenu> {
  late final _focus = FocusNode(debugLabel: 'Row menu', onKeyEvent: _key);

  int? _chosen;

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

  void _take(int item) => switch (item) {
    0 => widget.onInsert(true),
    1 => widget.onInsert(false),
    _ => widget.onDelete(),
  };

  KeyEventResult _key(FocusNode _, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown || LogicalKeyboardKey.arrowUp:
        final by = event.logicalKey == LogicalKeyboardKey.arrowDown ? 1 : -1;
        setState(() => _chosen = ((_chosen ?? (by > 0 ? -1 : 3)) + by) % 3);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is! KeyRepeatEvent && _chosen != null) _take(_chosen!);
      case LogicalKeyboardKey.escape:
        widget.onClose();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget item(int index, IconData icon, String label, {Color? color}) =>
        MouseRegion(
          onEnter: (_) => setState(() => _chosen = index),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => _take(index),
            child: Container(
              height: 30,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              decoration: BoxDecoration(
                color: index == _chosen ? ws.hover : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Row(
                children: [
                  Icon(icon, size: 14, color: color ?? ws.textFaint),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      label,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: color),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
    return Focus(
      focusNode: _focus,
      child: Container(
        key: const Key('row menu'),
        width: 200,
        padding: const EdgeInsets.all(WsMetrics.space2),
        decoration: BoxDecoration(
          color: ws.surface,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          boxShadow: ws.shadow,
        ),
        child: DefaultTextStyle.merge(
          style: TextStyle(fontSize: WsMetrics.fsUi, color: ws.text),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              item(0, LucideIcons.arrowUp, 'Insert above'),
              item(1, LucideIcons.arrowDown, 'Insert below'),
              Divider(height: 9, thickness: 1, color: ws.border),
              item(2, LucideIcons.trash, 'Delete row', color: ws.danger),
            ],
          ),
        ),
      ),
    );
  }
}

class HoverRegion extends StatefulWidget {
  const HoverRegion({super.key, required this.onHover, required this.child});

  final void Function(bool hovered, Object region) onHover;
  final Widget child;

  @override
  State<HoverRegion> createState() => _HoverRegionState();
}

class _HoverRegionState extends State<HoverRegion> {
  var _hovered = false;

  void _hover(bool hovered) {
    _hovered = hovered;
    widget.onHover(hovered, this);
  }

  @override
  void dispose() {
    if (_hovered) scheduleMicrotask(() => widget.onHover(false, this));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
    onEnter: (_) => _hover(true),
    onExit: (_) => _hover(false),
    child: widget.child,
  );
}

class RowHandles extends ChangeNotifier {
  RowHandles({
    required this.indexOf,
    required this.idAt,
    required this.insert,
    required this.remove,
  });

  final int? Function(String row) indexOf;

  final Future<String?> Function(int index) idAt;

  final Future<void> Function(String? after) insert;

  final Future<void> Function(String row) remove;

  final handle = OverlayPortalController();
  final handleAnchor = LayerLink();
  final menu = OverlayPortalController();
  final menuAnchor = LayerLink();

  String? handleRow;
  String? menuRow;

  int? _menuAt;

  String? _pointerRow;
  Object? _rowRegion;
  Object? _handleRegion;

  var _disposed = false;

  void _show() {
    final show =
        handleRow != null &&
        (_pointerRow == handleRow || _handleRegion != null || menuRow != null);
    if (show != handle.isShowing) show ? handle.show() : handle.hide();
  }

  void _changed() {
    _show();
    notifyListeners();
  }

  void hoverRow(String row, bool hovered, Object region) {
    if (_disposed) return;
    if (hovered) {
      _pointerRow = row;
      _rowRegion = region;
    } else if (_rowRegion == region) {
      _pointerRow = null;
      _rowRegion = null;
    }
    if (menuRow == null && _pointerRow != null) handleRow = _pointerRow;
    _changed();
  }

  void hoverHandle(bool hovered, Object region) {
    if (_disposed) return;
    if (hovered) {
      _handleRegion = region;
    } else if (_handleRegion == region) {
      _handleRegion = null;
    }
    _changed();
  }

  void toggleMenu() {
    if (menuRow != null) return closeMenu();
    menuRow = handleRow;
    _menuAt = handleRow == null ? null : indexOf(handleRow!);
    menu.show();
    notifyListeners();
  }

  void closeMenu() {
    if (menuRow == null) return;
    menuRow = null;
    handleRow = _pointerRow ?? handleRow;
    menu.hide();
    _changed();
  }

  void read() {
    if (menuRow case final row? when indexOf(row) != _menuAt) {
      menuRow = null;
      handleRow = _pointerRow ?? handleRow;
      menu.hide();
    }
    if (_pointerRow case final row? when indexOf(row) == null) {
      _pointerRow = null;
      _rowRegion = null;
    }
    if (handleRow case final row? when indexOf(row) == null) {
      handleRow = _pointerRow;
      menuRow = null;
      menu.hide();
    }
    _show();
  }

  Future<void> insertNext(String? row, {required bool above}) async {
    closeMenu();
    final index = row == null ? null : indexOf(row);
    if (index == null) return;
    var after = row;
    if (above) {
      after = index == 0 ? null : await idAt(index - 1);
      if (index > 0 && after == null) return;
    }
    if (!_disposed) await insert(after);
  }

  void deleteRow() {
    final row = menuRow;
    closeMenu();
    if (row != null) unawaited(remove(row));
  }

  Widget handleView(BuildContext context) => Align(
    alignment: Alignment.topLeft,
    child: CompositedTransformFollower(
      link: handleAnchor,
      showWhenUnlinked: false,
      followerAnchor: Alignment.topRight,
      child: HoverRegion(
        onHover: hoverHandle,
        child: TapRegion(
          groupId: menu,
          child: RowHandle(
            height: WsMetrics.rowTable,
            onInsert: (above) => unawaited(insertNext(handleRow, above: above)),
            menu: menuAnchor,
            menuOpen: menuRow != null,
            onMenu: toggleMenu,
          ),
        ),
      ),
    ),
  );

  Widget menuView(BuildContext context) {
    if (menuRow == null) return const SizedBox.shrink();
    return Align(
      alignment: Alignment.topLeft,
      child: CompositedTransformFollower(
        link: menuAnchor,
        showWhenUnlinked: false,
        targetAnchor: Alignment.bottomLeft,
        offset: const Offset(0, WsMetrics.space1),
        child: TapRegion(
          groupId: menu,
          onTapOutside: (_) => closeMenu(),
          child: RowMenu(
            onInsert: (above) => unawaited(insertNext(menuRow, above: above)),
            onDelete: deleteRow,
            onClose: closeMenu,
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
