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

  /// A worktree is being reviewed for removal.
  bool _reviewing = false;

  /// The folder is outside Git, so it has no worktrees.
  bool get _outsideGit =>
      widget.state.git.loaded.value && widget.state.git.status.value == null;

  @override
  void initState() {
    super.initState();
    if (!_outsideGit) unawaited(_list());
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
    if (_reviewing) return;
    _reviewing = true;
    try {
      final BridgeWorktreeRemoval review;
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
      await showDialog<void>(
        context: context,
        builder: (_) => _RemoveWorktree(
          state: widget.state,
          device: widget.device,
          review: review,
        ),
      );
    } finally {
      _reviewing = false;
      // What Git lists may have changed, whatever the dialog did.
      if (mounted) await _list();
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final worktrees = _worktrees;
    // The main checkout comes first; its folder names the repository.
    final repository = worktrees?.firstOrNull?.shown
        .split('/')
        .lastWhere((part) => part.isNotEmpty, orElse: () => 'this repository');
    Widget row(List<Widget> cells) => Container(
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
          SizedBox(width: 140, child: cells[2]),
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
            "Git's registered worktrees of ${repository ?? 'this repository'} on ${widget.device}, including ones created outside Liquid. Remove… opens the removal dialog: only clean worktrees, never forced; the branch stays.",
            style: faint,
          ),
          if (_outsideGit)
            Text(
              'This folder has no Git repository, so it has no worktrees.',
              style: faint,
            )
          else if (_error case final error?)
            WarningNotice(error),
          if (_outsideGit)
            const SizedBox.shrink()
          else if (worktrees == null && _error == null)
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
                      Text(worktree.shown, style: monoStyle(context)),
                      Text(
                        worktree.branch?.shown ?? 'detached',
                        style: monoStyle(context),
                        overflow: TextOverflow.ellipsis,
                      ),
                      switch (_kept(worktree)) {
                        final kept? => Text(kept, style: faint),
                        null => Align(
                          alignment: Alignment.centerLeft,
                          child: Semantics(
                            label: 'Remove the worktree at ${worktree.shown}',
                            excludeSemantics: true,
                            button: true,
                            child: SecondaryButton(
                              label: 'Remove…',
                              onPressed: () => unawaited(_remove(worktree)),
                            ),
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
    if (worktree.unreadable) 'unreadable by Git',
  ];
  return reasons.isEmpty ? null : reasons.join(' · ');
}

/// The approved removal dialog: a clean worktree is removed after confirmation, one with local
/// or ignored files or a running agent is kept and says why.
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
    final worktree = _review.worktree;
    // A question closed the dialog meanwhile, so the status bar says what happened.
    final shown = mounted
        ? 'this worktree'
        : 'the worktree at ${worktree.shown}';
    final error = switch (outcome.kind) {
      BridgeWorktreeOutcomeKind.done => null,
      BridgeWorktreeOutcomeKind.changed when mounted => null,
      BridgeWorktreeOutcomeKind.changed =>
        '${worktree.shown} changed since this dialog opened. Nothing was removed.',
      BridgeWorktreeOutcomeKind.agentRunning =>
        outcome.running.isEmpty
            // Another window's Git action or revert holds this worktree.
            ? 'Another action is using this repository; try again when it ends. Nothing was removed.'
            : '${whoRuns(outcome.running, where: 'in this repository')}. Stop it or wait until it finishes; nothing was removed.',
      BridgeWorktreeOutcomeKind.failed =>
        'Could not remove $shown: “${outcome.detail}” It stays listed until Git removes it.',
    };
    if (outcome.kind == BridgeWorktreeOutcomeKind.done) {
      widget.state.notify(
        BridgeLevel.info,
        'Removed the worktree at ${worktree.shown}${worktree.branch == null ? '' : '; the branch ${worktree.branch!.shown} stays'}.',
      );
      if (mounted) Navigator.of(context).pop();
    } else if (!mounted) {
      widget.state.notify(
        outcome.kind == BridgeWorktreeOutcomeKind.changed
            ? BridgeLevel.warning
            : BridgeLevel.error,
        error!,
      );
    } else {
      setState(() {
        _removing = false;
        _error = error;
        if (outcome.kind == BridgeWorktreeOutcomeKind.changed) {
          _review = outcome.removal!;
          _changed = true;
        }
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = DangerDialog.text(context);
    final review = _review;
    final worktree = review.worktree;
    final marked = review.files
        .where((file) => file.state == BridgeWorktreeFileState.marked)
        .length;
    final files = review.files.length - marked;
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
        dismissible: !_removing,
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
          if (worktree.main)
            Text(
              'The main checkout cannot be removed as a linked worktree.',
              style: text,
            ),
          if (worktree.current)
            Text(
              'This window works in this worktree, so it cannot remove it.',
              style: text,
            ),
          if (worktree.locked)
            Text(
              'A worktree locked by Git stays locked; Liquid does not force removal.',
              style: text,
            ),
          if (worktree.missing)
            Text(
              'Its folder is missing, so there is nothing to remove; Git can prune its record.',
              style: text,
            ),
          if (worktree.unreadable)
            Text(
              'Git cannot read this worktree, so Liquid cannot check what removing it would lose.',
              style: text,
            ),
          if (worktree.running.isNotEmpty)
            Text(
              '${whoRuns(worktree.running, where: 'in this worktree')}. Stop it or wait until it finishes.',
              style: text,
            ),
          if (review.stranded > 0)
            Text(
              '${plural(review.stranded.toInt(), 'commit')} ${review.stranded == 1 ? 'is' : 'are'} on no branch in this detached worktree and would be lost with it. Create a branch for ${review.stranded == 1 ? 'it' : 'them'} first.',
              style: text,
            ),
          if (marked > 0 || review.moreMarked)
            Text(
              '${marked == 0 ? 'More files' : '${plural(marked, 'file')}${review.moreMarked ? ' and more' : ''}'} ${marked == 1 && !review.moreMarked ? 'is' : 'are'} marked so that Git does not check ${marked == 1 && !review.moreMarked ? 'it' : 'them'} for edits, which removing the folder would lose. Clear the marks with git update-index --no-assume-unchanged or --no-skip-worktree, then review again.',
              style: text,
            ),
          if (files > 0 || review.moreFiles)
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
                  BridgeWorktreeFileState.marked => 'hidden from Git',
                }, style: text),
              ],
            ),
          if (ignored > 0 || review.moreIgnored)
            Text(
              '${ignored == 0 ? 'Git ignores files' : 'Git ignores ${review.moreIgnored ? 'more than ' : ''}${plural(ignored, 'file')}'} in this folder. Checkpoints and a normal stash do not save them. Move or remove them yourself before removing the worktree.',
              style: text,
            ),
          if (_error case final error?) WarningNotice(error),
        ],
      );
    }
    final unpushed = review.unpushed.toInt();
    return ActionDialog(
      title: 'Remove ${worktree.branch?.shown ?? worktree.shown} worktree?',
      dismissible: !_removing,
      actions: [remove],
      children: [
        ?changed,
        Text(
          'Removes ${worktree.shown} on ${widget.device}. Git reports no staged, unstaged, untracked or ignored files to preserve.',
          style: text,
        ),
        WarningNotice(
          worktree.branch == null
              ? 'Its commits remain in this repository; other branches, tags or remotes hold them.'
              : '${unpushed > 0 ? 'The branch has ${plural(unpushed, 'unpushed commit')}. ' : ''}The branch and its commits remain in this repository.',
          info: true,
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
