import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/thread.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'turn_scope.dart';

class TurnHeader extends StatelessWidget {
  const TurnHeader({
    super.key,
    required this.menuKey,
    required this.turns,
    required this.scope,
    required this.allLines,
    required this.repository,
    required this.onSelect,
    required this.onRevert,
  });

  final GlobalKey<PopupMenuButtonState<DiffScope>> menuKey;
  final List<BridgeTurn> turns;
  final DiffScope scope;

  final ({int added, int removed})? allLines;

  final bool repository;
  final ValueChanged<DiffScope> onSelect;

  final VoidCallback? onRevert;

  @override
  Widget build(BuildContext context) {
    final label = switch (scope) {
      LatestTurn() => 'Latest turn',
      OneTurn(:final number) => 'Turn $number',
      AllTurns() => 'All turns',
      BranchChanges() => 'Changes',
      UncommittedChanges() => 'Uncommitted',
    };
    return Wrap(
      spacing: WsMetrics.space3,
      runSpacing: WsMetrics.space2,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        _menu(context, label),
        if (onRevert case final revert?)
          SecondaryButton(
            label: 'Revert files to before this turn…',
            icon: LucideIcons.undo2,
            onPressed: revert,
          ),
      ],
    );
  }

  Widget _menu(BuildContext context, String label) {
    final ws = context.ws;
    final ended = finished(turns);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    PopupMenuItem<DiffScope> item(
      DiffScope value,
      String name,
      String detail,
    ) => PopupMenuItem(
      value: value,
      height: 32,
      child: Row(
        children: [
          Expanded(child: Text(name)),
          const SizedBox(width: WsMetrics.space5),
          Text(detail, style: small),
        ],
      ),
    );
    return PopupMenuButton<DiffScope>(
      key: menuKey,
      tooltip: 'Choose the turn',
      initialValue: scope,
      onSelected: onSelect,
      itemBuilder: (context) => [
        if (ended.isNotEmpty) ...[
          item(const LatestTurn(), 'Latest turn', ''),
          for (final turn in ended.reversed)
            item(
              OneTurn(turn.number),
              turnName(turn),
              turn.endedMs == null ? '' : formatClockMs(turn.endedMs!),
            ),
          item(
            const AllTurns(),
            'All turns',
            allLines == null ? '' : '+${allLines!.added} −${allLines!.removed}',
          ),
        ],
        if (repository) ...[
          if (ended.isNotEmpty) const PopupMenuDivider(),
          item(const BranchChanges(), 'Changes', ''),
          item(const UncommittedChanges(), 'Uncommitted', ''),
        ],
      ],
      child: Container(
        height: 28,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          border: Border.all(color: ws.borderStrong),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: Theme.of(context).textTheme.labelMedium
                    ?.copyWith(fontWeight: WsMetrics.fwSemibold),
              ),
            ),
            const SizedBox(width: WsMetrics.space2),
            Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
          ],
        ),
      ),
    );
  }
}
