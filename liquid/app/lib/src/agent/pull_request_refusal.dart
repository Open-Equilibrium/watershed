import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/git.dart';
import '../session/pull_requests.dart';
import '../session/session_state.dart';
import 'session_menus.dart';

/// What the dialog hands off to when the branch needs another step first.
enum PullRequestNext { push, branch, commit, compose }

/// Why the plan could not be read, and the steps that resolve it.
class PullRequestRefusal extends StatelessWidget {
  const PullRequestRefusal({
    super.key,
    required this.state,
    required this.device,
    required this.outcome,
    required this.compose,
    required this.beforePush,
    required this.composeTitle,
    required this.reading,
    required this.onCheckAgain,
  });

  final SessionState state;
  final String device;
  final BridgePullRequestOutcome outcome;

  /// "Commit, push & create PR" keeps its own title.
  final bool compose;
  final bool beforePush;
  final String composeTitle;

  /// GitHub is being read again, so Check again waits.
  final bool reading;
  final VoidCallback onCheckAgain;

  @override
  Widget build(BuildContext context) {
    final text = DangerDialog.text(context);
    final problem = pullRequestProblem(outcome, device: device)!;
    final uncommitted = state.git.status.value?.changed ?? 0;
    void next(PullRequestNext next) => Navigator.of(context).pop(next);
    final checkAgain = SecondaryButton(
      label: 'Check again',
      onPressed: reading ? null : onCheckAgain,
    );
    final (title, actions) = switch (outcome.kind) {
      BridgePullRequestOutcomeKind.needsPush => (
        'Push before creating a pull request',
        [
          PrimaryButton(
            label: 'Push…',
            onPressed: () => next(PullRequestNext.push),
          ),
          SecondaryButton(
            label: 'Commit, push & create PR…',
            onPressed: () => next(PullRequestNext.compose),
          ),
        ],
      ),
      BridgePullRequestOutcomeKind.noBranch => (
        'Detached HEAD',
        [
          PrimaryButton(
            label: 'Create branch…',
            onPressed: () => next(PullRequestNext.branch),
          ),
        ],
      ),
      BridgePullRequestOutcomeKind.noCommit => (
        'Nothing committed',
        [
          PrimaryButton(
            label: 'Commit changes…',
            onPressed: () => next(PullRequestNext.commit),
          ),
        ],
      ),
      BridgePullRequestOutcomeKind.behind => (
        'Pull before creating a pull request',
        <Widget>[],
      ),
      BridgePullRequestOutcomeKind.notGitHub when outcome.page.isNotEmpty => (
        'Create pull request',
        [
          PrimaryButton(
            label: 'Open in browser ↗',
            onPressed: () => unawaited(openInBrowser(state, outcome.page)),
          ),
          checkAgain,
        ],
      ),
      _ => ('Create pull request', [checkAgain]),
    };
    return ActionDialog(
      title: compose ? composeTitle : title,
      actions: actions,
      children: [
        Text(problem, style: text),
        if (beforePush) Text('Nothing was committed or pushed.', style: text),
        if (outcome.kind == BridgePullRequestOutcomeKind.needsPush &&
            uncommitted > 0)
          Text(
            '${plural(uncommitted, 'uncommitted file')} ${uncommitted == 1 ? 'is' : 'are'} not part of the pull request. Commit them first if you want to include them.',
            style: text,
          ),
      ],
    );
  }
}
