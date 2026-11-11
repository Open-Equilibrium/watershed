import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/git.dart';
import '../rust/api/history.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'turn_scope.dart';

/// The turn menu with what the shown turns are: their prompt, their time and what they are
/// compared with, or the branch's stashed changes with Restore and Discard.
class TurnHeader extends StatelessWidget {
  const TurnHeader({
    super.key,
    required this.menuKey,
    required this.turns,
    required this.scope,
    required this.allLines,
    required this.shown,
    required this.stash,
    required this.branch,
    required this.git,
    required this.onSelect,
    required this.onRevert,
    required this.onRestore,
    required this.onDiscard,
  });

  final GlobalKey<PopupMenuButtonState<DiffScope>> menuKey;
  final List<BridgeTurn> turns;
  final DiffScope scope;

  /// The lines all turns added and removed, once read.
  final ({int added, int removed})? allLines;

  /// The turns shown; none before a turn ended.
  final Compared? shown;
  final BridgeStash? stash;
  final String? branch;

  /// The checkout's Git state, whose uncommitted changes allow no restore.
  final ValueListenable<BridgeGitStatus?> git;
  final ValueChanged<DiffScope> onSelect;

  /// Asks to revert the files to before the shown turn; none unless one turn is shown.
  final VoidCallback? onRevert;
  final VoidCallback onRestore;
  final VoidCallback onDiscard;

  @override
  Widget build(BuildContext context) {
    final stash = this.stash;
    final stashed = scope is StashedChanges && stash != null;
    final (:label, :about) = _names();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            _menu(context, label),
            // A narrow tab shortens the label rather than the menu.
            if (stashed)
              Expanded(child: _stashActions(stash))
            else if (onRevert case final revert?)
              Expanded(
                child: Align(
                  alignment: Alignment.centerRight,
                  child: SecondaryButton(
                    label: 'Revert files to before this turn…',
                    icon: LucideIcons.undo2,
                    onPressed: revert,
                  ),
                ),
              ),
          ],
        ),
        const SizedBox(height: WsMetrics.space1),
        Text(
          about,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.bodySmall,
        ),
      ],
    );
  }

  /// The menu's label and what the shown turns are, in words.
  ({String label, String about}) _names() {
    final shown = this.shown;
    final stash = this.stash;
    final stashed = scope is StashedChanges && stash != null;
    final all = scope is AllTurns;
    final String about;
    final String label;
    if (scope is UncommittedChanges) {
      label = 'Uncommitted changes';
      about = 'Compared with the checkout’s last commit';
    } else if (stashed) {
      label = 'Stashed changes';
      about =
          'Left on $branch at ${formatClockMs(stash.createdMs)} · Restore moves the stashed '
          'files back to your uncommitted changes';
    } else if (shown != null) {
      final turn = shown.to;
      final started = shown.from.startedMs;
      final since = started == null ? '' : formatClockMs(started);
      final until = turn.endedMs == null ? '' : formatClockMs(turn.endedMs!);
      label = all ? 'All turns' : turnName(turn);
      about = all
          ? 'All turns of this session branch · since $since · compared with the project '
                'when the first turn started'
          : '“${turn.prompt}” · $since–$until · compared with the project when the turn '
                'started';
    } else {
      label = 'Choose changes';
      about = stash == null
          ? 'Compare uncommitted changes.'
          : 'Compare uncommitted changes or the branch’s stashed changes.';
    }
    return (label: label, about: about);
  }

  /// The menu button, labelled `label`, that chooses the turns and the changes to compare.
  Widget _menu(BuildContext context, String label) {
    final ws = context.ws;
    final stash = this.stash;
    final ended = finished(turns);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    String counts(BridgeTurn turn) => [
      if (turn.endedMs case final at?) formatClockMs(at),
      if (turn.failure != null)
        'failed'
      else if ((turn.added, turn.removed) case (final added?, final removed?))
        '+$added −$removed',
    ].join(' · ');
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
          PopupMenuItem(
            enabled: false,
            height: 24,
            child: Text('Turns of this session branch', style: small),
          ),
          item(
            const LatestTurn(),
            'Latest turn · ${turnName(ended.last)}',
            counts(ended.last),
          ),
          for (final older in ended.reversed.skip(1))
            item(OneTurn(older.id), turnName(older), counts(older)),
          const PopupMenuDivider(),
          item(
            const AllTurns(),
            'All turns',
            [
              if (turns.first.startedMs case final at?)
                'since ${formatClockMs(at)}',
              if (allLines case final lines?)
                '+${lines.added} −${lines.removed}',
            ].join(' · '),
          ),
        ],
        if (ended.isNotEmpty && (git.value != null || stash != null))
          const PopupMenuDivider(),
        if (git.value != null) ...[
          item(const UncommittedChanges(), 'Uncommitted changes', ''),
        ],
        if (stash != null) ...[
          item(
            const StashedChanges(),
            'Stashed changes',
            'left at ${formatClockMs(stash.createdMs)}',
          ),
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
            Text(
              label,
              style: Theme.of(context).textTheme.labelMedium
                  ?.copyWith(fontWeight: WsMetrics.fwSemibold),
            ),
            const SizedBox(width: WsMetrics.space2),
            Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
          ],
        ),
      ),
    );
  }

  /// Restore and Discard for the stashed changes.
  Widget _stashActions(BridgeStash stash) => Row(
    mainAxisAlignment: MainAxisAlignment.end,
    spacing: WsMetrics.space3,
    children: [
      // As GitHub Desktop, only into a checkout without changes, and only on
      // the tree the changes were left on, where Git applies them unmerged.
      ValueListenableBuilder(
        valueListenable: git,
        builder: (context, status, _) {
          final why = !stash.current
              ? 'The branch has new commits since these changes were left, so '
                    'Liquid cannot restore them. Restore them with Git.'
              : (status?.changed ?? 0) > 0
              ? 'Unable to restore stash when changes are present on your '
                    'branch.'
              : null;
          final button = PrimaryButton(
            label: 'Restore',
            onPressed: why == null && status?.branch == branch
                ? onRestore
                : null,
          );
          return why == null ? button : Tooltip(message: why, child: button);
        },
      ),
      SecondaryButton(label: 'Discard…', onPressed: onDiscard),
    ],
  );
}
