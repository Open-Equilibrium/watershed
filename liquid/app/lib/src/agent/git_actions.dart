import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'commit_dialog.dart';

enum _GitAction { commit, push, pull, fetch, create }

/// The next single Git step and the approved, explicitly invoked Git menu.
class GitActions extends StatelessWidget {
  const GitActions({super.key, required this.state, required this.device});
  final SessionState state;
  final String device;

  Future<void> _push(BuildContext context) async {
    final generation = state.generation;
    BridgePushConfirmation? confirmed;
    while (context.mounted && generation == state.generation) {
      final outcome = await state.git.push(
        device: device,
        confirmed: confirmed,
      );
      if (outcome?.kind != BridgeGitOutcomeKind.pushDefault ||
          !context.mounted ||
          generation != state.generation) {
        return;
      }
      final branch = outcome!.transfer!.destination;
      final transfer = outcome.transfer!;
      final upstream = transfer.upstream;
      final count = transfer.ahead;
      final decision = await showDialog<bool>(
        context: context,
        builder: (context) {
          void push() => Navigator.of(context).pop(true);
          return ActionDialog(
            title: 'Push to $branch?',
            bindings: {const Keys(LogicalKeyboardKey.enter).activator: push},
            actions: [
              PrimaryButton(
                label: 'Push to $branch',
                shortcut: '⏎',
                onPressed: push,
              ),
              SecondaryButton(
                label: 'Create branch…',
                onPressed: () => Navigator.of(context).pop(false),
              ),
            ],
            children: [
              Text(
                "This pushes ${plural(count, 'commit')} directly to $upstream, the repository's default branch. You can push it as it is, or create a branch first and push that.",
                style: DangerDialog.text(context),
              ),
            ],
          );
        },
      );
      if (!context.mounted || generation != state.generation) return;
      if (decision == true) {
        confirmed = transfer.confirmation;
        continue;
      } else if (decision == false) {
        await showBranchMenu(context, state);
      }
      return;
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.git.status,
      state.git.refreshed,
      state.git.problem,
      state.git.loaded,
      state.git.writing,
      state.git.progress,
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
      final remote = status.remote;
      final diverged = status.ahead > 0 && status.behind > 0;
      final divergence = diverged
          ? divergedHint(status.branch!, status.upstream!)
          : '';
      final noDestination = status.hasRemotes
          ? noUpstreamRemoteHint
          : noRemoteHint;
      String? blocker(_GitAction action) {
        if (status.merging && action != _GitAction.fetch) {
          return mergeHint(status);
        }
        if (action == _GitAction.create) return null;
        if (detached && action != _GitAction.fetch) return detachedCommitHint;
        switch (action) {
          case _GitAction.commit:
            return status.changed == 0
                ? 'Branch is up to date. Nothing to commit.'
                : null;
          case _GitAction.push:
            if (status.commit == null) {
              return 'Commit before pushing this branch.';
            }
            if (remote == null) return noDestination;
            if (diverged) return divergence;
            if (status.behind > 0) {
              return behindHint(
                status.branch!,
                status.upstream!,
                status.behind,
              );
            }
            return status.upstream != null && !status.gone && status.ahead == 0
                ? 'Branch is up to date. Nothing to push.'
                : null;
          case _GitAction.pull:
            if (remote == null) return noDestination;
            if (diverged) return divergence;
            if (status.upstream == null || status.gone) return noUpstreamHint;
            return status.behind == 0
                ? 'Branch is up to date. Nothing to pull.'
                : null;
          case _GitAction.fetch:
            return remote == null || remote == '.' ? noDestination : null;
          case _GitAction.create:
            return null;
        }
      }

      final blockers = {
        for (final action in _GitAction.values) action: blocker(action),
      };
      final enabled = {
        for (final action in _GitAction.values)
          action: blockers[action] == null,
      };
      final next = status.merging
          ? _GitAction.commit
          : detached
          ? _GitAction.create
          : enabled[_GitAction.commit]! ||
                remote == null ||
                status.commit == null
          ? _GitAction.commit
          : enabled[_GitAction.push]! || status.ahead > 0
          ? _GitAction.push
          : enabled[_GitAction.pull]!
          ? _GitAction.pull
          : _GitAction.commit;
      final labels = {
        _GitAction.commit: 'Commit',
        _GitAction.push: 'Push',
        _GitAction.pull: 'Pull',
        _GitAction.create: 'Create branch…',
      };
      final icons = {
        _GitAction.commit: LucideIcons.gitCommit,
        _GitAction.push: LucideIcons.arrowUp,
        _GitAction.pull: LucideIcons.arrowDown,
        _GitAction.create: LucideIcons.gitBranch,
      };
      final hints = {
        _GitAction.commit:
            blockers[_GitAction.commit] ??
            (remote == null
                ? noDestination
                : 'Commit ${plural(status.changed, 'changed file')} on ${status.branch}.'),
        _GitAction.push:
            blockers[_GitAction.push] ??
            (status.upstream == null || status.gone
                ? 'Push ${status.branch} to $remote and track it.'
                : 'Push ${plural(status.ahead, 'commit')} to ${status.upstream}.'),
        _GitAction.pull:
            blockers[_GitAction.pull] ??
            'Pull ${plural(status.behind, 'commit')} from ${status.upstream}.',
        _GitAction.fetch: blockers[_GitAction.fetch] ?? 'Fetch from $remote.',
        _GitAction.create:
            blockers[_GitAction.create] ??
            (detached
                ? detachedCommitHint
                : 'Create a branch from ${status.branch}.'),
      };
      final counts = {
        _GitAction.commit: enabled[_GitAction.commit]!
            ? '${status.changed}'
            : '',
        _GitAction.push: !diverged && status.ahead > 0
            ? '↑${status.ahead}'
            : '',
        _GitAction.pull: status.behind > 0 ? '↓${status.behind}' : '',
      };
      void run(_GitAction action) {
        switch (action) {
          case _GitAction.commit:
            unawaited(showCommitDialog(context, state));
          case _GitAction.push:
            unawaited(_push(context));
          case _GitAction.pull:
            unawaited(git.pull(device: device));
          case _GitAction.fetch:
            unawaited(git.fetchNow());
          case _GitAction.create:
            unawaited(showBranchMenu(context, state));
        }
      }

      final fetched = git.fetched;
      final heading =
          '${status.branch ?? status.commit ?? 'no commit'}${status.upstream == null ? '' : ' → ${status.upstream}'}${fetched == null ? '' : ' · fetched ${formatClockMs(fetched.millisecondsSinceEpoch)}'}';
      PopupMenuItem<_GitAction> item(
        _GitAction action,
        String label,
        String trailing,
      ) => PopupMenuItem(
        value: action,
        enabled: enabled[action]! && !git.writing.value,
        child: Tooltip(
          message: hints[action]!,
          child: Row(
            children: [
              SizedBox(
                width: 14,
                child: action == _GitAction.fetch
                    ? null
                    : Icon(icons[action], size: 14),
              ),
              const SizedBox(width: WsMetrics.space3),
              Text(label),
              const Spacer(),
              if (trailing.isNotEmpty) Text(trailing),
            ],
          ),
        ),
      );
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
              message: hints[next]!,
              child: TextButton(
                key: const Key('git-next'),
                onPressed: enabled[next]! && !git.writing.value
                    ? () => run(next)
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
                    if (git.progress.value == null)
                      Icon(icons[next], size: 14)
                    else
                      const SizedBox.square(
                        dimension: 14,
                        child: CircularProgressIndicator(strokeWidth: 1.5),
                      ),
                    Text(git.progress.value ?? labels[next]!),
                    if (git.progress.value == null &&
                        (counts[next] ?? '').isNotEmpty)
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
                          counts[next]!,
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
              enabled: !git.writing.value,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
              icon: const Icon(LucideIcons.chevronDown, size: 13),
              onSelected: run,
              itemBuilder: (_) => [
                PopupMenuItem(enabled: false, child: Text(heading)),
                item(
                  _GitAction.commit,
                  'Commit…',
                  enabled[_GitAction.commit]!
                      ? plural(status.changed, 'file')
                      : '',
                ),
                item(_GitAction.push, 'Push', counts[_GitAction.push]!),
                item(
                  _GitAction.pull,
                  'Pull',
                  remote != null &&
                          status.upstream != null &&
                          !detached &&
                          status.behind == 0
                      ? 'up to date'
                      : counts[_GitAction.pull]!,
                ),
                item(_GitAction.fetch, 'Fetch', remote ?? ''),
                const PopupMenuDivider(),
                item(_GitAction.create, 'Create branch…', ''),
              ],
            ),
          ],
        ),
      );
    },
  );
}
