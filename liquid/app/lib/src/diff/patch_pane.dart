import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_lines.dart';
import 'diff_model.dart';
import 'diff_settings.dart';

/// The shown file's changes, under the turn header with the whitespace and layout switches.
class PatchPane extends StatelessWidget {
  const PatchPane({
    super.key,
    required this.header,
    required this.change,
    required this.lines,
    required this.widest,
    required this.note,
    required this.settings,
    required this.scroll,
    required this.bottom,
    required this.onLayout,
    required this.onWhitespace,
  });

  /// The turn menu and what the shown turns are.
  final Widget? header;
  final BridgeFileChange? change;
  final List<DiffLine> lines;

  /// The shown lines likely widest.
  final List<String> widest;

  /// Why the file shows no lines.
  final String? note;
  final DiffSettings settings;
  final ScrollController scroll;

  /// Room below the last line for the floating input area.
  final double bottom;
  final ValueChanged<DiffLayout> onLayout;
  final VoidCallback onWhitespace;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final change = this.change;
    final note = this.note;
    if (change == null) return const SizedBox.shrink();
    final split = settings.layout == DiffLayout.split;
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space7,
        WsMetrics.space5,
        WsMetrics.space7,
        0,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: header ?? const SizedBox.shrink()),
              _Toggle(
                label: 'Hide whitespace',
                on: settings.hideWhitespace,
                onTap: onWhitespace,
              ),
              const SizedBox(width: WsMetrics.space3),
              Segments(
                options: const [
                  (DiffLayout.unified, 'Unified'),
                  (DiffLayout.split, 'Split'),
                ],
                selected: settings.layout,
                onSelect: onLayout,
              ),
            ],
          ),
          const SizedBox(height: WsMetrics.space4),
          Container(
            height: 36,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: const BorderRadius.vertical(
                top: Radius.circular(WsMetrics.radiusLg),
              ),
            ),
            alignment: Alignment.centerLeft,
            child: Text(change.shown, style: monoStyle(context)),
          ),
          Expanded(
            child: DecoratedBox(
              decoration: BoxDecoration(
                border: Border.all(color: ws.border),
                borderRadius: const BorderRadius.vertical(
                  bottom: Radius.circular(WsMetrics.radiusLg),
                ),
              ),
              // Long lines scroll sideways together, so no change is out of reach.
              child: note != null
                  ? Center(
                      child: Text(
                        note,
                        style: Theme.of(context).textTheme.bodySmall,
                      ),
                    )
                  : LayoutBuilder(
                      builder: (context, constraints) {
                        final style = monoStyle(context);
                        // Split shows each side as wide as the unified view's code.
                        final code =
                            widest.fold(
                              0.0,
                              (wide, text) => math.max(
                                wide,
                                measureText(text, style).width,
                              ),
                            ) +
                            WsMetrics.space6;
                        final rows = split
                            ? splitRows(lines)
                            : const <SplitRow>[];
                        return SingleChildScrollView(
                          scrollDirection: Axis.horizontal,
                          child: SizedBox(
                            width: math.max(
                              constraints.maxWidth,
                              split
                                  ? 2 * (UnifiedLineRow.side + code)
                                  : UnifiedLineRow.gutter + code,
                            ),
                            height: constraints.maxHeight,
                            child: ListView.builder(
                              controller: scroll,
                              padding: EdgeInsets.only(bottom: bottom),
                              itemExtent: UnifiedLineRow.height,
                              itemCount: split ? rows.length : lines.length,
                              itemBuilder: (context, index) => split
                                  ? SplitLineRow(row: rows[index])
                                  : UnifiedLineRow(line: lines[index]),
                            ),
                          ),
                        );
                      },
                    ),
            ),
          ),
          const SizedBox(height: WsMetrics.space4),
        ],
      ),
    );
  }
}

/// A switch shown as a button that stays pressed while on.
class _Toggle extends StatelessWidget {
  const _Toggle({required this.label, required this.on, required this.onTap});

  final String label;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    // Its state is told to assistive technology, not only drawn.
    return Semantics(
      toggled: on,
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          child: Ink(
            height: 28,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            decoration: BoxDecoration(
              color: on ? ws.accentSubtle : null,
              border: on ? null : Border.all(color: ws.borderStrong),
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Center(
              widthFactor: 1,
              child: Text(
                label,
                style: TextStyle(
                  fontSize: WsMetrics.fsCompact,
                  color: on ? ws.accent : ws.textMuted,
                  fontWeight: on ? WsMetrics.fwMedium : WsMetrics.fwRegular,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
