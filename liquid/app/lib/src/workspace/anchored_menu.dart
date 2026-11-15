import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/tokens.g.dart';

/// A menu that hangs from the widget it wraps, its anchor: under it, or above it when the
/// menu's [height] does not fit under it and there is more room above. It takes at most
/// the room on its side, [WsMetrics.space1] short of the window's edge, and chooses again
/// whenever the anchor moves, so it stays in the window while the Page scrolls under it or
/// the window changes size. A click in it is no click outside a text field.
class AnchoredMenu extends StatelessWidget {
  const AnchoredMenu({
    super.key,
    required this.controller,
    required this.menu,
    required this.child,
    this.height,
    this.gap = WsMetrics.space1,
    this.end = false,
    this.onTapOutside,
  });

  /// Shows and hides the menu.
  final OverlayPortalController controller;

  /// Builds the menu.
  final WidgetBuilder menu;

  /// The anchor.
  final Widget child;

  /// The menu's whole height; without one it always hangs under the anchor.
  final double? height;

  /// The space between the anchor and the menu.
  final double gap;

  /// Whether the menu lines up with the anchor's right edge, not its left.
  final bool end;

  /// Called on a click outside both the menu and the anchor.
  final TapRegionCallback? onTapOutside;

  @override
  Widget build(BuildContext context) {
    final tapped = onTapOutside;
    return OverlayPortal.overlayChildLayoutBuilder(
      controller: controller,
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
          shown = TapRegion(
            groupId: controller,
            onTapOutside: tapped,
            child: shown,
          );
        }
        return Stack(
          children: [
            Positioned(
              left: end ? null : anchor.left,
              right: end ? window.width - anchor.right : null,
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
            ),
          ],
        );
      },
      child: tapped == null
          ? child
          : TapRegion(groupId: controller, child: child),
    );
  }
}
