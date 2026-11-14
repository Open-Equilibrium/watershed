import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';
import 'diff_settings.dart';
import 'diff_view.dart';

/// The Diff as T3 Code's right panel beside the chat: its tab with a close button, then the
/// changes. The chat area lays the panel out at its width; its left edge drags that width, which
/// the Diff settings keep as T3 Code keeps its right panel's.
class DiffPanel extends StatelessWidget {
  const DiffPanel({
    super.key,
    required this.state,
    required this.settings,
    required this.parse,
    required this.onClose,
  });

  /// The width before the panel is dragged, as T3 Code's right panel starts.
  static const double defaultWidth = 540;

  /// The narrowest the panel gets, as T3 Code's right panel goes.
  static const double minWidth = 360;

  /// The room the chat keeps beside the panel, as T3 Code's chat keeps.
  static const double chatMinWidth = 360;

  /// The widest the panel gets in a window `viewport` wide beside a row `container` wide: 70 %
  /// of the window, and the chat keeps [chatMinWidth].
  static double maxWidthOf(double viewport, double container) =>
      math.max(minWidth, math.min(viewport * 0.7, container - chatMinWidth));

  final SessionState state;
  final DiffSettingsController settings;
  final PatchParser parse;

  /// Closes the panel; the close button and ⌘D both do.
  final VoidCallback onClose;

  /// Dragging the left edge left widens the panel. The drag only sets the width while it runs, so
  /// the chat area lays the panel out again; the release saves the width it ended at.
  void _dragBy(BuildContext context, double dx) {
    final box = context.findRenderObject()! as RenderBox;
    settings.value = settings.value.copyWith(width: box.size.width - dx);
  }

  /// Saves the width the panel was laid out at, once the drag's last layout has run.
  void _dragEnded(BuildContext context) =>
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        final box = context.findRenderObject()! as RenderBox;
        unawaited(
          settings.change(settings.value.copyWith(width: box.size.width)),
        );
      });

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Stack(
      fit: StackFit.expand,
      children: [
        DecoratedBox(
          decoration: BoxDecoration(
            border: Border(left: BorderSide(color: ws.border)),
          ),
          child: Column(
            children: [
              Container(
                height: 36,
                alignment: Alignment.centerLeft,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                child: Semantics(
                  selected: true,
                  child: Container(
                    key: const Key('diff-panel'),
                    height: 26,
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space3,
                    ),
                    decoration: BoxDecoration(
                      color: ws.hover,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      spacing: WsMetrics.space2,
                      children: [
                        Icon(
                          LucideIcons.fileDiff,
                          size: 16,
                          color: ws.textFaint,
                        ),
                        Text(
                          'Diff',
                          style: TextStyle(
                            color: ws.text,
                            fontSize: WsMetrics.fsCompact,
                            fontWeight: WsMetrics.fwMedium,
                          ),
                        ),
                        InkWell(
                          key: const Key('diff-panel-close'),
                          onTap: onClose,
                          child: Icon(
                            LucideIcons.x,
                            size: 11,
                            color: ws.textFaint,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              Expanded(
                child: DiffView(state: state, settings: settings, parse: parse),
              ),
            ],
          ),
        ),
        Positioned(
          left: 0,
          top: 0,
          bottom: 0,
          width: 8,
          child: MouseRegion(
            cursor: SystemMouseCursors.resizeColumn,
            child: GestureDetector(
              key: const Key('diff-resize'),
              behavior: HitTestBehavior.opaque,
              onHorizontalDragUpdate: (details) =>
                  _dragBy(context, details.delta.dx),
              onHorizontalDragEnd: (_) => _dragEnded(context),
              onHorizontalDragCancel: () => _dragEnded(context),
            ),
          ),
        ),
      ],
    );
  }
}
