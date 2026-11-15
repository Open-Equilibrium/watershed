import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'commit_dialog.dart';
import 'publish_dialog.dart';
import 'session_card_rows.dart';
import 'session_menus.dart';

/// T3 Code's GitActionsControl: a split button whose main part runs the quick action and whose
/// chevron opens the Git action options. While an action runs it shows the progress, and after it
/// the result for ten seconds.
class GitActionsControl extends StatelessWidget {
  const GitActionsControl({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<GitState>(
    valueListenable: state.git,
    builder: (context, git, _) => ListenableBuilder(
      listenable: Listenable.merge([git.status, git.running, git.result]),
      builder: (context, _) {
        final status = git.status.value;
        if (git.running.value case final running?) {
          return _Progress(running: running);
        }
        if (status != null) {
          if (!status.repository) return _InitializeRow(git: git);
          if (git.result.value case final result?) {
            return _Result(
              state: state,
              result: result,
              terms: terminologyFor(status),
            );
          }
        }
        return _SplitRow(state: state, git: git);
      },
    ),
  );
}

/// The folder has no repository: T3's "Initialize Git" row.
class _InitializeRow extends StatelessWidget {
  const _InitializeRow({required this.git});

  final GitState git;

  @override
  Widget build(BuildContext context) => CardRow(
    onTap: () => unawaited(git.initialize()),
    children: [
      Icon(LucideIcons.plus, size: 16, color: context.ws.textFaint),
      const Text('Initialize Git'),
    ],
  );
}

/// The running action: its phase, the latest hook line and the time since it started.
class _Progress extends StatelessWidget {
  const _Progress({required this.running});

  final GitRunning running;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      child: Row(
        spacing: 10,
        children: [
          const SizedBox.square(
            dimension: 14,
            child: CircularProgressIndicator(strokeWidth: 1.5),
          ),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  running.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    fontWeight: WsMetrics.fwMedium,
                    color: ws.text,
                  ),
                ),
                if (running.output case final output?)
                  Tooltip(
                    message: output,
                    child: Text(
                      output,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: ws.textMuted,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ),
              ],
            ),
          ),
          // Ticks every second, so the elapsed time moves while the action runs.
          StreamBuilder<int>(
            stream: Stream.periodic(const Duration(seconds: 1), (tick) => tick),
            builder: (context, _) => Text(
              elapsed(DateTime.now().difference(running.since)),
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// A finished action's title and description, with the pull request it opened or pushed.
class _Result extends StatelessWidget {
  const _Result({
    required this.state,
    required this.result,
    required this.terms,
  });

  final SessionState state;
  final GitResult result;
  final Terminology terms;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      child: Row(
        spacing: 10,
        children: [
          Icon(LucideIcons.check, size: 14, color: ws.success),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  result.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: WsMetrics.fsCompact,
                    fontWeight: WsMetrics.fwMedium,
                    color: ws.text,
                  ),
                ),
                if (result.description case final description?)
                  Text(
                    description,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(
                      color: ws.textMuted,
                      fontSize: WsMetrics.fsSmall,
                    ),
                  ),
              ],
            ),
          ),
          if (result.url case final url?)
            SecondaryButton(
              label: 'Open ${terms.short}',
              onPressed: () => unawaited(openInBrowser(state, url)),
            ),
        ],
      ),
    );
  }
}

/// The idle split button: the quick action, and the Git action options behind the chevron.
class _SplitRow extends StatelessWidget {
  const _SplitRow({required this.state, required this.git});

  final SessionState state;
  final GitState git;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final status = git.status.value;
    final quick = git.quickAction();
    final action = quick.action;
    final VoidCallback? onPressed = switch (quick.kind) {
      QuickKind.runAction =>
        action == null ? null : () => unawaited(_run(context, state, action)),
      QuickKind.runPull => () => unawaited(git.pull()),
      QuickKind.openPublish => () => unawaited(
        showPublishDialog(context, state),
      ),
      QuickKind.showHint => null,
    };
    final row = CardRow(
      onTap: onPressed,
      children: [
        Icon(_quickIcon(quick), size: 16, color: ws.textFaint),
        Flexible(
          child: Text(
            quick.label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
        ),
      ],
    );
    final entries = status == null
        ? const <_Entry>[]
        : _menu(context, state, status);
    return Row(
      children: [
        Expanded(
          child: onPressed == null
              ? Tooltip(
                  message: quick.hint ?? quick.label,
                  child: Opacity(opacity: 0.45, child: row),
                )
              : row,
        ),
        SizedBox(
          width: 32,
          child: PopupMenuButton<VoidCallback>(
            tooltip: 'Git action options',
            enabled: entries.isNotEmpty,
            padding: EdgeInsets.zero,
            icon: Icon(LucideIcons.chevronDown, size: 16, color: ws.textFaint),
            onSelected: (run) => run(),
            itemBuilder: (_) => [
              for (final entry in entries)
                PopupMenuItem<VoidCallback>(
                  value: entry.run,
                  enabled: entry.reason == null,
                  height: 32,
                  child: switch (entry.reason) {
                    final reason? => Tooltip(
                      message: reason,
                      child: _EntryLabel(entry),
                    ),
                    null => _EntryLabel(entry),
                  },
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// One item of the Git action options: its label and icon, why it is disabled, and what it runs.
class _Entry {
  const _Entry(this.label, this.icon, this.run, {this.reason});

  final String label;
  final IconData icon;
  final VoidCallback run;
  final String? reason;
}

class _EntryLabel extends StatelessWidget {
  const _EntryLabel(this.entry);

  final _Entry entry;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      SizedBox(width: 14, child: Icon(entry.icon, size: 14)),
      const SizedBox(width: WsMetrics.space3),
      Text(entry.label),
    ],
  );
}

/// T3's menu items with their disabled reasons, then Publish repository when the checkout has no
/// remote.
List<_Entry> _menu(
  BuildContext context,
  SessionState state,
  BridgeGitStatus status,
) {
  final terms = terminologyFor(status);
  return [
    for (final (item, reason) in state.git.value.menuItems())
      switch (item) {
        MenuItem.commit => _Entry(
          'Commit',
          LucideIcons.gitCommit,
          () => unawaited(showCommitDialog(context, state)),
          reason: reason,
        ),
        MenuItem.push => _Entry(
          'Push',
          LucideIcons.arrowUp,
          () => unawaited(_run(context, state, BridgeGitAction.push)),
          reason: reason,
        ),
        MenuItem.createPullRequest => _Entry(
          'Create ${terms.short}',
          LucideIcons.gitPullRequest,
          () => unawaited(
            _run(context, state, BridgeGitAction.createPullRequest),
          ),
          reason: reason,
        ),
      },
    if (!status.remote)
      _Entry(
        'Publish repository...',
        LucideIcons.arrowUp,
        () => unawaited(showPublishDialog(context, state)),
      ),
  ];
}

IconData _quickIcon(QuickAction quick) => switch (quick.kind) {
  QuickKind.runPull => LucideIcons.arrowDown,
  QuickKind.runAction => switch (quick.action) {
    BridgeGitAction.commit => LucideIcons.gitCommit,
    BridgeGitAction.push || BridgeGitAction.commitPush => LucideIcons.arrowUp,
    _ => LucideIcons.gitPullRequest,
  },
  QuickKind.openPublish => LucideIcons.arrowUp,
  QuickKind.showHint => LucideIcons.info,
};

/// Runs `action`. On the default branch T3 asks first and offers a feature branch as the other
/// way on; the checkout's commit makes the feature branch's name.
Future<void> _run(
  BuildContext context,
  SessionState state,
  BridgeGitAction action,
) async {
  final git = state.git.value;
  final status = git.status.value;
  final branch = status?.branch;
  if (status == null ||
      branch == null ||
      !status.defaultBranch ||
      action == BridgeGitAction.commit) {
    unawaited(git.run(action));
    return;
  }
  final includesCommit =
      (action == BridgeGitAction.commitPush ||
          action == BridgeGitAction.commitPushPullRequest) &&
      status.changes.isNotEmpty;
  final copy = defaultBranchCopy(
    action,
    branch,
    includesCommit: includesCommit,
    terms: terminologyFor(status),
  );
  final unavailable = featureBranchUnavailable(
    action,
    branch,
    changes: status.changes.isNotEmpty,
  );
  // true checks out a feature branch first; false continues on the default branch.
  final featureBranch = await showDialog<bool>(
    context: context,
    builder: (context) => ActionDialog(
      title: copy.title,
      dismiss: 'Abort',
      actions: [
        PrimaryButton(
          label: 'Check out feature branch & continue',
          onPressed: unavailable == null
              ? () => Navigator.of(context).pop(true)
              : null,
        ),
        SecondaryButton(
          label: copy.continueLabel,
          onPressed: () => Navigator.of(context).pop(false),
        ),
      ],
      children: [
        Text(copy.description, style: DangerDialog.text(context)),
        if (unavailable case final reason?)
          Text(reason, style: DangerDialog.text(context)),
      ],
    ),
  );
  if (featureBranch != null) {
    unawaited(git.run(action, featureBranch: featureBranch));
  }
}
