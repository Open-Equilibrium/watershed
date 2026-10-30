import 'package:flutter/material.dart';

import '../controls.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// One row (approved look mockup): the view switch on the left, the summary and Compare on the
/// right.
class TrajectoryToolbar extends StatelessWidget {
  const TrajectoryToolbar({
    super.key,
    required this.equalWidth,
    required this.comparing,
    required this.summary,
    required this.range,
    required this.onClearRange,
    required this.filters,
    required this.onFilter,
    required this.onEqualWidth,
    required this.onCompare,
  });

  final bool equalWidth;
  final bool comparing;
  final String summary;

  /// The range limiting the list, while there is one.
  final String? range;
  final VoidCallback onClearRange;

  /// How many filters are on.
  final int filters;
  final VoidCallback onFilter;
  final ValueChanged<bool> onEqualWidth;
  final VoidCallback onCompare;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final keys = Keymap.of(context);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Container(
      height: 40,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      child: Row(
        children: [
          Segments(
            options: const [(false, 'Real time'), (true, 'Equal width')],
            selected: equalWidth,
            onSelect: onEqualWidth,
          ),
          if (!comparing) ...[
            const SizedBox(width: WsMetrics.space3),
            GhostButton(
              icon: LucideIcons.listFilter,
              label: filters == 0 ? 'Filter' : 'Filter · $filters',
              shortcut: keys.label(Command.filterSteps),
              onPressed: onFilter,
            ),
          ],
          Expanded(
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (range case final range?) ...[
                  Container(
                    key: const Key('range'),
                    padding: const EdgeInsets.only(left: WsMetrics.space3),
                    decoration: BoxDecoration(
                      color: ws.accentSubtle,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      children: [
                        Text(
                          'Range $range',
                          style: small.copyWith(color: ws.accent),
                        ),
                        Tooltip(
                          message: 'Clear the range',
                          child: InkWell(
                            onTap: onClearRange,
                            borderRadius: BorderRadius.circular(
                              WsMetrics.radiusMd,
                            ),
                            child: Padding(
                              padding: const EdgeInsets.all(WsMetrics.space2),
                              child: Icon(
                                LucideIcons.x,
                                size: 12,
                                color: ws.accent,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space4),
                ],
                Flexible(
                  child: Text(
                    summary,
                    style: small,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: WsMetrics.space4),
          if (comparing)
            // Clicking it, return or space leave the comparison, as its key does.
            Shortcuts(
              shortcuts: activateKeys,
              child: InkWell(
                onTap: onCompare,
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                child: Container(
                  height: 28,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                  ),
                  decoration: BoxDecoration(
                    color: ws.accentSubtle,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: Row(
                    children: [
                      Text(
                        'Comparing',
                        style: TextStyle(
                          color: ws.accent,
                          fontSize: WsMetrics.fsCompact,
                        ),
                      ),
                      const SizedBox(width: WsMetrics.space3),
                      Hint(keys.label(Command.compare)),
                    ],
                  ),
                ),
              ),
            )
          else
            SecondaryButton(
              icon: LucideIcons.arrowLeftRight,
              label: 'Compare',
              shortcut: keys.label(Command.compare),
              onPressed: onCompare,
            ),
        ],
      ),
    );
  }
}
