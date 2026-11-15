import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/tokens.g.dart';

/// A menu that hangs from the widget it wraps, its anchor, lined up with its left edge:
/// under it, or above it when the menu's [height] does not fit under it and there is more
/// room above. It takes at most the room on its side, [WsMetrics.space1] short of the
/// window's edge, and chooses again whenever the anchor moves, so it stays in the window
/// while the Page scrolls under it or the window changes size. A click in it is no click
/// outside a text field.
class AnchoredMenu extends StatefulWidget {
  const AnchoredMenu({
    super.key,
    this.controller,
    required this.menu,
    required this.child,
    this.height,
    this.gap = WsMetrics.space1,
    this.onTapOutside,
  });

  /// Shows and hides the menu; without one, the menu shows while the anchor is built.
  /// A controller stays with one anchor: handed to the next one, it loses the menu.
  final OverlayPortalController? controller;

  /// Builds the menu.
  final WidgetBuilder menu;

  /// The anchor.
  final Widget child;

  /// The menu's whole height; without one it always hangs under the anchor.
  final double? height;

  /// The space between the anchor and the menu.
  final double gap;

  /// Called on a click outside both the menu and the anchor.
  final TapRegionCallback? onTapOutside;

  @override
  State<AnchoredMenu> createState() => _AnchoredMenuState();
}

class _AnchoredMenuState extends State<AnchoredMenu> {
  late final _menu = widget.controller ?? (OverlayPortalController()..show());

  @override
  Widget build(BuildContext context) {
    final AnchoredMenu(:height, :gap, :menu, :child) = widget;
    final tapped = widget.onTapOutside;
    return OverlayPortal.overlayChildLayoutBuilder(
      controller: _menu,
      overlayChildBuilder: (context, info) {
        final anchor = MatrixUtils.transformRect(
          info.childPaintTransform,
          Offset.zero & info.childSize,
        );
        final window = info.overlaySize;
        final below = window.height - anchor.bottom - gap - WsMetrics.space1;
        final above = anchor.top - gap - WsMetrics.space1;
        final wanted = height;
        final up = wanted != null && wanted > below && above > below;
        Widget shown = TextFieldTapRegion(child: menu(context));
        if (tapped != null) {
          shown = TapRegion(groupId: _menu, onTapOutside: tapped, child: shown);
        }
        // The overlay lays its child out as a Stack does.
        return Positioned(
          left: anchor.left,
          top: up ? null : anchor.bottom + gap,
          bottom: up ? window.height - anchor.top + gap : null,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: wanted == null
                  ? double.infinity
                  : math.max(0, up ? above : below),
            ),
            child: shown,
          ),
        );
      },
      child: tapped == null ? child : TapRegion(groupId: _menu, child: child),
    );
  }
}
