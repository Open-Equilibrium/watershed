import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/tokens.g.dart';

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

  final OverlayPortalController? controller;

  final WidgetBuilder menu;

  final Widget child;

  final double? height;

  final double gap;

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
