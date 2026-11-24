import 'package:flutter/material.dart';

import '../controls.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'explorer.dart' show untitled;
import 'page_views.dart';

class ArrangeBar extends StatelessWidget {
  const ArrangeBar({super.key, required this.views, required this.title});

  final PageViews views;
  final String title;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    Widget segment(String label, bool narrow) => ValueListenableBuilder(
      valueListenable: views.narrowPreview,
      builder: (context, shown, _) {
        final on = shown == narrow;
        return Semantics(
          button: true,
          selected: on,
          child: GestureDetector(
            key: Key('preview $label'),
            behavior: HitTestBehavior.opaque,
            onTap: () => views.narrowPreview.value = narrow,
            child: Container(
              height: 26,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: on ? ws.surface : null,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              ),
              child: Text(
                label,
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: on ? ws.text : ws.textMuted,
                ),
              ),
            ),
          ),
        );
      },
    );
    return Container(
      key: const Key('arrange bar'),
      height: WsMetrics.rowHeader,
      color: ws.accentSubtle,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
      child: Row(
        children: [
          Icon(LucideIcons.layoutGrid, size: 16, color: ws.accent),
          const SizedBox(width: WsMetrics.space3),
          Flexible(
            child: Text(
              'Arranging ${title.isEmpty ? untitled : title}',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: ws.accent,
                fontWeight: WsMetrics.fwSemibold,
              ),
            ),
          ),
          const SizedBox(width: WsMetrics.space3),
          const Flexible(flex: 2, child: _Hint()),
          const SizedBox(width: WsMetrics.space3),
          Container(
            padding: const EdgeInsets.all(2),
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [segment('Desktop', false), segment('Narrow', true)],
            ),
          ),
          const SizedBox(width: WsMetrics.space3),
          PrimaryButton(
            key: const Key('arrange done'),
            label: 'Done',
            shortcut: 'esc',
            onPressed: () => views.arranging.value = false,
          ),
        ],
      ),
    );
  }
}

class _Hint extends StatelessWidget {
  const _Hint();

  static const _text = 'Drag the top handle to move, the right edge to resize';

  @override
  Widget build(BuildContext context) {
    final style = DefaultTextStyle.of(context).style.merge(
      TextStyle(fontSize: WsMetrics.fsCompact, color: context.ws.accent),
    );
    return LayoutBuilder(
      builder: (context, room) {
        final whole = TextPainter(
          text: TextSpan(text: _text, style: style),
          textDirection: Directionality.of(context),
          textScaler: MediaQuery.textScalerOf(context),
        )..layout();
        final fits = whole.width <= room.maxWidth;
        whole.dispose();
        return fits
            ? Text(_text, maxLines: 1, style: style)
            : const SizedBox.shrink();
      },
    );
  }
}
