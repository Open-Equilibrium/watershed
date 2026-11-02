import 'dart:async';

import 'package:flutter/material.dart';

import '../format.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'commit_dialog.dart';

enum _GitAction { commit, create }

/// The next single Git step and the approved, explicitly invoked Git menu.
class GitActions extends StatelessWidget {
  const GitActions({super.key, required this.state});
  final SessionState state;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.git.status,
      state.git.problem,
      state.git.loaded,
      state.git.writing,
    ]),
    builder: (context, _) {
      final git = state.git;
      if (!git.loaded.value || git.problem.value != null) {
        return const SizedBox.shrink();
      }
      final status = git.status.value;
      if (status == null) {
        return Tooltip(
          message: 'This folder has no Git repository. Initialize Git creates one; checkpoints start with the next turn.',
          child: OutlinedButton.icon(
            onPressed: git.writing.value ? null : git.initialize,
            icon: const Icon(LucideIcons.plus, size: 14),
            label: const Text('Initialize Git'),
          ),
        );
      }
      final detached = status.branch == null;
      final label = detached ? 'Create branch…' : 'Commit';
      final canCommit =
          !git.writing.value &&
          !detached &&
          !status.merging &&
          status.changed > 0;
      final enabled =
          canCommit || (detached && !status.merging && !git.writing.value);
      final hint = status.merging
          ? mergeHint(status)
          : detached
          ? detachedCommitHint
          : status.changed == 0
          ? status.ahead == 0 && status.behind == 0
                ? 'Branch is up to date. Nothing to commit.'
                : 'Nothing to commit.'
          : 'Commit ${status.changed} changed ${status.changed == 1 ? 'file' : 'files'} on ${status.branch}.';
      void commit() => unawaited(showCommitDialog(context, state));
      void create() => unawaited(showBranchMenu(context, state));
      final fetched = git.fetched;
      final heading =
          '${status.branch ?? status.commit ?? 'no commit'}${status.upstream == null ? '' : ' → ${status.upstream}'}${fetched == null ? '' : ' · fetched ${formatClockMs(fetched.millisecondsSinceEpoch)}'}';
      final ws = context.ws;
      return Container(
        height: 28,
        decoration: BoxDecoration(
          border: Border.all(color: ws.borderStrong),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Tooltip(
              message: hint,
              child: TextButton(
                key: const Key('git-next'),
                onPressed: enabled
                    ? detached
                          ? create
                          : commit
                    : null,
                style: TextButton.styleFrom(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  minimumSize: Size.zero,
                ),
                child: Row(
                  spacing: WsMetrics.space2,
                  children: [
                    Icon(
                      detached ? LucideIcons.gitBranch : LucideIcons.gitCommit,
                      size: 14,
                    ),
                    Text(label),
                    if (canCommit)
                      Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: WsMetrics.space2,
                        ),
                        decoration: BoxDecoration(
                          color: ws.hover,
                          borderRadius: BorderRadius.circular(
                            WsMetrics.radiusSm,
                          ),
                        ),
                        child: Text(
                          '${status.changed}',
                          style: TextStyle(
                            color: ws.textMuted,
                            fontSize: WsMetrics.fsSmall,
                            height: 1.5,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
            Container(width: 1, color: ws.borderStrong),
            PopupMenuButton<_GitAction>(
              key: const Key('git-menu'),
              tooltip: 'Git',
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              icon: const Icon(LucideIcons.chevronDown, size: 13),
              onSelected: (value) {
                if (value == _GitAction.commit) {
                  commit();
                } else {
                  create();
                }
              },
              itemBuilder: (_) => [
                PopupMenuItem(enabled: false, child: Text(heading)),
                PopupMenuItem(
                  value: _GitAction.commit,
                  enabled: canCommit,
                  child: Tooltip(
                    message: hint,
                    child: Row(
                      children: [
                        const Icon(LucideIcons.gitCommit, size: 14),
                        const SizedBox(width: WsMetrics.space3),
                        const Text('Commit…'),
                        const Spacer(),
                        if (status.changed > 0)
                          Text(
                            '${status.changed} ${status.changed == 1 ? 'file' : 'files'}',
                          ),
                      ],
                    ),
                  ),
                ),
                const PopupMenuDivider(),
                PopupMenuItem(
                  value: _GitAction.create,
                  enabled: !status.merging && !git.writing.value,
                  child: const Row(
                    children: [
                      Icon(LucideIcons.gitBranch, size: 14),
                      SizedBox(width: WsMetrics.space3),
                      Text('Create branch…'),
                    ],
                  ),
                ),
              ],
            ),
          ],
        ),
      );
    },
  );
}
