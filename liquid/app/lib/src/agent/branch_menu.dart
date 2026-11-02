import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import 'commit_dialog.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The checkout's branch with its commits ahead and behind its upstream, in the session header;
/// it opens the branch menu (approved Git mockups).
class BranchChip extends StatelessWidget {
  const BranchChip({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) {
    final git = state.git;
    return ListenableBuilder(
      listenable: Listenable.merge([git.status, git.problem]),
      builder: (context, _) {
        final ws = context.ws;
        final small = TextStyle(
          color: ws.textFaint,
          fontSize: WsMetrics.fsSmall,
          fontFeatures: const [FontFeature.tabularFigures()],
        );
        if (git.problem.value case final problem?) {
          return Tooltip(
            message: problem,
            child: _Frame(
              children: [
                Icon(LucideIcons.triangleAlert, size: 13, color: ws.danger),
                Text(
                  'Git unavailable',
                  style: TextStyle(
                    color: ws.danger,
                    fontSize: WsMetrics.fsCompact,
                  ),
                ),
              ],
            ),
          );
        }
        final status = git.status.value;
        if (status == null) return const SizedBox.shrink();
        final name = status.branch ?? status.commit ?? 'no commit';
        return Semantics(
          key: const Key('branch-chip'),
          button: true,
          onTap: () => unawaited(showBranchMenu(context, state)),
          label: [
            'Branch $name',
            if (status.ahead > 0) '${status.ahead} ahead',
            if (status.behind > 0) '${status.behind} behind',
            if (!status.merging &&
                status.branch != null &&
                status.upstream == null)
              'no upstream',
          ].join(', '),
          excludeSemantics: true,
          child: InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            onTap: () => unawaited(showBranchMenu(context, state)),
            child: _Frame(
              children: [
                Icon(LucideIcons.gitBranch, size: 14, color: ws.textMuted),
                Flexible(
                  child: Text(
                    name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: monoStyle(context, color: ws.text),
                  ),
                ),
                if (status.merging) Text('merging', style: small),
                if (status.ahead > 0) Text('↑${status.ahead}', style: small),
                if (status.behind > 0)
                  Text(
                    '↓${status.behind}',
                    style: small.copyWith(color: ws.warning),
                  ),
                if (!status.merging &&
                    status.branch != null &&
                    status.upstream == null)
                  Text('no upstream', style: small),
                Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _Frame extends StatelessWidget {
  const _Frame({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Container(
    height: 28,
    padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
    decoration: BoxDecoration(
      border: Border.all(color: context.ws.borderStrong),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      spacing: WsMetrics.space2,
      children: children,
    ),
  );
}

/// What the branch menu was closed with.
sealed class _Chosen {}

class _Switch extends _Chosen {
  _Switch(this.branch);
  final BridgeBranch branch;
}

class _Create extends _Chosen {
  _Create(this.name);
  final String name;
}

/// Opens the branch menu, then switches to or creates the branch chosen, asking first where
/// uncommitted changes go, as GitHub Desktop does.
Future<void> showBranchMenu(BuildContext context, SessionState state) async {
  final git = state.git;
  // Fresh, so a change made since the last read is asked about.
  await git.refresh();
  final BridgeBranches branches;
  try {
    branches = await git.branches();
  } on Object catch (error) {
    state.notify(BridgeLevel.error, 'Could not list the branches: $error');
    return;
  }
  if (!context.mounted) return;
  final chosen = await showDialog<_Chosen>(
    context: context,
    barrierColor: Colors.transparent,
    builder: (context) =>
        _BranchMenu(branches: branches, status: git.status.value),
  );
  if (!context.mounted) return;
  switch (chosen) {
    case _Create(:final name):
      await git.create(name);
    case _Switch(:final branch):
      await _switch(context, state, branch, branches);
    case null:
  }
}

Future<void> _switch(
  BuildContext context,
  SessionState state,
  BridgeBranch branch,
  BridgeBranches branches,
) async {
  final git = state.git;
  final left = git.status.value?.branch;
  final changed = git.status.value?.changed ?? 0;
  final stashed = git.stash.value;
  final from = branches.branches.where((branch) => branch.current).firstOrNull;
  Future<BridgeGitOutcome?> switchTo(bool leave) async {
    if (leave &&
        (left != from?.shown ||
            left != git.status.value?.branch ||
            stashed?.id != git.stash.value?.id)) {
      state.notify(
        BridgeLevel.warning,
        'The checkout branch or stash changed. Choose the branch again.',
      );
      return null;
    }
    return git.switchTo(
      branch,
      leaveChanges: leave,
      fromBranch: from?.name,
      replacingStash: stashed?.id,
    );
  }

  var leave = false;
  // On no branch the changes can only come along.
  if (changed > 0 && left != null) {
    final choice = await showDialog<bool>(
      context: context,
      builder: (context) =>
          _SwitchDialog(left: left, target: branch.shown, stashed: stashed),
    );
    if (choice == null || !context.mounted) return;
    leave = choice;
  }
  var outcome = await switchTo(leave);
  while (outcome?.kind == BridgeGitOutcomeKind.wouldOverwrite) {
    if (!context.mounted) return;
    final again = await showDialog<String>(
      context: context,
      builder: (context) => _RefusedDialog(
        target: branch.shown,
        files: outcome!.files,
        left: left,
        stashed: stashed,
      ),
    );
    if (!context.mounted) return;
    if (again == 'commit') {
      await showCommitDialog(context, state);
      return;
    }
    if (again != 'leave') return;
    outcome = await switchTo(true);
  }
}

/// The searchable list of local and remote branches, with "Create branch" for a new name
/// (approved Git mockups).
class _BranchMenu extends StatefulWidget {
  const _BranchMenu({required this.branches, required this.status});

  final BridgeBranches branches;
  final BridgeGitStatus? status;

  int get changed => status?.changed ?? 0;
  String? get current => status?.branch;

  @override
  State<_BranchMenu> createState() => _BranchMenuState();
}

class _BranchMenuState extends State<_BranchMenu> {
  final _query = TextEditingController();

  @override
  void initState() {
    super.initState();
    _query.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final query = _query.text.trim();
    final running = widget.branches.running;
    final refused = running.isNotEmpty;
    final matches = [
      for (final branch in widget.branches.branches)
        if (branch.shown.toLowerCase().contains(query.toLowerCase())) branch,
    ];
    final exists = widget.branches.branches.any(
      (branch) => !branch.remote && branch.name == query,
    );
    final creatable = query.isNotEmpty && !exists;
    final problem = creatable ? branchNameProblem(query) : null;
    final now = DateTime.now();
    void choose(BridgeBranch branch) =>
        Navigator.of(context).pop(branch.current ? null : _Switch(branch));
    void create() => Navigator.of(context).pop(_Create(query));
    void submit() {
      if (refused) return;
      if (matches.where((branch) => !branch.current).firstOrNull
          case final first?) {
        choose(first);
      } else if (creatable && problem == null) {
        create();
      }
    }

    Widget label(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(
        WsMetrics.space3,
        WsMetrics.space2,
        WsMetrics.space3,
        WsMetrics.space1,
      ),
      child: Text(text, style: small),
    );
    Widget row({
      required Widget leading,
      required Widget name,
      String? detail,
      VoidCallback? onTap,
    }) => InkWell(
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      onTap: onTap,
      child: SizedBox(
        height: 28,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          child: Row(
            spacing: WsMetrics.space3,
            children: [
              SizedBox(width: 14, child: leading),
              Expanded(
                child: DefaultTextStyle.merge(
                  style: monoStyle(
                    context,
                    color: onTap == null ? ws.textFaint : ws.text,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  child: name,
                ),
              ),
              if (detail != null)
                Flexible(
                  child: Text(
                    detail,
                    style: small,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
            ],
          ),
        ),
      ),
    );
    Widget branchRow(BridgeBranch branch) => row(
      leading: branch.current
          ? Icon(LucideIcons.check, size: 14, color: ws.accent)
          : const SizedBox.shrink(),
      name: Text(branch.shown),
      detail: [
        if (branch.stashed) 'stashed changes',
        if (branch.current) ...[
          if ((widget.status?.ahead ?? 0) > 0) '↑${widget.status!.ahead}',
          if ((widget.status?.behind ?? 0) > 0) '↓${widget.status!.behind}',
          'now',
        ] else
          formatAgo(branch.committedMs, now),
      ].join(' · '),
      onTap: refused ? null : () => choose(branch),
    );
    final local = [
      for (final branch in matches)
        if (!branch.remote) branch,
    ];
    final remote = [
      for (final branch in matches)
        if (branch.remote) branch,
    ];
    final changes = widget.changed == 1
        ? '1 uncommitted change'
        : '${widget.changed} uncommitted changes';
    return Stack(
      children: [
        Positioned(
          top: 76,
          right: WsMetrics.space7,
          width: 360,
          child: Material(
            color: ws.surface,
            elevation: 8,
            shadowColor: Colors.black54,
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            child: CallbackShortcuts(
              bindings: {
                const Keys(LogicalKeyboardKey.enter).activator: submit,
              },
              child: Padding(
                padding: const EdgeInsets.all(WsMetrics.space2),
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxHeight: 420),
                  child: ListView(
                    shrinkWrap: true,
                    children: [
                      TextField(
                        key: const Key('branch-search'),
                        controller: _query,
                        autofocus: true,
                        style: TextStyle(
                          fontSize: WsMetrics.fsCompact,
                          color: ws.text,
                        ),
                        decoration: const InputDecoration(
                          isDense: true,
                          hintText: 'Search or create a branch',
                        ),
                      ),
                      const SizedBox(height: WsMetrics.space1),
                      if (refused)
                        Padding(
                          padding: const EdgeInsets.symmetric(
                            vertical: WsMetrics.space1,
                          ),
                          child: WarningNotice(
                            '${whoRuns(running)}. ${stopFirst(running)}, then switch branches.',
                          ),
                        ),
                      if (local.isNotEmpty) ...[
                        label('Local branches'),
                        for (final branch in local) branchRow(branch),
                      ],
                      if (remote.isNotEmpty) ...[
                        label('Remote branches'),
                        for (final branch in remote) branchRow(branch),
                      ],
                      if (creatable) ...[
                        if (matches.isEmpty) label('No branch matches'),
                        row(
                          leading: Icon(
                            LucideIcons.gitBranch,
                            size: 14,
                            color: ws.textMuted,
                          ),
                          name: Text.rich(
                            TextSpan(
                              children: [
                                TextSpan(
                                  text: 'Create branch ',
                                  style: DefaultTextStyle.of(context).style,
                                ),
                                TextSpan(text: query),
                              ],
                            ),
                          ),
                          detail: problem == null && matches.isEmpty
                              ? '⏎'
                              : null,
                          onTap: refused || problem != null ? null : create,
                        ),
                        _Foot(
                          problem == null
                              ? [
                                  'Starts from the current commit',
                                  if (widget.current case final current?)
                                    ' on $current',
                                  if (widget.changed > 0)
                                    '; your $changes come along.'
                                  else
                                    '.',
                                ].join()
                              : '⚠ $problem',
                          color: problem == null ? null : ws.danger,
                        ),
                      ] else if (widget.changed > 0 && !refused) ...[
                        const Divider(height: WsMetrics.space3),
                        _Foot(
                          'You have $changes; switching asks where they go.',
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

class _Foot extends StatelessWidget {
  const _Foot(this.text, {this.color});

  final String text;
  final Color? color;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(
      WsMetrics.space3,
      WsMetrics.space1,
      WsMetrics.space3,
      WsMetrics.space1,
    ),
    child: Text(
      text,
      style: TextStyle(
        color: color ?? context.ws.textFaint,
        fontSize: WsMetrics.fsSmall,
      ),
    ),
  );
}

/// Asks where uncommitted changes go when switching, with GitHub Desktop's words; closes with
/// `true` to leave them in the stash of the branch left, `false` to bring them along.
class _SwitchDialog extends StatefulWidget {
  const _SwitchDialog({
    required this.left,
    required this.target,
    required this.stashed,
  });

  final String left;
  final String target;

  /// The stashed changes of the branch left, which leaving replaces.
  final BridgeStash? stashed;

  @override
  State<_SwitchDialog> createState() => _SwitchDialogState();
}

class _SwitchDialogState extends State<_SwitchDialog> {
  bool _leave = true;

  @override
  Widget build(BuildContext context) {
    void confirmed() => Navigator.of(context).pop(_leave);
    Widget choice(bool leave, String title, String detail) => _Choice(
      selected: _leave == leave,
      title: title,
      detail: detail,
      onTap: () => setState(() => _leave = leave),
    );
    return ActionDialog(
      title: 'Switch branch',
      bindings: {
        const Keys(LogicalKeyboardKey.enter).activator: confirmed,
        const Keys(LogicalKeyboardKey.arrowUp).activator: () =>
            setState(() => _leave = true),
        const Keys(LogicalKeyboardKey.arrowDown).activator: () =>
            setState(() => _leave = false),
      },
      actions: [
        PrimaryButton(
          label: _leave && widget.stashed != null
              ? 'Replace stash and switch'
              : 'Switch branch',
          shortcut: '⏎',
          onPressed: confirmed,
        ),
      ],
      children: [
        Text(
          'You have changes on this branch. What would you like to do with them?',
          style: DangerDialog.text(context),
        ),
        choice(
          true,
          'Leave my changes on ${widget.left}',
          'Your in-progress work will be stashed on this branch for you to return to later',
        ),
        if (widget.stashed case final stashed? when _leave)
          WarningNotice(
            '⚠ ${widget.left} already has stashed changes from '
            '${formatClockMs(stashed.createdMs)} (${stashed.files.length == 1 ? '1 file' : '${stashed.files.length} files'}). '
            'Leaving these changes replaces them.',
          ),
        choice(
          false,
          'Bring my changes to ${widget.target}',
          'Your in-progress work will follow you to the new branch',
        ),
      ],
    );
  }
}

class _Choice extends StatelessWidget {
  const _Choice({
    required this.selected,
    required this.title,
    required this.detail,
    required this.onTap,
  });

  final bool selected;
  final String title;
  final String detail;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      inMutuallyExclusiveGroup: true,
      checked: selected,
      child: InkWell(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        onTap: onTap,
        child: Container(
          padding: const EdgeInsets.symmetric(
            horizontal: WsMetrics.space4,
            vertical: WsMetrics.space3,
          ),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            border: Border.all(
              color: selected ? ws.accent : ws.borderStrong,
              width: selected ? 2 : 1,
            ),
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: WsMetrics.space4,
            children: [
              Container(
                width: 14,
                height: 14,
                margin: const EdgeInsets.only(top: 3),
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  border: Border.all(
                    color: selected ? ws.accent : ws.borderStrong,
                    width: selected ? 4 : 1.5,
                  ),
                ),
              ),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      title,
                      style: TextStyle(
                        color: ws.text,
                        fontWeight: WsMetrics.fwMedium,
                        fontSize: WsMetrics.fsCompact,
                      ),
                    ),
                    Text(detail, style: DangerDialog.text(context)),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Says that Git refused to bring the changes along and offers to leave them instead; closes
/// with `true` to switch leaving them.
class _RefusedDialog extends StatelessWidget {
  const _RefusedDialog({
    required this.target,
    required this.files,
    required this.left,
    required this.stashed,
  });

  final String target;
  final List<String> files;
  final String? left;
  final BridgeStash? stashed;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final left = this.left;
    void leave() => Navigator.of(context).pop('leave');
    return ActionDialog(
      title: 'Could not switch to $target',
      // Nothing changed, so it closes rather than cancels.
      dismiss: 'Close',
      bindings: {
        if (left != null) const Keys(LogicalKeyboardKey.enter).activator: leave,
      },
      actions: [
        if (left != null)
          PrimaryButton(
            label: stashed != null
                ? 'Replace the stash on $left and switch'
                : 'Leave them on $left and switch',
            shortcut: '⏎',
            onPressed: leave,
          ),
        SecondaryButton(
          label: 'Commit changes…',
          onPressed: () => Navigator.of(context).pop('commit'),
        ),
      ],
      children: [
        Text(
          'Git did not switch, because it would overwrite your uncommitted changes in these '
          'files. Nothing was changed.',
          style: DangerDialog.text(context),
        ),
        Container(
          constraints: const BoxConstraints(maxHeight: 200),
          padding: const EdgeInsets.all(WsMetrics.space2),
          decoration: BoxDecoration(
            color: ws.subtle,
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          ),
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final file in files)
                Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space2,
                    vertical: WsMetrics.space1,
                  ),
                  child: Text(file, style: monoStyle(context)),
                ),
            ],
          ),
        ),
        if (left != null)
          Text(
            'Leave them on $left to switch, or commit them first.',
            style: DangerDialog.text(context),
          ),
      ],
    );
  }
}
