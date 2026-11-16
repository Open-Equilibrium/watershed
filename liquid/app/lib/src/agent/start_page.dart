import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/git.dart';
import '../rust/api/session.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'input_lane.dart';
import 'sidebar.dart';

/// The start page: T3 Code's greeting for the project, above the input area that starts a session.
class StartPage extends StatelessWidget {
  const StartPage({super.key, required this.state, required this.info});

  final SessionState state;
  final SessionInfo info;

  @override
  Widget build(BuildContext context) => Column(
    children: [
      const SizedBox(height: WsMetrics.space8),
      Expanded(
        child: Center(
          child: Text(
            'What should we build in ${info.project}?',
            textAlign: TextAlign.center,
            style: Theme.of(context).textTheme.titleLarge
                ?.copyWith(fontWeight: WsMetrics.fwSemibold),
          ),
        ),
      ),
      const SizedBox(height: inputClearance),
    ],
  );
}

/// The chips under the input area that set up a new session, as T3 Code's composer shows them:
/// workspace, model with its options, and permissions.
class StartChips extends StatelessWidget {
  const StartChips({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.model,
      state.providers,
      state.permissions,
    ]),
    builder: (context, _) => Wrap(
      spacing: WsMetrics.space2,
      runSpacing: WsMetrics.space2,
      children: [
        WorkspaceMenu(state: state),
        ModelMenu(
          providers: state.providers.value,
          choice: state.model.value,
          onChanged: (next) => unawaited(state.setModel(next)),
        ),
        PermissionsMenu(
          value: state.permissions.value,
          onChanged: (next) => unawaited(state.setPermissions(next)),
        ),
      ],
    ),
  );
}

/// The workspace selector: a new session starts in the project folder or in a new worktree from a
/// base ref, as T3 Code's workspace selector lets it.
class WorkspaceMenu extends StatelessWidget {
  const WorkspaceMenu({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([
      state.newWorktree,
      state.base,
      state.worktree,
    ]),
    builder: (context, _) {
      final worktree = state.newWorktree.value;
      final base = state.base.value;
      // T3's resolveCurrentWorkspaceLabel.
      final current = state.worktree.value == null
          ? 'Current checkout'
          : 'Current worktree';
      return Wrap(
        spacing: WsMetrics.space2,
        runSpacing: WsMetrics.space2,
        children: [
          PopupMenuButton<bool>(
            tooltip: 'Workspace',
            onSelected: (next) => state.newWorktree.value = next,
            itemBuilder: (context) => [
              PopupMenuItem(value: false, child: Text(current)),
              const PopupMenuItem(value: true, child: Text('New worktree')),
            ],
            child: _Chip(
              icon: worktree ? LucideIcons.gitBranch : LucideIcons.folder,
              label: worktree ? 'New worktree' : current,
              menu: true,
            ),
          ),
          if (worktree)
            Builder(
              builder: (context) => InkWell(
                borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                onTap: () => _pickBase(context),
                child: _Chip(
                  icon: LucideIcons.gitBranch,
                  label: base == null ? 'Select ref' : 'From $base',
                  menu: true,
                ),
              ),
            ),
        ],
      );
    },
  );

  /// Lists the project's refs under the chip and sets the one picked as the base.
  Future<void> _pickBase(BuildContext context) async {
    final box = context.findRenderObject()! as RenderBox;
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final position = RelativeRect.fromRect(
      box.localToGlobal(Offset.zero) & box.size,
      Offset.zero & overlay.size,
    );
    final BridgeRefs refs;
    try {
      refs = await state.git.value.backend.refs('', limit: 200);
    } on Object catch (error) {
      state.notify(NoticeLevel.error, 'Could not read the branches: $error');
      return;
    }
    if (!context.mounted) return;
    final chosen = await showMenu<String>(
      context: context,
      position: position,
      items: [
        for (final ref in refs.refs)
          PopupMenuItem(value: ref.name, child: Text(ref.shown)),
      ],
    );
    if (chosen != null) state.base.value = chosen;
  }
}

/// The model picker: the models grouped by provider, under "Choose model", with a chip for each
/// option the chosen model has, such as its reasoning effort.
class ModelMenu extends StatelessWidget {
  const ModelMenu({
    super.key,
    required this.providers,
    required this.choice,
    required this.onChanged,
  });

  final List<BridgeProvider> providers;
  final BridgeModelChoice? choice;
  final ValueChanged<BridgeModelChoice> onChanged;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final current = choice;
    final model = current == null ? null : modelOf(providers, current);
    return Row(
      mainAxisSize: MainAxisSize.min,
      spacing: WsMetrics.space2,
      children: [
        // A long model name shortens in the chip rather than overflowing the chips' row.
        Flexible(
          child: PopupMenuButton<BridgeModelChoice>(
            tooltip: 'Model',
            onSelected: onChanged,
            itemBuilder: (context) => [
              for (final provider in providers)
                if (provider.models.isNotEmpty) ...[
                  PopupMenuItem(
                    enabled: false,
                    height: 28,
                    child: Text(
                      provider.name,
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ),
                  for (final entry in provider.models)
                    PopupMenuItem(
                      value: BridgeModelChoice(
                        instance: provider.instance,
                        model: entry.id,
                        optionIds: const [],
                        values: const [],
                      ),
                      enabled: provider.available,
                      child: Text(entry.name),
                    ),
                ],
            ],
            child: _Chip(
              icon: LucideIcons.sparkles,
              label: model?.name ?? current?.model ?? 'Choose model',
              menu: true,
            ),
          ),
        ),
        if (current != null && model != null)
          for (final option in model.options)
            _EffortMenu(option: option, choice: current, onChanged: onChanged),
      ],
    );
  }
}

/// One select option of the chosen model, such as its reasoning effort.
class _EffortMenu extends StatelessWidget {
  const _EffortMenu({
    required this.option,
    required this.choice,
    required this.onChanged,
  });

  final BridgeModelOption option;
  final BridgeModelChoice choice;
  final ValueChanged<BridgeModelChoice> onChanged;

  @override
  Widget build(BuildContext context) {
    final at = choice.optionIds.indexOf(option.id);
    final value = at < 0 ? option.default_ : choice.values[at];
    final picked = option.choices.indexOf(value ?? '');
    return PopupMenuButton<String>(
      tooltip: option.label,
      onSelected: (id) => onChanged(_withValue(choice, option.id, id)),
      itemBuilder: (context) => [
        for (final (index, id) in option.choices.indexed)
          PopupMenuItem(value: id, child: Text(option.labels[index])),
      ],
      child: _Chip(
        icon: LucideIcons.slidersHorizontal,
        label: picked < 0 ? option.label : option.labels[picked],
        menu: true,
      ),
    );
  }
}

/// `choice` with option `optionId` set to `value`.
BridgeModelChoice _withValue(
  BridgeModelChoice choice,
  String optionId,
  String value,
) {
  final ids = [...choice.optionIds];
  final values = [...choice.values];
  final at = ids.indexOf(optionId);
  if (at < 0) {
    ids.add(optionId);
    values.add(value);
  } else {
    values[at] = value;
  }
  return BridgeModelChoice(
    instance: choice.instance,
    model: choice.model,
    optionIds: ids,
    values: values,
  );
}

/// The model `choice` names, as `providers` list it.
BridgeModel? modelOf(
  List<BridgeProvider> providers,
  BridgeModelChoice choice,
) => providers
    .where((provider) => provider.instance == choice.instance)
    .expand((provider) => provider.models)
    .where((model) => model.id == choice.model)
    .firstOrNull;

/// The model's name for `choice`, or its id when no provider lists it.
String modelName(List<BridgeProvider> providers, BridgeModelChoice choice) =>
    modelOf(providers, choice)?.name ?? choice.model;

/// T3's name and description of a permission mode.
(String, String) permissionsText(BridgePermissions mode) => switch (mode) {
  BridgePermissions.supervised => (
    'Supervised',
    'Ask before commands and file changes.',
  ),
  BridgePermissions.autoAcceptEdits => (
    'Auto-accept edits',
    'Auto-approve edits, ask before other actions.',
  ),
  BridgePermissions.auto => (
    'Auto',
    'Supported providers approve routine actions; others still ask.',
  ),
  BridgePermissions.fullAccess => (
    'Full access',
    'Allow commands and edits without prompts.',
  ),
};

/// The permissions chip: what the agent may do without asking, as T3 Code's composer footer shows.
class PermissionsMenu extends StatelessWidget {
  const PermissionsMenu({
    super.key,
    required this.value,
    required this.onChanged,
  });

  final BridgePermissions value;
  final ValueChanged<BridgePermissions> onChanged;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return PopupMenuButton<BridgePermissions>(
      tooltip: 'Permissions',
      onSelected: onChanged,
      itemBuilder: (context) => [
        for (final mode in BridgePermissions.values)
          PopupMenuItem(
            value: mode,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(permissionsText(mode).$1),
                Text(
                  permissionsText(mode).$2,
                  style: TextStyle(
                    color: ws.textFaint,
                    fontSize: WsMetrics.fsSmall,
                  ),
                ),
              ],
            ),
          ),
      ],
      child: _Chip(
        icon: LucideIcons.shield,
        label: permissionsText(value).$1,
        menu: true,
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({required this.icon, required this.label, this.menu = false});

  final IconData icon;
  final String label;
  final bool menu;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      height: 28,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
      decoration: BoxDecoration(
        border: Border.all(color: ws.borderStrong),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 13, color: ws.textMuted),
          const SizedBox(width: WsMetrics.space1),
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: ws.textMuted,
                fontSize: WsMetrics.fsCompact,
              ),
            ),
          ),
          if (menu) ...[
            const SizedBox(width: WsMetrics.space1),
            Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
          ],
        ],
      ),
    );
  }
}
