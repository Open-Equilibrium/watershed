import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../rust/api/history.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The approved commit dialog's whole files, selection and message, kept current as the
/// checkout and turns change; Commit changes and "Commit, push & create PR" share it.
class CommitDraft extends ChangeNotifier {
  /// Starts from `plan`, or reads one.
  CommitDraft(this.state, [BridgeCommitPlan? plan]) {
    if (plan == null) {
      unawaited(refresh());
    } else {
      _accept(plan);
    }
    message.addListener(notifyListeners);
    messageFocus.addListener(notifyListeners);
    state.git.refreshed.addListener(_reread);
    state.status.addListener(_turnChanged);
  }

  final SessionState state;
  final message = TextEditingController();
  final messageFocus = FocusNode();
  final selected = <String>{};

  /// The latest plan; none before the first read.
  BridgeCommitPlan? plan;

  /// The last read failed.
  bool failed = false;
  bool editing = false;

  /// A commit runs; the plan is not read meanwhile.
  bool busy = false;
  List<BridgeRunningAgent> running = const [];
  Set<String> _editStart = const {};
  int _reads = 0;
  bool _disposed = false;

  bool get agentRunning =>
      running.isNotEmpty ||
      state.status.value == RunStatus.running ||
      state.status.value == RunStatus.waitingForYou;

  bool get ready =>
      plan != null &&
      !busy &&
      !agentRunning &&
      state.git.status.value?.merging != true &&
      selected.isNotEmpty &&
      message.text.trim().isNotEmpty;

  List<String> get paths => [
    for (final file in plan?.files ?? const <BridgeCommitFile>[])
      if (selected.contains(file.change.path)) file.change.path,
  ];

  void _accept(BridgeCommitPlan next) {
    if (plan == null) {
      selected.addAll(next.files.map((file) => file.change.path));
    } else {
      selected.retainAll(next.files.map((file) => file.change.path));
    }
    plan = next;
    running = next.running;
    failed = false;
  }

  void _reread() => unawaited(refresh());

  void _turnChanged() {
    if (state.status.value != RunStatus.running &&
        state.status.value != RunStatus.waitingForYou) {
      unawaited(refresh());
    } else {
      notifyListeners();
    }
  }

  /// Reads the files again, keeping the selection of those still changed.
  Future<void> refresh() async {
    if (busy) return;
    final read = ++_reads;
    try {
      final next = await state.git.commitPlan();
      if (_disposed || read != _reads) return;
      _accept(next);
    } on Object catch (error) {
      if (_disposed || read != _reads) return;
      failed = plan == null;
      state.notify(
        BridgeLevel.error,
        'Could not read uncommitted changes: $error',
      );
    }
    notifyListeners();
  }

  /// Switches between Edit and Done; returns whether editing ended with another selection.
  bool toggleEditing() {
    final changed = editing && !setEquals(_editStart, selected);
    if (!editing) _editStart = {...selected};
    editing = !editing;
    notifyListeners();
    return changed;
  }

  void select(Set<String> next) {
    selected
      ..clear()
      ..addAll(next);
    notifyListeners();
  }

  /// Commits the selected files as the plan's snapshot holds them, on a new `branch` if
  /// given.
  Future<BridgeGitOutcome?> commit({String? branch}) async {
    busy = true;
    notifyListeners();
    final outcome = await state.git.commit(
      paths,
      message.text,
      snapshot: plan!.snapshot,
      branch: branch,
    );
    if (_disposed) return outcome;
    busy = false;
    if (outcome?.kind == BridgeGitOutcomeKind.agentRunning) {
      running = outcome!.running;
    }
    notifyListeners();
    // The reads the commit's own refresh asked for waited; a retry needs the files as they
    // are now.
    if (outcome?.kind != BridgeGitOutcomeKind.done &&
        outcome?.kind != BridgeGitOutcomeKind.hookChanged) {
      unawaited(refresh());
    }
    return outcome;
  }

  @override
  void dispose() {
    _disposed = true;
    state.git.refreshed.removeListener(_reread);
    state.status.removeListener(_turnChanged);
    message.dispose();
    messageFocus.dispose();
    super.dispose();
  }
}

/// The approved commit dialog's fields for `draft`: the branch with `trailing`, the files,
/// their notices, the message and the checkpoint note. `onEdited` runs when editing the
/// selection ends with a change.
List<Widget> commitDraftFields(
  BuildContext context,
  CommitDraft draft, {
  Widget? trailing,
  VoidCallback? onEdited,
  bool enabled = true,
}) {
  final ws = context.ws;
  final status = draft.state.git.status.value;
  final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
  final plan = draft.plan;
  final editable = enabled && !draft.busy;
  final included = (plan?.files ?? const <BridgeCommitFile>[]).where(
    (file) => draft.selected.contains(file.change.path),
  );
  return [
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
                ?trailing,
              ],
            ),
          ),
          if (plan != null)
            CommitFileList(
              files: plan.files,
              selected: draft.selected,
              editing: draft.editing,
              enabled: editable,
              onEditing: () {
                if (draft.toggleEditing()) onEdited?.call();
              },
              onSelected: draft.select,
            )
          else if (!draft.failed)
            Text('Reading uncommitted changes…', style: small),
        ],
      ),
    ),
    if (draft.failed)
      const WarningNotice(
        'Could not read the uncommitted files, so nothing can be committed. Close and open this dialog again.',
      ),
    for (final file in included.where((file) => file.stagedPartly))
      WarningNotice(
        'You staged part of ${file.change.shown} yourself. The commit takes the whole file as it is now.',
      ),
    if (draft.running.isNotEmpty)
      WarningNotice(
        '${whoRuns(draft.running)}. ${stopFirst(draft.running)}, then commit.',
      ),
    if (status?.branch == null) const WarningNotice(detachedCommitHint),
    if (status?.merging == true) WarningNotice(mergeHint(status)),
    const Text('Commit message'),
    TextField(
      key: const Key('commit-message'),
      controller: draft.message,
      focusNode: draft.messageFocus,
      textInputAction: TextInputAction.newline,
      enabled: editable,
      minLines: 3,
      maxLines: 5,
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
  ];
}

/// The approved commit dialog's whole files with their attribution and line counts; Edit
/// shows checkboxes to leave files out.
class CommitFileList extends StatelessWidget {
  const CommitFileList({
    super.key,
    required this.files,
    required this.selected,
    required this.editing,
    required this.enabled,
    required this.onEditing,
    required this.onSelected,
  });

  final List<BridgeCommitFile> files;
  final Set<String> selected;
  final bool editing;
  final bool enabled;

  /// Switches between Edit and Done.
  final VoidCallback onEditing;

  /// Receives the new selection.
  final ValueChanged<Set<String>> onSelected;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final included = files.where((file) => selected.contains(file.change.path));
    final added = included.fold(0, (sum, file) => sum + file.change.added);
    final removed = included.fold(0, (sum, file) => sum + file.change.removed);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          child: Row(
            children: [
              if (editing)
                Checkbox(
                  key: const Key('commit-select-all'),
                  semanticLabel: 'All files',
                  materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  visualDensity: VisualDensity.compact,
                  tristate: true,
                  value: selected.isEmpty
                      ? false
                      : selected.length == files.length
                      ? true
                      : null,
                  onChanged: enabled
                      ? (_) => onSelected(
                          selected.length == files.length
                              ? {}
                              : {for (final file in files) file.change.path},
                        )
                      : null,
                ),
              Text('Files', style: small),
              const SizedBox(width: WsMetrics.space2),
              Text(
                selected.length == files.length
                    ? '${files.length}'
                    : '${selected.length} of ${files.length}',
                style: small,
              ),
              const SizedBox(width: WsMetrics.space2),
              Text('· +$added', style: small.copyWith(color: ws.success)),
              const SizedBox(width: WsMetrics.space2),
              Text('−$removed', style: small.copyWith(color: ws.danger)),
              const Spacer(),
              TextButton(
                onPressed: enabled ? onEditing : null,
                child: Text(editing ? 'Done' : 'Edit'),
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
    );
  }

  Widget _file(BuildContext context, BridgeCommitFile file) {
    final ws = context.ws;
    final change = file.change;
    final included = selected.contains(change.path);
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
      color: included ? ws.textFaint : ws.textMuted,
    );
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space3,
        vertical: WsMetrics.space2,
      ),
      child: Row(
        spacing: WsMetrics.space2,
        children: [
          if (editing)
            Checkbox(
              key: Key('commit-select-${change.path}'),
              semanticLabel: change.shown,
              materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              visualDensity: VisualDensity.compact,
              value: included,
              onChanged: enabled
                  ? (value) => onSelected(
                      value == true
                          ? {...selected, change.path}
                          : ({...selected}..remove(change.path)),
                    )
                  : null,
            ),
          Text(
            letter,
            style: small.copyWith(color: included ? color : ws.textFaint),
          ),
          Expanded(
            flex: 2,
            child: Text(
              change.shown,
              style: monoStyle(
                context,
                color: included ? ws.text : ws.textFaint,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (!included)
            Text('Excluded', style: small)
          else ...[
            if (!editing && file.attributionAvailable) ...[
              Flexible(
                child: Tooltip(
                  message: attribution,
                  child: Text(
                    attribution,
                    style: small,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
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
