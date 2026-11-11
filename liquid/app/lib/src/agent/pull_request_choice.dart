import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// Lets the user choose the target repository. Completes with the chosen `owner/name`, or none
/// when the dialog closes.
Future<String?> showTargetChoice(
  BuildContext context,
  BridgePullRequestPlan plan,
) => showDialog<String>(
  context: context,
  builder: (context) => _TargetChoice(plan: plan),
);

/// Lets the user choose the base branch among the target's branches on GitHub.
Future<BridgeName?> showBaseChoice(
  BuildContext context, {
  required String target,
  required List<BridgeName> names,
  required bool more,
  required bool unknown,
}) => showDialog<BridgeName>(
  context: context,
  builder: (context) =>
      _BaseChoice(target: target, names: names, more: more, unknown: unknown),
);

/// The target repository: GitHub's parent of a fork, or the fork itself.
class _TargetChoice extends StatelessWidget {
  const _TargetChoice({required this.plan});

  final BridgePullRequestPlan plan;

  @override
  Widget build(BuildContext context) => ActionDialog(
    title: 'Target repository',
    actions: const [],
    children: [
      for (final target in plan.targets)
        _ChoiceRow(
          label: target,
          detail: target == plan.identity.headRepository
              ? 'this fork'
              : 'parent of ${plan.identity.headRepository}',
          onTap: () => Navigator.of(context).pop(target),
        ),
    ],
  );
}

/// The base branch, from the target's branches on GitHub.
class _BaseChoice extends StatefulWidget {
  const _BaseChoice({
    required this.target,
    required this.names,
    required this.more,
    required this.unknown,
  });

  final String target;
  final List<BridgeName> names;

  /// GitHub may list more branches than [names].
  final bool more;

  /// Liquid has no record of the branch's start, and GitHub's default is unknown.
  final bool unknown;

  @override
  State<_BaseChoice> createState() => _BaseChoiceState();
}

class _BaseChoiceState extends State<_BaseChoice> {
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

  List<BridgeName> get _matches {
    final query = _query.text.trim().toLowerCase();
    return [
      for (final name in widget.names)
        if (name.shown.toLowerCase().contains(query)) name,
    ];
  }

  void _submit() {
    final typed = _query.text.trim();
    if (_matches.firstOrNull case final first?) {
      Navigator.of(context).pop(first);
    } else if (widget.more && typed.isNotEmpty) {
      // GitHub checks a branch Liquid did not list when the plan is read again.
      Navigator.of(context).pop(BridgeName(name: typed, shown: typed));
    }
  }

  @override
  Widget build(BuildContext context) {
    final matches = _matches;
    final text = DangerDialog.text(context);
    return ActionDialog(
      title: 'Base branch',
      actions: const [],
      children: [
        TextField(
          key: const Key('base-filter'),
          controller: _query,
          autofocus: true,
          decoration: const InputDecoration(hintText: 'Filter branches'),
          onSubmitted: (_) => _submit(),
        ),
        for (final name in matches)
          _ChoiceRow(
            label: name.shown,
            onTap: () => Navigator.of(context).pop(name),
          ),
        if (matches.isEmpty)
          Text(
            widget.more
                ? 'No match among the first ${widget.names.length} branches. Press Enter to use the name as typed.'
                : 'No branch matches.',
            style: text,
          ),
        Text(
          'The base is the branch you want these commits to join.${widget.unknown ? " Liquid has no record of where this branch started, and GitHub's default could not be read." : ''} Only branches in ${widget.target} are listed.',
          style: text,
        ),
      ],
    );
  }
}

class _ChoiceRow extends StatelessWidget {
  const _ChoiceRow({required this.label, required this.onTap, this.detail});

  final String label;
  final String? detail;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
    onTap: onTap,
    child: SizedBox(
      height: 28,
      child: Row(
        spacing: WsMetrics.space3,
        children: [
          Expanded(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: monoStyle(context, color: context.ws.text),
            ),
          ),
          if (detail case final detail?)
            Text(
              detail,
              style: TextStyle(
                color: context.ws.textFaint,
                fontSize: WsMetrics.fsSmall,
              ),
            ),
        ],
      ),
    ),
  );
}
