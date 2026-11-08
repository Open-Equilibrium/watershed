import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The width of a row's handle, left of the Table, with the gap that keeps the pointer on
/// it from the row.
const rowHandleWidth = 46.0;

/// A hovered row's handle (approved Table View mockups, section 5): + inserts a row below,
/// ⌥ and + one above, as in Notion, and ⋮⋮ opens the row's menu.
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

  /// Inserts a row above or, without ⌥, below.
  final ValueChanged<bool> onInsert;

  /// Where the row's menu hangs from.
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
      // The gap to the row belongs to the handle, so the pointer crossing it keeps it.
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

/// A row's menu (approved Table View mockups, section 5): insert a row above or below, or
/// delete it without asking. ↑↓ choose, ⏎ takes, esc closes.
class RowMenu extends StatefulWidget {
  const RowMenu({
    super.key,
    required this.onInsert,
    required this.onDelete,
    required this.onClose,
  });

  /// Inserts a row above or, when false, below.
  final ValueChanged<bool> onInsert;
  final VoidCallback onDelete;
  final VoidCallback onClose;

  @override
  State<RowMenu> createState() => _RowMenuState();
}

class _RowMenuState extends State<RowMenu> {
  late final _focus = FocusNode(debugLabel: 'Row menu', onKeyEvent: _key);

  /// The item ↑↓ or the pointer chose: Insert above, Insert below or Delete row.
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

/// A region that tells when the pointer enters it and leaves it, also when it goes from
/// under the pointer, which Flutter's MouseRegion does not tell, so no hover outlives what
/// was hovered. Each telling names the region, so a late leave of a region gone is told
/// apart from a newer enter of the same thing's new region.
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
    // Told after the frame that removed it, not while it is built.
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

/// A Table's row handle and row menu and what they write (approved Table View mockups,
/// section 5): the handle follows the row under the pointer unless the menu holds it, and
/// the menu closes when its row moves elsewhere.
class RowHandles extends ChangeNotifier {
  RowHandles({
    required this.indexOf,
    required this.idAt,
    required this.insert,
    required this.remove,
  });

  /// Where a row is among those read, or none.
  final int? Function(String row) indexOf;

  /// The row at a position, read if it is not.
  final Future<String?> Function(int index) idAt;

  /// Inserts a row after `after`, or first; a refused write shows on the Page.
  final Future<void> Function(String? after) insert;

  /// Deletes a row; a refused write shows on the Page.
  final Future<void> Function(String row) remove;

  final handle = OverlayPortalController();
  final handleAnchor = LayerLink();
  final menu = OverlayPortalController();
  final menuAnchor = LayerLink();

  /// The row the handle is on, and the row whose menu is open.
  String? handleRow;
  String? menuRow;

  /// Where the menu's row was when the menu opened; a change elsewhere moving it closes
  /// the menu, as other menus close when what they hang from moves.
  int? _menuAt;

  /// The row the pointer is on, which the handle moves to once no menu holds it, and the
  /// regions that told the pointer is on the row and on the handle.
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
    // While a row's menu is open, its handle stays on that row.
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

  /// Rows read again: a row moved elsewhere closes its menu, and a row gone takes its
  /// handle along.
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

  /// Inserts a row above or below `row`, which the core places next to it (ADR-0247).
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

  /// Delete row deletes the menu's row without asking (section 5).
  void deleteRow() {
    final row = menuRow;
    closeMenu();
    if (row != null) unawaited(remove(row));
  }

  /// The handle, left of its row's first field.
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

  /// The open menu, under ⋮⋮.
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
