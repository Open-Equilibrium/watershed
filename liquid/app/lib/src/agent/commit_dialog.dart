import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../shortcuts/keymap.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The approved whole-file commit dialog; its message and selection remain a draft.
Future<void> showCommitDialog(BuildContext context, SessionState state) async {
  await state.git.refresh();
  final BridgeCommitPlan plan;
  try {
    plan = await state.git.commitPlan();
  } on Object catch (error) {
    state.notify(
      BridgeLevel.error,
      'Could not read uncommitted changes: $error',
    );
    return;
  }
  if (!context.mounted) return;
  await showDialog<void>(
    context: context,
    builder: (_) => _CommitDialog(state: state, plan: plan),
  );
}

class _CommitDialog extends StatefulWidget {
  const _CommitDialog({required this.state, required this.plan});
  final SessionState state;
  final BridgeCommitPlan plan;
  @override
  State<_CommitDialog> createState() => _CommitDialogState();
}

class _CommitDialogState extends State<_CommitDialog> {
  final _message = TextEditingController();
  late final _selected = {
    for (final file in widget.plan.files) file.change.path,
  };
  late var _running = widget.plan.running;
  bool _editing = false;
  bool _saving = false;
  bool get _enabled =>
      !_saving &&
      _running.isEmpty &&
      _selected.isNotEmpty &&
      _message.text.trim().isNotEmpty;
  @override
  void dispose() {
    _message.dispose();
    super.dispose();
  }

  Future<void> _commit([String? branch]) async {
    if (!_enabled) return;
    setState(() => _saving = true);
    final result = await widget.state.git.commit(
      [
        for (final file in widget.plan.files)
          if (_selected.contains(file.change.path)) file.change.path,
      ],
      _message.text,
      branch: branch,
    );
    if (!mounted) return;
    if (result?.kind == BridgeGitOutcomeKind.done ||
        result?.kind == BridgeGitOutcomeKind.hookChanged) {
      Navigator.of(context).pop();
    } else {
      setState(() {
        _saving = false;
        if (result?.kind == BridgeGitOutcomeKind.agentRunning) {
          _running = result!.running;
        }
      });
    }
  }

  Future<void> _newBranch() async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => _CommitBranch(
        branch: widget.state.git.status.value?.branch,
        count: _selected.length,
      ),
    );
    if (mounted && name != null) await _commit(name);
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final status = widget.state.git.status.value;
    final files = widget.plan.files;
    final included = files.where(
      (file) => _selected.contains(file.change.path),
    );
    final added = included.fold(0, (sum, file) => sum + file.change.added);
    final removed = included.fold(0, (sum, file) => sum + file.change.removed);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return ActionDialog(
      title: 'Commit changes',
      bindings: {
        if (_enabled)
          const Keys(LogicalKeyboardKey.enter).activator: () => _commit(),
      },
      actions: [
        PrimaryButton(
          label: 'Commit',
          shortcut: '⏎',
          onPressed: _enabled ? () => _commit() : null,
        ),
        SecondaryButton(
          label: 'Commit on new branch…',
          onPressed: _enabled ? _newBranch : null,
        ),
      ],
      children: [
        Container(
          padding: const EdgeInsets.all(WsMetrics.space3),
          decoration: BoxDecoration(
            color: ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Padding(
                padding: const EdgeInsets.all(WsMetrics.space3),
                child: Row(
                  spacing: WsMetrics.space3,
                  children: [
                    Text('Branch', style: small),
                    Expanded(
                      child: Text(
                        status?.branch ?? status?.commit ?? 'no commit',
                        style: monoStyle(context),
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    if (status?.branch != null &&
                        status?.branch == status?.defaultBranch)
                      Text(
                        'Default branch',
                        style: small.copyWith(color: ws.warning),
                      ),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                child: Row(
                  children: [
                    if (_editing)
                      Checkbox(
                        key: const Key('commit-select-all'),
                        materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        visualDensity: VisualDensity.compact,
                        tristate: true,
                        value: _selected.isEmpty
                            ? false
                            : _selected.length == files.length
                            ? true
                            : null,
                        onChanged: _saving
                            ? null
                            : (_) => setState(() {
                                if (_selected.length == files.length) {
                                  _selected.clear();
                                } else {
                                  _selected.addAll(
                                    files.map((file) => file.change.path),
                                  );
                                }
                              }),
                      ),
                    Text('Files', style: small),
                    const SizedBox(width: WsMetrics.space2),
                    Text(
                      _selected.length == files.length
                          ? '${files.length}'
                          : '${_selected.length} of ${files.length}',
                      style: small,
                    ),
                    const SizedBox(width: WsMetrics.space2),
                    Text('· +$added', style: small.copyWith(color: ws.success)),
                    const SizedBox(width: WsMetrics.space2),
                    Text('−$removed', style: small.copyWith(color: ws.danger)),
                    const Spacer(),
                    TextButton(
                      onPressed: _saving
                          ? null
                          : () => setState(() => _editing = !_editing),
                      child: Text(_editing ? 'Done' : 'Edit'),
                    ),
                  ],
                ),
              ),
              ConstrainedBox(
                constraints: const BoxConstraints(maxHeight: 240),
                child: ColoredBox(
                  color: ws.canvas,
                  child: ListView(
                    shrinkWrap: true,
                    children: [for (final file in files) _file(context, file)],
                  ),
                ),
              ),
            ],
          ),
        ),
        for (final file in included.where((file) => file.stagedPartly))
          WarningNotice(
            'You staged part of ${file.change.shown} yourself. The commit takes the whole file as it is now.',
          ),
        if (_running.isNotEmpty)
          WarningNotice(
            '${whoRuns(_running)}. ${stopFirst(_running)}, then commit.',
          ),
        const Text('Commit message'),
        TextField(
          key: const Key('commit-message'),
          controller: _message,
          enabled: !_saving,
          minLines: 3,
          maxLines: 5,
          onChanged: (_) => setState(() {}),
          style: const TextStyle(fontSize: WsMetrics.fsCompact),
          decoration: InputDecoration(
            hintText: 'Describe the change: a short first line, details below',
            filled: true,
            fillColor: ws.canvas,
            contentPadding: const EdgeInsets.all(WsMetrics.space3),
            border: OutlineInputBorder(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              borderSide: BorderSide(color: ws.borderStrong),
            ),
          ),
        ),
        Text(
          'Checkpoints stay: each turn keeps showing its changes, and a later revert changes files, never this commit.',
          style: small,
        ),
      ],
    );
  }

  Widget _file(BuildContext context, BridgeCommitFile file) {
    final ws = context.ws;
    final change = file.change;
    final selected = _selected.contains(change.path);
    final turns = file.turns;
    final attribution = turns.isEmpty
        ? 'not this session'
        : '${turns.length == 1 ? 'turn' : 'turns'} ${turns.join(', ')}${file.edited ? ' and you' : ''}';
    final (letter, color) = switch (change.status) {
      BridgeChangeStatus.added => ('A', ws.success),
      BridgeChangeStatus.deleted => ('D', ws.danger),
      BridgeChangeStatus.modified => ('M', ws.warning),
    };
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      color: selected ? ws.textFaint : ws.textMuted,
    );
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space3,
        vertical: WsMetrics.space2,
      ),
      child: Row(
        spacing: WsMetrics.space2,
        children: [
          if (_editing)
            Checkbox(
              key: Key('commit-select-${change.path}'),
              materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              visualDensity: VisualDensity.compact,
              value: selected,
              onChanged: _saving
                  ? null
                  : (value) => setState(() {
                      if (value == true) {
                        _selected.add(change.path);
                      } else {
                        _selected.remove(change.path);
                      }
                    }),
            ),
          Text(
            letter,
            style: small.copyWith(color: selected ? color : ws.textFaint),
          ),
          Expanded(
            child: Text(
              change.shown,
              style: monoStyle(
                context,
                color: selected ? ws.text : ws.textFaint,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (!selected)
            Text('Excluded', style: small)
          else ...[
            if (!_editing) ...[
              Text(attribution, style: small),
              Text('·', style: small),
            ],
            Text('+${change.added}', style: small.copyWith(color: ws.success)),
            Text('−${change.removed}', style: small.copyWith(color: ws.danger)),
          ],
        ],
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
