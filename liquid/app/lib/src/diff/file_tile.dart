import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';

/// A folder row, which folds, or a changed file's row, which selects its diff.
class FileTile extends StatelessWidget {
  const FileTile({
    super.key,
    required this.row,
    required this.selected,
    required this.folded,
    required this.onTap,
    required this.onFold,
  });

  final FileRow row;
  final bool selected;

  /// Whether this folder row is folded.
  final bool folded;
  final ValueChanged<BridgeFileChange> onTap;
  final ValueChanged<String> onFold;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final change = row.change;
    Widget letter(String mark, Color color) => Text(
      mark,
      style: TextStyle(
        color: color,
        fontSize: WsMetrics.fsSmall,
        fontWeight: WsMetrics.fwSemibold,
      ),
    );
    final lead = switch (change?.status) {
      BridgeChangeStatus.added => letter('A', ws.success),
      BridgeChangeStatus.deleted => letter('D', ws.danger),
      BridgeChangeStatus.modified => letter('M', ws.warning),
      null => Icon(
        folded ? LucideIcons.chevronRight : LucideIcons.chevronDown,
        size: 14,
        color: ws.textFaint,
      ),
    };
    return InkWell(
      canRequestFocus: false,
      onTap: change == null ? () => onFold(row.folder) : () => onTap(change),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      child: Container(
        height: WsMetrics.rowNav,
        padding: EdgeInsets.only(
          left: WsMetrics.space3 + row.depth * 16,
          right: WsMetrics.space3,
        ),
        decoration: BoxDecoration(
          color: selected ? ws.selected : null,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          children: [
            SizedBox(width: 16, child: lead),
            if (change == null) ...[
              const SizedBox(width: WsMetrics.space1),
              Icon(LucideIcons.folder, size: 14, color: ws.textFaint),
            ],
            const SizedBox(width: WsMetrics.space2),
            Expanded(
              child: Text(
                row.name,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: selected ? ws.text : ws.textMuted,
                  fontSize: WsMetrics.fsCompact,
                ),
              ),
            ),
            if (change != null)
              Text.rich(
                TextSpan(
                  children: [
                    if (change.added > 0)
                      TextSpan(
                        text: '+${change.added} ',
                        style: TextStyle(color: ws.success),
                      ),
                    if (change.removed > 0)
                      TextSpan(
                        text: '−${change.removed}',
                        style: TextStyle(color: ws.danger),
                      ),
                  ],
                ),
                style: const TextStyle(fontSize: WsMetrics.fsSmall),
              ),
          ],
        ),
      ),
    );
  }
}
