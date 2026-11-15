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
import 'pull_request.dart';
import 'session_card_rows.dart';

/// Pushes the branch, asking first before pushing to the remote's default branch; a completed
/// push to another branch offers a pull request unless `offer` is false. Returns the last
/// push's outcome, none when the user did not confirm or the push could not run.
Future<BridgeGitOutcome?> pushBranch(
  BuildContext context,
  SessionState state, {
  required String device,
  bool offer = true,
}) async {
  final generation = state.generation;
  BridgePushConfirmation? confirmed;
  while (context.mounted && generation == state.generation) {
    final outcome = await state.git.push(device: device, confirmed: confirmed);
    if (outcome?.kind == BridgeGitOutcomeKind.done &&
        confirmed == null &&
        offer) {
      unawaited(state.pulls.pushed(outcome!.transfer!));
    }
    if (outcome?.kind != BridgeGitOutcomeKind.pushDefault ||
        !context.mounted ||
        generation != state.generation) {
      return outcome;
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
    if (!context.mounted || generation != state.generation) return null;
    if (decision == true) {
      confirmed = transfer.confirmation;
      continue;
    } else if (decision == false) {
      await showBranchMenu(context, state);
    }
    return null;
  }
  return null;
}

enum _GitAction { commit, compose, push, pull, fetch, create, pullRequest }

const _nothingToPush = 'Branch is up to date. Nothing to push.';
const _nothingToCompose =
    'Nothing to commit or push. Create pull request… opens one for a pushed branch.';

const _labels = {
  _GitAction.commit: 'Commit',
  _GitAction.push: 'Push',
  _GitAction.pull: 'Pull',
  _GitAction.create: 'Create branch…',
};

const _icons = {
  _GitAction.commit: LucideIcons.gitCommit,
  _GitAction.compose: LucideIcons.gitPullRequest,
  _GitAction.push: LucideIcons.arrowUp,
  _GitAction.pull: LucideIcons.arrowDown,
  _GitAction.create: LucideIcons.gitBranch,
  _GitAction.pullRequest: LucideIcons.gitPullRequest,
};

/// What each Git action offers for one status: why it is blocked, the hint and count it shows,
/// and the step the split button runs next.
class _GitMenu {
  _GitMenu(this.status, this.device);

  final BridgeGitStatus status;
  final String device;
  late final detached = status.branch == null;
  late final remote = status.remote;
  late final diverged = status.ahead > 0 && status.behind > 0;
  late final divergence = diverged
      ? divergedHint(status.branch!, status.upstream!)
      : '';
  late final noDestination = status.hasRemotes
      ? noUpstreamRemoteHint
      : noRemoteHint;
  late final blockers = {
    for (final action in _GitAction.values) action: _blocker(action),
  };
  late final enabled = {
    for (final action in _GitAction.values) action: blockers[action] == null,
  };
  late final next = status.merging
      ? _GitAction.commit
      : detached
      ? _GitAction.create
      : enabled[_GitAction.commit]! || remote == null || status.commit == null
      ? _GitAction.commit
      : enabled[_GitAction.push]! || status.ahead > 0
      ? _GitAction.push
      : enabled[_GitAction.pull]!
      ? _GitAction.pull
      : _GitAction.commit;
  late final hints = {
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
    _GitAction.compose:
        blockers[_GitAction.compose] ??
        (status.changed == 0
            ? 'Push ${status.branch} and review a pull request on GitHub in one dialog.'
            : 'Commit, push and review a pull request on GitHub in one dialog.'),
    _GitAction.fetch: blockers[_GitAction.fetch] ?? 'Fetch from $remote.',
    _GitAction.create:
        blockers[_GitAction.create] ??
        (detached
            ? detachedCommitHint
            : 'Create a branch from ${status.branch}.'),
    _GitAction.pullRequest:
        blockers[_GitAction.pullRequest] ??
        'Review a pull request for ${status.branch ?? 'this branch'} on GitHub before gh on $device creates it.',
  };
  late final counts = {
    _GitAction.commit: enabled[_GitAction.commit]! ? '${status.changed}' : '',
    _GitAction.push: !diverged && status.ahead > 0 ? '↑${status.ahead}' : '',
    _GitAction.pull: status.behind > 0 ? '↓${status.behind}' : '',
  };

  String? _blocker(_GitAction action) {
    if (status.merging && action != _GitAction.fetch) {
      return mergeHint(status);
    }
    if (action == _GitAction.create || action == _GitAction.pullRequest) {
      return null;
    }
    if (detached && action != _GitAction.fetch) return detachedCommitHint;
    switch (action) {
      case _GitAction.commit:
        return status.changed == 0
            ? 'Branch is up to date. Nothing to commit.'
            : null;
      case _GitAction.compose:
        // It pushes what Push would, after committing any changes.
        final push = _blocker(_GitAction.push);
        if (push != _nothingToPush) return push;
        return status.changed == 0 ? _nothingToCompose : null;
      case _GitAction.push:
        if (status.commit == null) {
          return 'Commit before pushing this branch.';
        }
        if (remote == null) return noDestination;
        if (diverged) return divergence;
        if (status.behind > 0) {
          return behindHint(status.branch!, status.upstream!, status.behind);
        }
        return status.upstream != null && !status.gone && status.ahead == 0
            ? _nothingToPush
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
      case _GitAction.create || _GitAction.pullRequest:
        return null;
    }
  }
}

/// The session card's Git action row: the next single Git step, and the approved, explicitly
/// invoked Git menu behind its chevron.
class GitActions extends StatelessWidget {
  const GitActions({super.key, required this.state, required this.device});
  final SessionState state;
  final String device;

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
      if (status == null) return _initialize(context);
      final menu = _GitMenu(status, device);
      return SizedBox(
        height: 32,
        child: Row(
          children: [
            Expanded(child: _nextButton(context, menu)),
            _menuButton(context, menu),
          ],
        ),
      );
    },
  );

  /// Offers to create a repository in a folder that has none.
  Widget _initialize(BuildContext context) => Tooltip(
    message: 'This folder has no Git repository. Initialize Git creates one; checkpoints start with the next turn.',
    child: CardRow(
      onTap: state.git.writing.value ? null : state.git.initialize,
      children: [
        Icon(LucideIcons.plus, size: 16, color: context.ws.textFaint),
        const Text('Initialize Git'),
      ],
    ),
  );

  void _run(BuildContext context, _GitAction action) {
    final git = state.git;
    switch (action) {
      case _GitAction.commit:
        unawaited(showCommitDialog(context, state));
      case _GitAction.push:
        unawaited(pushBranch(context, state, device: device));
      case _GitAction.pull:
        unawaited(git.pull(device: device));
      case _GitAction.fetch:
        unawaited(git.fetchNow());
      case _GitAction.create:
        unawaited(showBranchMenu(context, state));
      case _GitAction.compose:
        unawaited(
          showPullRequestDialog(context, state, device: device, compose: true),
        );
      case _GitAction.pullRequest:
        unawaited(showPullRequestDialog(context, state, device: device));
    }
  }

  /// The row's label: the next step, with its icon or progress, label and count.
  Widget _nextButton(BuildContext context, _GitMenu menu) {
    final git = state.git;
    final ws = context.ws;
    final next = menu.next;
    final counted =
        git.progress.value == null && (menu.counts[next] ?? '').isNotEmpty;
    return Tooltip(
      message: menu.hints[next]!,
      child: TextButton(
        key: const Key('git-next'),
        onPressed: menu.enabled[next]! && !git.writing.value
            ? () => _run(context, next)
            : null,
        style: TextButton.styleFrom(
          foregroundColor: ws.text,
          overlayColor: Colors.transparent,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          minimumSize: Size.zero,
          alignment: Alignment.centerLeft,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          ),
          textStyle: const TextStyle(
            fontSize: WsMetrics.fsCompact,
            fontWeight: WsMetrics.fwMedium,
          ),
        ),
        child: Row(
          spacing: 10,
          children: [
            if (git.progress.value == null)
              Icon(_icons[next], size: 16, color: ws.textFaint)
            else
              const SizedBox.square(
                dimension: 14,
                child: CircularProgressIndicator(strokeWidth: 1.5),
              ),
            Flexible(
              child: Text(
                git.progress.value ?? _labels[next]!,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (counted)
              Text(
                menu.counts[next]!,
                style: TextStyle(
                  color: ws.textMuted,
                  fontSize: WsMetrics.fsSmall,
                  fontWeight: WsMetrics.fwRegular,
                ),
              ),
          ],
        ),
      ),
    );
  }

  /// The chevron's menu of every Git action, headed by the branch and when it was fetched.
  Widget _menuButton(BuildContext context, _GitMenu menu) {
    final git = state.git;
    final status = menu.status;
    final remote = menu.remote;
    final fetched = git.fetched;
    final heading =
        '${status.branch ?? status.commit ?? 'no commit'}${status.upstream == null ? '' : ' → ${status.upstream}'}${fetched == null ? '' : ' · fetched ${formatClockMs(fetched.millisecondsSinceEpoch)}'}';
    return SizedBox(
      width: 32,
      child: PopupMenuButton<_GitAction>(
        key: const Key('git-menu'),
        tooltip: 'Git',
        enabled: !git.writing.value,
        padding: EdgeInsets.zero,
        icon: Icon(
          LucideIcons.chevronDown,
          size: 16,
          color: context.ws.textFaint,
        ),
        onSelected: (action) => _run(context, action),
        itemBuilder: (_) => [
          PopupMenuItem(enabled: false, child: Text(heading)),
          _item(
            menu,
            _GitAction.commit,
            'Commit…',
            menu.enabled[_GitAction.commit]!
                ? plural(status.changed, 'file')
                : '',
          ),
          _item(
            menu,
            _GitAction.compose,
            'Commit, push & create PR…',
            menu.blockers[_GitAction.compose] == _nothingToCompose
                ? 'no changes'
                : '',
          ),
          _item(menu, _GitAction.push, 'Push', menu.counts[_GitAction.push]!),
          _item(menu, _GitAction.pullRequest, 'Create pull request…', ''),
          _item(
            menu,
            _GitAction.pull,
            'Pull',
            remote != null &&
                    status.upstream != null &&
                    !menu.detached &&
                    status.behind == 0
                ? 'up to date'
                : menu.counts[_GitAction.pull]!,
          ),
          _item(menu, _GitAction.fetch, 'Fetch', remote ?? ''),
          const PopupMenuDivider(),
          _item(menu, _GitAction.create, 'Create branch…', ''),
        ],
      ),
    );
  }

  /// One entry of the Git menu, with its hint as tooltip.
  PopupMenuItem<_GitAction> _item(
    _GitMenu menu,
    _GitAction action,
    String label,
    String trailing,
  ) => PopupMenuItem(
    value: action,
    enabled: menu.enabled[action]! && !state.git.writing.value,
    child: Tooltip(
      message: menu.hints[action]!,
      child: Row(
        children: [
          SizedBox(
            width: 14,
            child: action == _GitAction.fetch
                ? null
                : Icon(_icons[action], size: 14),
          ),
          const SizedBox(width: WsMetrics.space3),
          Flexible(
            child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          const Spacer(),
          if (trailing.isNotEmpty) Text(trailing),
        ],
      ),
    ),
  );
}
