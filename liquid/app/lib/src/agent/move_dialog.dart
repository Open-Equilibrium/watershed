import 'package:flutter/material.dart';

import 'dart:async';

import 'package:flutter/services.dart';

import '../agent_hosts/agent_hosts.dart';
import '../controls.dart';
import '../format.dart';
import '../rust/api/moves.dart';
import '../session/backend.dart';
import '../session/move.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

Future<void> moveSession(
  BuildContext context,
  SessionState state,
  HostThread listed, {
  required String Function(String? address) nameOf,
}) async {
  final source = state.projectOf(listed.address);
  if (source == null) {
    state.notify(
      NoticeLevel.error,
      'Could not move the session: its agent host does not answer.',
    );
    return;
  }
  final model = MoveModel(
    source: source,
    sourceAddress: listed.address,
    open: state.connectTo,
    running: state.runningMoves,
    started: state.startedMoves,
    threadId: listed.thread.id,
    sourceName: nameOf(listed.address),
    candidates: [
      for (final address in state.hostAddresses)
        if (address != listed.address)
          MoveCandidate(
            address: address,
            name: nameOf(address),
            project: state.projectOf(address),
          ),
    ],
  );
  await model.load();
  if (!context.mounted) return;
  final check = model.check;
  if (check == null) {
    state.notify(
      NoticeLevel.error,
      model.problem ?? 'Could not read the session.',
    );
    return;
  }
  if (!check.movable) {
    state.notify(
      NoticeLevel.error,
      'Only Claude Code and Codex sessions can move.',
    );
    return;
  }
  final title = check.title;
  var showing = true;
  BuildContext? dialog;
  void close() {
    final open = dialog;
    if (showing && open != null && open.mounted) Navigator.of(open).pop();
    showing = false;
  }

  await showDialog<void>(
    context: context,
    builder: (dialogContext) {
      dialog = dialogContext;
      return MoveDialog(
        model: model,
        onMoved: (session, address) {
          close();
          state.notify(
            NoticeLevel.info,
            'Moved “$title” to ${nameOf(address)}.',
          );
          unawaited(state.showMoved(session, address));
        },
        onUnconfirmed: (unconfirmed, address) {
          close();
          state.notify(NoticeLevel.warning, unconfirmed.message);
          final session = unconfirmed.session;
          unawaited(
            session == null
                ? state.loadStart()
                : state.showMoved(session, address),
          );
        },
        onFailed: (failure) {
          close();
          state.notify(NoticeLevel.error, failure.message);
        },
      );
    },
  );
  showing = false;
}

class MoveDialog extends StatelessWidget {
  const MoveDialog({
    super.key,
    required this.model,
    required this.onMoved,
    required this.onUnconfirmed,
    required this.onFailed,
  });

  final MoveModel model;

  final void Function(SessionBackend session, String? address) onMoved;

  final void Function(MoveUnconfirmed unconfirmed, String? address)
  onUnconfirmed;

  final void Function(MoveFailure failure) onFailed;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: model,
    builder: (context, _) {
      final flow = model.flow;
      final check = model.check;
      final target = model.selected?.candidate.name;
      final move = PrimaryButton(
        label: 'Move',
        onPressed: flow == null && model.canMove ? _start : null,
      );
      final reason = model.blockedReason;
      return ActionDialog(
        title: flow == null || target == null
            ? 'Move “${check?.title ?? 'this session'}” to another host'
            : 'Moving to $target',
        bindings: {
          if (flow == null && model.canMove)
            const Keys(LogicalKeyboardKey.enter).activator: () => _start(),
        },
        actions: [
          if (reason == null || flow != null)
            move
          else
            Tooltip(message: reason, child: move),
        ],
        children: flow == null
            ? [_Choices(model: model)]
            : [_Progress(flow: flow)],
      );
    },
  );

  void _start() {
    final address = model.selected?.candidate.address;
    model.start().then(
      (session) => onMoved(session, address),
      onError: (Object error) => switch (error) {
        MoveUnconfirmed() => onUnconfirmed(error, address),
        MoveFailure() => onFailed(error),
        _ => onFailed(MoveFailure('starting the move', '$error')),
      },
    );
  }
}

class _Choices extends StatelessWidget {
  const _Choices({required this.model});

  final MoveModel model;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final check = model.check;
    final selected = model.selected;
    final target = selected?.candidate.name;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space4,
      children: [
        if (model.problem case final problem?)
          WarningNotice(problem, danger: true),
        if (check != null)
          Text(
            [
              ?check.harness,
              ?check.model,
              model.sourceName,
              if (check.branch case final branch?) 'branch $branch',
            ].join(' · '),
            style: TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall),
          ),
        for (final option in model.options)
          _HostRow(
            option: option,
            selected: option == selected,
            onTap: option.available ? () => model.select(option) : null,
          ),
        if (model.options.every((option) => !option.available))
          const WarningNotice('No other agent host can take this session.'),
        if (check != null && selected != null && target != null)
          _CheckBox(
            children: [
              if (check.blocked case final blocked?) _Check.warn(blocked),
              _CommitCheck(model: model),
              if (check.remoteUrl != null || check.blocked == null)
                _ProjectCheck(model: model, target: target),
              _ConfigCheck(model: model, target: target),
              const _Check.info(
                'Secrets you entered in this session are asked for again there',
              ),
            ],
          ),
      ],
    );
  }
}

class _HostRow extends StatelessWidget {
  const _HostRow({required this.option, required this.selected, this.onTap});

  final MoveOption option;
  final bool selected;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final color = option.available ? ws.text : ws.textFaint;
    return Semantics(
      selected: selected,
      inMutuallyExclusiveGroup: true,
      child: Shortcuts(
        shortcuts: activateKeys,
        child: InkWell(
          key: Key('move-target-${option.candidate.name}'),
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          onTap: onTap,
          child: Container(
            height: 40,
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
            decoration: BoxDecoration(
              color: selected ? ws.selected : null,
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
            ),
            child: Row(
              children: [
                Container(
                  width: 14,
                  height: 14,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: selected ? ws.accent : ws.borderStrong,
                      width: selected ? 4 : 1.5,
                    ),
                  ),
                ),
                const SizedBox(width: WsMetrics.space3),
                Expanded(
                  child: Text(
                    option.candidate.name,
                    style: TextStyle(color: color),
                  ),
                ),
                Text(
                  option.reason ?? (option.available ? 'Connected' : 'Offline'),
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _CheckBox extends StatelessWidget {
  const _CheckBox({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.all(WsMetrics.space4),
    decoration: BoxDecoration(
      color: context.ws.subtle,
      borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space3,
      children: children,
    ),
  );
}

class _Check extends StatelessWidget {
  const _Check._(this._kind, this._text, {this.control});

  const _Check.ok(String text, {Widget? control})
    : this._(_CheckKind.ok, text, control: control);
  const _Check.info(String text) : this._(_CheckKind.info, text);
  const _Check.warn(String text, {Widget? control})
    : this._(_CheckKind.warn, text, control: control);

  final _CheckKind _kind;
  final String _text;
  final Widget? control;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (icon, color) = switch (_kind) {
      _CheckKind.ok => (LucideIcons.check, ws.success),
      _CheckKind.warn => (LucideIcons.triangleAlert, ws.warning),
      _CheckKind.info => (LucideIcons.info, ws.textFaint),
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space2,
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(icon, size: 16, color: color),
            const SizedBox(width: WsMetrics.space3),
            Expanded(
              child: Text(
                _text,
                style: TextStyle(color: ws.text, fontSize: WsMetrics.fsCompact),
              ),
            ),
          ],
        ),
        if (control case final control?)
          Padding(padding: const EdgeInsets.only(left: 28), child: control),
      ],
    );
  }
}

enum _CheckKind { ok, info, warn }

class _CommitCheck extends StatelessWidget {
  const _CommitCheck({required this.model});

  final MoveModel model;

  @override
  Widget build(BuildContext context) {
    final check = model.check!;
    if (check.uncommittedFiles == 0) {
      return _Check.ok(
        check.branch == null
            ? 'No uncommitted changes'
            : 'No uncommitted changes; ${check.branch} is pushed to its remote when you move',
      );
    }
    final files = check.uncommittedFiles;
    return _Check.warn(
      '${plural(files, 'file')} ${files == 1 ? 'has' : 'have'} uncommitted changes',
      control: Segments<bool>(
        options: [
          (true, 'Commit and push'),
          (false, 'Leave on ${model.sourceName}'),
        ],
        selected: model.commit,
        onSelect: (commit) => model.commit = commit,
        dense: true,
      ),
    );
  }
}

class _ProjectCheck extends StatefulWidget {
  const _ProjectCheck({required this.model, required this.target});

  final MoveModel model;
  final String target;

  @override
  State<_ProjectCheck> createState() => _ProjectCheckState();
}

class _ProjectCheckState extends State<_ProjectCheck> {
  late final _path = TextEditingController(text: widget.model.clonePath);

  @override
  void dispose() {
    _path.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    final target = widget.target;
    final matches = model.matching;
    if (matches.isNotEmpty) {
      final root = model.chosenRoot ?? matches.first.root;
      final landing = switch (model.landingOf(root)) {
        BridgeLanding(kind: BridgeLandingKind.blocked, reason: final reason?) =>
          _Check.warn(reason),
        BridgeLanding(kind: BridgeLandingKind.continue_, path: final path?) =>
          _Check.info(
            'The branch is checked out at $path, so the session continues there.',
          ),
        BridgeLanding(kind: BridgeLandingKind.attach) => const _Check.info(
          'The branch is not checked out here, so the session gets a worktree from it.',
        ),
        _ => null,
      };
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space3,
        children: [
          _Check.ok(
            'Project on $target: $root, same remote',
            control: matches.length > 1
                ? _ChangeLink(model: model, target: target)
                : null,
          ),
          ?landing,
        ],
      );
    }
    if (model.check?.remoteUrl == null && model.picked == null) {
      return _Check.warn(noRemoteFilesReason);
    }
    return _Check.warn(
      'No project on $target has this remote',
      control: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space2,
        children: [
          Segments<bool>(
            options: const [(false, 'Clone to'), (true, 'Use a project')],
            selected: model.useProject,
            onSelect: (useProject) => model.useProject = useProject,
            dense: true,
          ),
          if (model.useProject)
            _PickerField(model: model, target: target)
          else
            TextField(
              key: const Key('move-folder'),
              controller: _path,
              style: const TextStyle(fontSize: WsMetrics.fsCompact),
              onChanged: (value) => model.clonePath = value,
            ),
        ],
      ),
    );
  }
}

class _PickerField extends StatelessWidget {
  const _PickerField({required this.model, required this.target});

  final MoveModel model;
  final String target;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return InkWell(
      key: const Key('move-project-field'),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      onTap: () => showProjectPicker(context, model),
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: WsMetrics.space3,
          vertical: WsMetrics.space2,
        ),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          border: Border.all(color: ws.borderStrong),
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(
                model.picked?.root ?? 'Choose a project…',
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  color: model.picked == null ? ws.textFaint : ws.text,
                  fontSize: WsMetrics.fsCompact,
                ),
              ),
            ),
            Icon(LucideIcons.chevronDown, size: 14, color: ws.textFaint),
          ],
        ),
      ),
    );
  }
}

class _ChangeLink extends StatelessWidget {
  const _ChangeLink({required this.model, required this.target});

  final MoveModel model;
  final String target;

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.centerRight,
    child: InkWell(
      onTap: () => showProjectPicker(context, model),
      child: Text('Change…', style: TextStyle(color: context.ws.accent)),
    ),
  );
}

Future<void> showProjectPicker(BuildContext context, MoveModel model) async {
  final project = await showDialog<BridgeHostProject>(
    context: context,
    builder: (dialogContext) => ActionDialog(
      title:
          'Choose a project on ${model.selected?.candidate.name ?? 'the host'}',
      actions: const [],
      children: [
        ListenableBuilder(
          listenable: model,
          builder: (listContext, _) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final project in model.projects)
                _ProjectRow(
                  project: project,
                  reason: model.reasonAgainst(project),
                  onTap: () => Navigator.of(dialogContext).pop(project),
                ),
              _AddFolder(model: model),
            ],
          ),
        ),
      ],
    ),
  );
  if (project != null) model.pick(project);
}

class _ProjectRow extends StatelessWidget {
  const _ProjectRow({
    required this.project,
    required this.reason,
    required this.onTap,
  });

  final BridgeHostProject project;

  final String? reason;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final enabled = reason == null;
    final row = InkWell(
      key: Key('pick-${project.root}'),
      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      onTap: enabled ? onTap : null,
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: WsMetrics.space2),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              project.root,
              style: TextStyle(color: enabled ? ws.text : ws.textFaint),
            ),
            Text(
              project.remoteUrl ?? 'No Git remote',
              style: TextStyle(
                color: ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
            if (reason case final reason?)
              Text(
                reason,
                style: TextStyle(
                  color: ws.warning,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
          ],
        ),
      ),
    );
    return reason == null ? row : Tooltip(message: reason!, child: row);
  }
}

class _AddFolder extends StatefulWidget {
  const _AddFolder({required this.model});

  final MoveModel model;

  @override
  State<_AddFolder> createState() => _AddFolderState();
}

class _AddFolderState extends State<_AddFolder> {
  final _path = TextEditingController();
  String? _problem;
  bool _adding = false;

  @override
  void dispose() {
    _path.dispose();
    super.dispose();
  }

  Future<void> _add() async {
    final root = _path.text.trim();
    if (root.isEmpty) return;
    setState(() {
      _adding = true;
      _problem = null;
    });
    try {
      await widget.model.addFolder(root);
      if (mounted) writeText(_path, TextEditingValue.empty);
    } on Object catch (error) {
      if (mounted) setState(() => _problem = messageOf(error));
    } finally {
      if (mounted) setState(() => _adding = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space2,
      children: [
        Row(
          children: [
            Expanded(
              child: TextField(
                key: const Key('add-folder-path'),
                controller: _path,
                style: const TextStyle(fontSize: WsMetrics.fsCompact),
                decoration: const InputDecoration(
                  hintText: 'A folder of this host',
                  isDense: true,
                ),
              ),
            ),
            const SizedBox(width: WsMetrics.space3),
            SecondaryButton(
              key: const Key('add-folder-submit'),
              label: 'Add folder',
              onPressed: _adding ? null : _add,
            ),
          ],
        ),
        if (_problem case final problem?)
          Text(
            problem,
            style: TextStyle(color: ws.warning, fontSize: WsMetrics.fsSmall),
          ),
      ],
    );
  }
}

class _ConfigCheck extends StatefulWidget {
  const _ConfigCheck({required this.model, required this.target});

  final MoveModel model;
  final String target;

  @override
  State<_ConfigCheck> createState() => _ConfigCheckState();
}

class _ConfigCheckState extends State<_ConfigCheck> {
  bool _shown = false;

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    final differences = model.differences;
    final target = widget.target;
    if (differences.isEmpty) {
      return _Check.info(
        "$target's configuration matches ${model.sourceName}'s",
      );
    }
    final ws = context.ws;
    return _Check.warn(
      "$target's configuration differs: ${describeDifferences(differences)}",
      control: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space2,
        children: [
          Row(
            children: [
              const Spacer(),
              InkWell(
                onTap: () => setState(() => _shown = !_shown),
                child: Text(
                  _shown ? 'Hide' : 'Show',
                  style: TextStyle(color: ws.accent),
                ),
              ),
            ],
          ),
          if (_shown)
            Text(
              [
                for (final change in differences)
                  '${change.name} (${_kindName(change.kind)}, ${change.direction.name})',
              ].join('\n'),
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
          Segments<bool>(
            options: [
              (false, "Use $target's"),
              (true, "Bring ${model.sourceName}'s"),
            ],
            selected: model.bringConfiguration,
            onSelect: (bring) => model.bringConfiguration = bring,
            dense: true,
          ),
        ],
      ),
    );
  }
}

String describeDifferences(List<BridgeConfigChange> changes) {
  int count(BridgeConfigKind kind) =>
      changes.where((change) => change.kind == kind).length;
  return [
    if (count(BridgeConfigKind.skill) > 0)
      plural(count(BridgeConfigKind.skill), 'skill'),
    if (count(BridgeConfigKind.mcpServer) > 0)
      plural(count(BridgeConfigKind.mcpServer), 'MCP server'),
    for (final change in changes)
      if (change.kind == BridgeConfigKind.instruction) change.name,
  ].join(', ');
}

String _kindName(BridgeConfigKind kind) => switch (kind) {
  BridgeConfigKind.instruction => 'instruction',
  BridgeConfigKind.skill => 'skill',
  BridgeConfigKind.mcpServer => 'MCP server',
};

class _Progress extends StatelessWidget {
  const _Progress({required this.flow});

  final MoveFlow flow;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return ValueListenableBuilder(
      valueListenable: flow.steps,
      builder: (context, steps, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        spacing: WsMetrics.space3,
        children: [
          for (final step in steps)
            Row(
              children: [
                switch (step.state) {
                  MoveStepState.done => Icon(
                    LucideIcons.check,
                    size: 16,
                    color: ws.success,
                  ),
                  MoveStepState.now => Icon(
                    LucideIcons.loader,
                    size: 16,
                    color: ws.textMuted,
                  ),
                  MoveStepState.failed => Icon(
                    LucideIcons.circleX,
                    size: 16,
                    color: ws.danger,
                  ),
                },
                const SizedBox(width: WsMetrics.space3),
                Expanded(
                  child: Text(
                    step.text,
                    style: TextStyle(
                      color: step.state == MoveStepState.done
                          ? ws.textMuted
                          : ws.text,
                      fontWeight: step.state == MoveStepState.now
                          ? WsMetrics.fwMedium
                          : null,
                    ),
                  ),
                ),
              ],
            ),
        ],
      ),
    );
  }
}
