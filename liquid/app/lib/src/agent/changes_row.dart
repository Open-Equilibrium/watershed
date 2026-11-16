import 'package:flutter/material.dart';

import '../diff/turn_scope.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';

/// The Changes row of the session card, as T3 Code's thread details show it: the branch's lines
/// against its base ("vs main"), or the uncommitted changes when the branch has no lines against
/// its base. A repository shows the row even with no changes; a folder outside Git shows none.
class ChangesRow extends StatelessWidget {
  const ChangesRow({super.key, required this.state, this.onOpen});

  final SessionState state;

  /// Opens the Diff panel at the changes the row names.
  final ValueChanged<DiffScope>? onOpen;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<GitState>(
    valueListenable: state.git,
    builder: (context, git, _) => ValueListenableBuilder<BridgeGitStatus?>(
      valueListenable: git.status,
      builder: (context, status, _) {
        if (status == null || !status.repository) {
          return const SizedBox.shrink();
        }
        final ws = context.ws;
        final branch = status.branchAdded > 0 || status.branchRemoved > 0;
        final working = status.workingAdded > 0 || status.workingRemoved > 0;
        final useBranch = branch || !working;
        final base = useBranch ? status.branchBase : null;
        final added = useBranch ? status.branchAdded : status.workingAdded;
        final removed = useBranch
            ? status.branchRemoved
            : status.workingRemoved;
        final label = base == null ? 'Uncommitted' : 'vs $base';
        final numbers = monoStyle(context).copyWith(
          fontSize: WsMetrics.fsSmall,
          fontWeight: WsMetrics.fwRegular,
        );
        return CardRow(
          key: const Key('changes-row'),
          onTap: onOpen == null
              ? null
              : () => onOpen!(
                  base == null
                      ? const UncommittedChanges()
                      : const BranchChanges(),
                ),
          children: [
            Icon(LucideIcons.fileDiff, size: 16, color: ws.textFaint),
            const Text('Changes'),
            Flexible(
              child: Tooltip(
                message: label,
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    fontWeight: WsMetrics.fwRegular,
                    color: ws.textMuted,
                  ),
                ),
              ),
            ),
            Text(
              '+$added',
              style: numbers.copyWith(
                color: added > 0 ? ws.success : ws.textMuted,
              ),
            ),
            Text(
              '−$removed',
              style: numbers.copyWith(
                color: removed > 0 ? ws.danger : ws.textMuted,
              ),
            ),
          ],
        );
      },
    ),
  );
}
