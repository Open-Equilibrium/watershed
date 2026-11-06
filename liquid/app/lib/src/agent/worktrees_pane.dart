import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/worktrees.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_menus.dart';

/// Settings › Worktrees until Chats exist (pull request and worktree mockups §10): Git's
/// registered worktrees of this window's repository on this device, and removing a clean one.
class WorktreesPane extends StatefulWidget {
  const WorktreesPane({super.key, required this.state, required this.device});

  final SessionState state;
  final String device;

  @override
  State<WorktreesPane> createState() => _WorktreesPaneState();
}

class _WorktreesPaneState extends State<WorktreesPane> {
  List<BridgeWorktree>? _worktrees;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_list());
  }

  Future<void> _list() async {
    try {
      final worktrees = await widget.state.worktrees();
      if (mounted) {
        setState(() {
          _worktrees = worktrees;
          _error = null;
        });
      }
    } on Object catch (error) {
      if (mounted) setState(() => _error = 'Could not list worktrees: $error');
    }
  }

  Future<void> _remove(BridgeWorktree worktree) async {
    BridgeWorktreeRemoval review;
    try {
      review = await widget.state.worktreeRemoval(worktree.path);
    } on Object catch (error) {
      widget.state.notify(
        BridgeLevel.error,
        'Could not check ${worktree.shown}: $error',
      );
      return;
    }
    if (!mounted) return;
    final removed = await showDialog<bool>(
      context: context,
      builder: (_) => _RemoveWorktree(
        state: widget.state,
        device: widget.device,
        review: review,
      ),
    );
    if (removed == true) await _list();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final repository = widget.state.git.status.value?.repository;
    final worktrees = _worktrees;
    Widget row(List<Widget> cells, {bool header = false}) => Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space2,
      ),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: ws.border)),
      ),
      child: Row(
        spacing: WsMetrics.space4,
        children: [
          Expanded(child: cells[0]),
          SizedBox(width: 150, child: cells[1]),
          SizedBox(width: 180, child: cells[2]),
        ],
      ),
    );
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space6,
        WsMetrics.space5,
        WsMetrics.space6,
        WsMetrics.space5,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space4,
        children: [
          Text(
            'Worktrees',
            style: Theme.of(context).textTheme.titleMedium
                ?.copyWith(fontWeight: WsMetrics.fwSemibold),
          ),
          Text(
            "Git's registered worktrees of ${repository == null ? 'this repository' : projectOf(repository)} on ${widget.device}, including ones created outside Liquid. Remove… opens the removal dialog: only clean worktrees, never forced; the branch stays.",
            style: faint,
          ),
          if (_error case final error?) WarningNotice(error),
          if (worktrees == null && _error == null)
            Text('Reading worktrees…', style: faint)
          else if (worktrees != null)
            Container(
              decoration: BoxDecoration(
                border: Border.all(color: ws.border),
                borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              ),
              child: Column(
                children: [
                  row([
                    Text('Folder', style: faint),
                    Text('Branch', style: faint),
                    const SizedBox.shrink(),
                  ]),
                  for (final worktree in worktrees)
                    row([
                      Text(
                        worktree.shown,
                        style: monoStyle(context),
                        overflow: TextOverflow.ellipsis,
                      ),
                      Text(
                        worktree.branch?.shown ?? 'detached',
                        style: monoStyle(context),
                        overflow: TextOverflow.ellipsis,
                      ),
                      switch (_kept(worktree)) {
                        final kept? => Text(kept, style: faint),
                        null => Align(
                          alignment: Alignment.centerLeft,
                          child: SecondaryButton(
                            label: 'Remove…',
                            onPressed: () => unawaited(_remove(worktree)),
                          ),
                        ),
                      },
                    ]),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// What keeps `worktree` from being offered for removal; none when it may be.
String? _kept(BridgeWorktree worktree) {
  final reasons = [
    if (worktree.main) 'main checkout',
    if (worktree.current) 'this window',
    if (worktree.running.isNotEmpty) 'agent running',
    if (worktree.locked) 'locked by Git',
    if (worktree.missing) 'folder missing',
  ];
  return reasons.isEmpty ? null : reasons.join(' · ');
}

/// The repository folder's name from its Git directory.
String projectOf(String gitDir) {
  final parts = gitDir.split('/').where((part) => part.isNotEmpty).toList();
  return parts.length > 1 && parts.last == '.git'
      ? parts[parts.length - 2]
      : parts.lastOrNull ?? gitDir;
}

/// The approved removal dialog: a clean worktree is removed after confirmation, one with local
/// or ignored files or a running agent is kept and says why. It closes with `true` once Git
/// removed it.
class _RemoveWorktree extends StatefulWidget {
  const _RemoveWorktree({
    required this.state,
    required this.device,
    required this.review,
  });

  final SessionState state;
  final String device;
  final BridgeWorktreeRemoval review;

  @override
  State<_RemoveWorktree> createState() => _RemoveWorktreeState();
}

class _RemoveWorktreeState extends State<_RemoveWorktree> {
  late var _review = widget.review;
  bool _removing = false;
  bool _changed = false;
  String? _error;

  Future<void> _remove() async {
    setState(() {
      _removing = true;
      _error = null;
    });
    BridgeWorktreeOutcome outcome;
    try {
      outcome = await widget.state.removeWorktree(_review);
    } on Object catch (error) {
      outcome = BridgeWorktreeOutcome(
        kind: BridgeWorktreeOutcomeKind.failed,
        running: const [],
        detail: '$error',
      );
    }
    if (!mounted) return;
    final worktree = _review.worktree;
    switch (outcome.kind) {
      case BridgeWorktreeOutcomeKind.done:
        widget.state.notify(
          BridgeLevel.info,
          'Removed the worktree at ${worktree.shown}${worktree.branch == null ? '' : '; the branch ${worktree.branch!.shown} stays'}.',
        );
        Navigator.of(context).pop(true);
      case BridgeWorktreeOutcomeKind.changed:
        setState(() {
          _review = outcome.removal!;
          _changed = true;
          _removing = false;
        });
      case BridgeWorktreeOutcomeKind.agentRunning:
        setState(() {
          _removing = false;
          _error =
              '${whoRuns(outcome.running)}. Stop it or wait until it finishes; nothing was removed.';
        });
      case BridgeWorktreeOutcomeKind.failed:
        setState(() {
          _removing = false;
          _error =
              'Could not remove this worktree: ${outcome.detail} It stays listed until Git removes it.';
        });
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = DangerDialog.text(context);
    final review = _review;
    final worktree = review.worktree;
    final files = review.files.length;
    final ignored = review.ignored.toInt();
    final kept = !review.removable;
    final changed = _changed
        ? WarningNotice(
            '${worktree.shown} changed since this dialog opened. Review it again before removing.',
          )
        : null;
    final remove = DangerButton(
      label: 'Remove worktree',
      onPressed: kept || _removing ? null : _remove,
    );
    if (kept) {
      return ActionDialog(
        title: 'Could not remove this worktree',
        dismiss: 'Close',
        actions: [
          remove,
          if (ignored > 0 || review.moreIgnored)
            SecondaryButton(
              label: 'Open folder',
              onPressed: () =>
                  unawaited(openInFileManager(widget.state, worktree.path)),
            ),
        ],
        children: [
          ?changed,
          if (worktree.running.isNotEmpty)
            Text(
              '${whoRuns(worktree.running).replaceFirst('in this checkout', 'in this worktree')}. Stop it or wait until it finishes.',
              style: text,
            ),
          if (files > 0) ...[
            Text(
              '${plural(files, 'file')}${review.moreFiles ? ' and more' : ''} ${files == 1 && !review.moreFiles ? 'has' : 'have'} local changes. Commit them or leave them in a stash through the approved branch flow before removing the worktree; a stash remains shared by this repository.',
              style: text,
            ),
            for (final file in review.files)
              Row(
                spacing: WsMetrics.space3,
                children: [
                  Expanded(
                    child: Text(
                      file.shown,
                      style: monoStyle(context),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  Text(switch (file.state) {
                    BridgeWorktreeFileState.staged => 'staged',
                    BridgeWorktreeFileState.unstaged => 'unstaged',
                    BridgeWorktreeFileState.untracked => 'untracked',
                  }, style: text),
                ],
              ),
          ],
          if (ignored > 0 || review.moreIgnored)
            Text(
              'Git ignores ${review.moreIgnored ? 'more than ' : ''}${plural(ignored, 'file')} in this folder. Checkpoints and a normal stash do not save them. Move or remove them yourself before removing the worktree.',
              style: text,
            ),
          if (_error case final error?) WarningNotice(error),
        ],
      );
    }
    final unpushed = review.unpushed.toInt();
    return ActionDialog(
      title: 'Remove ${worktree.branch?.shown ?? worktree.shown} worktree?',
      actions: [remove],
      children: [
        ?changed,
        Text(
          'Removes ${worktree.shown} on ${widget.device}. Git reports no staged, unstaged, untracked or ignored files to preserve.',
          style: text,
        ),
        Text(
          '${unpushed > 0 ? 'The branch has ${plural(unpushed, 'unpushed commit')}. ' : ''}The branch and its commits remain in this repository.',
          style: text,
        ),
        Text(
          'Sessions that worked in this folder keep their history and checkpoints, but cannot resume file work in the removed folder.',
          style: text,
        ),
        if (_error case final error?) WarningNotice(error),
      ],
    );
  }
}
