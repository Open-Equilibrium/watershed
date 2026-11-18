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

class DiffPanel extends StatelessWidget {
  const DiffPanel({
    super.key,
    required this.state,
    required this.settings,
    required this.parse,
    required this.onClose,
  });

  static const double defaultWidth = 540;

  static const double minWidth = 360;

  static const double chatMinWidth = 360;

  static double maxWidthOf(double viewport, double container) =>
      math.max(minWidth, math.min(viewport * 0.7, container - chatMinWidth));

  final SessionState state;
  final DiffSettingsController settings;
  final PatchParser parse;

  final VoidCallback onClose;

  void _dragBy(BuildContext context, double dx) {
    final box = context.findRenderObject()! as RenderBox;
    settings.value = settings.value.copyWith(width: box.size.width - dx);
  }

  void _dragEnded(BuildContext context) =>
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || settings.value.width == null) return;
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
                        IconButton(
                          key: const Key('diff-panel-close'),
                          tooltip: 'Close the Diff',
                          onPressed: onClose,
                          padding: EdgeInsets.zero,
                          constraints: const BoxConstraints.tightFor(
                            width: 24,
                            height: 24,
                          ),
                          icon: Icon(
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
            ),
          ),
        ),
      ],
    );
  }
}
