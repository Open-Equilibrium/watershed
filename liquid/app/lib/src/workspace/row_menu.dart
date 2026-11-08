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
