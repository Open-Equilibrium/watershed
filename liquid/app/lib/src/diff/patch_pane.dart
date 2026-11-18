import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_lines.dart';
import 'diff_model.dart';
import 'diff_settings.dart';

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
    required this.onLayout,
    required this.onWhitespace,
  });

  final Widget? header;
  final BridgeFileChange? change;
  final List<DiffLine> lines;

  final List<String> widest;

  final String? note;
  final DiffSettings settings;
  final ScrollController scroll;

  final ValueChanged<DiffLayout> onLayout;
  final VoidCallback onWhitespace;

  @override
  Widget build(BuildContext context) {
    final change = this.change;
    if (change == null) return const SizedBox.shrink();
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
          _toolbar(),
          const SizedBox(height: WsMetrics.space4),
          _fileName(context, change.shown),
          Expanded(child: _body(context)),
          const SizedBox(height: WsMetrics.space4),
        ],
      ),
    );
  }

  static const double _oneLineWidth = 560;

  Widget _toolbar() => LayoutBuilder(
    builder: (context, constraints) {
      final switches = [
        _Toggle(
          label: settings.hideWhitespace
              ? 'Show whitespace changes'
              : 'Hide whitespace changes',
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
      ];
      final top = header ?? const SizedBox.shrink();
      if (constraints.maxWidth >= _oneLineWidth) {
        return Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Expanded(child: top),
            ...switches,
          ],
        );
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          top,
          const SizedBox(height: WsMetrics.space2),
          Wrap(
            alignment: WrapAlignment.end,
            crossAxisAlignment: WrapCrossAlignment.center,
            runSpacing: WsMetrics.space2,
            children: switches,
          ),
        ],
      );
    },
  );

  Widget _fileName(BuildContext context, String path) => Container(
    height: 36,
    padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
    decoration: BoxDecoration(
      color: context.ws.subtle,
      borderRadius: const BorderRadius.vertical(
        top: Radius.circular(WsMetrics.radiusLg),
      ),
    ),
    alignment: Alignment.centerLeft,
    child: Text(path, style: monoStyle(context)),
  );

  Widget _body(BuildContext context) {
    final note = this.note;
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border.all(color: context.ws.border),
        borderRadius: const BorderRadius.vertical(
          bottom: Radius.circular(WsMetrics.radiusLg),
        ),
      ),
      child: note != null
          ? Center(
              child: Text(note, style: Theme.of(context).textTheme.bodySmall),
            )
          : LayoutBuilder(builder: _lineList),
    );
  }

  Widget _lineList(BuildContext context, BoxConstraints constraints) {
    final style = DefaultTextStyle.of(context).style.merge(monoStyle(context));
    final scaler = MediaQuery.textScalerOf(context);
    final split = settings.layout == DiffLayout.split;
    final code =
        widest.fold(
          0.0,
          (wide, text) =>
              math.max(wide, measureText(text, style, scaler).width),
        ) +
        WsMetrics.space6;
    final rows = split ? splitRows(lines) : const <SplitRow>[];
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
          itemExtent: UnifiedLineRow.heightIn(scaler),
          itemCount: split ? rows.length : lines.length,
          itemBuilder: (context, index) => split
              ? SplitLineRow(row: rows[index])
              : UnifiedLineRow(line: lines[index]),
        ),
      ),
    );
  }
}

class _Toggle extends StatelessWidget {
  const _Toggle({required this.label, required this.on, required this.onTap});

  final String label;
  final bool on;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
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
