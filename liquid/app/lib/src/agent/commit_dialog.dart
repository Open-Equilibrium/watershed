import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'commit_draft.dart';

/// The approved whole-file commit dialog; its message and selection remain a draft.
Future<void> showCommitDialog(BuildContext context, SessionState state) async {
  final generation = state.generation;
  await state.git.openCommitPlan((plan) async {
    if (!context.mounted || generation != state.generation) return;
    await showDialog<void>(
      context: context,
      builder: (_) => _CommitDialog(state: state, plan: plan),
    );
  });
}

class _CommitDialog extends StatefulWidget {
  const _CommitDialog({required this.state, required this.plan});
  final SessionState state;
  final BridgeCommitPlan plan;
  @override
  State<_CommitDialog> createState() => _CommitDialogState();
}

class _CommitDialogState extends State<_CommitDialog> {
  late final _draft = CommitDraft(widget.state, widget.plan)
    ..addListener(_rebuild);

  void _rebuild() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    _draft.dispose();
    super.dispose();
  }

  Future<void> _commit([String? branch]) async {
    if (!_draft.ready ||
        (branch == null && widget.state.git.status.value?.branch == null)) {
      return;
    }
    final result = await _draft.commit(branch: branch);
    if (!mounted || ModalRoute.of(context)?.isCurrent != true) return;
    if (result?.kind == BridgeGitOutcomeKind.done ||
        result?.kind == BridgeGitOutcomeKind.hookChanged) {
      Navigator.of(context).pop();
    }
  }

  Future<void> _newBranch() async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => _CommitBranch(
        branch: widget.state.git.status.value?.branch,
        count: _draft.selected.length,
      ),
    );
    if (mounted && name != null) await _commit(name);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final status = widget.state.git.status.value;
    final ready = _draft.ready;
    final enter = !_draft.messageFocus.hasFocus;
    return ActionDialog(
      title: 'Commit changes',
      bindings: {
        if (ready && status?.branch != null && enter)
          const Keys(LogicalKeyboardKey.enter).activator: () => _commit(),
      },
      actions: [
        PrimaryButton(
          label: 'Commit',
          shortcut: enter ? '⏎' : null,
          onPressed: ready && status?.branch != null ? () => _commit() : null,
        ),
        if (!_draft.agentRunning)
          SecondaryButton(
            label: 'Commit on new branch…',
            onPressed: ready ? _newBranch : null,
          ),
      ],
      children: commitDraftFields(
        context,
        _draft,
        trailing:
            status?.branch != null && status?.branch == status?.defaultBranch
            ? Text(
                'Default branch',
                style: TextStyle(
                  color: ws.warning,
                  fontSize: WsMetrics.fsSmall,
                ),
              )
            : null,
      ),
    );
  }
}

class _CommitBranch extends StatefulWidget {
  const _CommitBranch({required this.branch, required this.count});
  final String? branch;
  final int count;
  @override
  State<_CommitBranch> createState() => _CommitBranchState();
}

class _CommitBranchState extends State<_CommitBranch> {
  final _name = TextEditingController();
  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final name = _name.text.trim();
    final problem = branchNameProblem(name);
    final submit = name.isEmpty || problem != null
        ? null
        : () => Navigator.of(context).pop(name);
    return ActionDialog(
      title: 'Commit on a new branch',
      dismiss: 'Back',
      bindings: {const Keys(LogicalKeyboardKey.enter).activator: ?submit},
      actions: [
        PrimaryButton(
          label: 'Create branch and commit',
          shortcut: '⏎',
          onPressed: submit,
        ),
      ],
      children: [
        Text(
          'Creates the branch from the current commit on ${widget.branch ?? 'detached HEAD'}, switches to it and commits the ${widget.count} ${widget.count == 1 ? 'file' : 'files'} there. ${widget.branch ?? 'The current commit'} stays as it is.',
          style: DangerDialog.text(context),
        ),
        TextField(
          key: const Key('commit-branch-name'),
          controller: _name,
          autofocus: true,
          style: monoStyle(context),
          onSubmitted: (_) => submit?.call(),
          onChanged: (_) => setState(() {}),
          decoration: InputDecoration(
            labelText: 'Branch name',
            errorText: problem,
          ),
        ),
      ],
    );
  }
}
