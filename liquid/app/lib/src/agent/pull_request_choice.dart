import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/pull_requests.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The pause after typing before GitHub is asked for branch names.
const _searchPause = Duration(milliseconds: 300);

/// GitHub's answer for the text `query`: the branches it names, or why it did not answer.
typedef _Answer = ({String query, List<BridgeName> names, String? failure});

/// Lets the user choose the target repository. Completes with the chosen `owner/name`, or none
/// when the dialog closes.
Future<String?> showTargetChoice(
  BuildContext context,
  BridgePullRequestPlan plan,
) => showDialog<String>(
  context: context,
  builder: (context) => _TargetChoice(plan: plan),
);

/// Lets the user choose the base branch among the target's branches, and those GitHub finds for
/// the text typed. Completes with the chosen branch, or none when the dialog closes.
Future<BridgeName?> showBaseChoice(
  BuildContext context, {
  required String target,
  required List<BridgeName> names,
  required bool more,
  required bool unknown,
  required Future<BridgePullRequestOutcome> Function(String query) search,
}) => showDialog<BridgeName>(
  context: context,
  builder: (context) => _BaseChoice(
    target: target,
    names: names,
    more: more,
    unknown: unknown,
    search: search,
  ),
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
    required this.search,
  });

  final String target;
  final List<BridgeName> names;

  /// GitHub may list more branches than [names].
  final bool more;

  /// Liquid has no record of the branch's start, and GitHub's default is unknown.
  final bool unknown;

  /// Asks GitHub for the branches whose names contain the text.
  final Future<BridgePullRequestOutcome> Function(String query) search;

  @override
  State<_BaseChoice> createState() => _BaseChoiceState();
}

class _BaseChoiceState extends State<_BaseChoice> {
  final _query = TextEditingController();
  Timer? _pause;

  /// The text of the request in flight; one runs at a time.
  String? _flying;

  /// GitHub's answer for the text last asked.
  _Answer? _answer;

  @override
  void initState() {
    super.initState();
    _query.addListener(_typed);
  }

  @override
  void dispose() {
    _pause?.cancel();
    _query.dispose();
    super.dispose();
  }

  String get _text => _query.text.trim();

  void _typed() {
    setState(() {});
    _pause?.cancel();
    _pause = Timer(_searchPause, _search);
  }

  Future<void> _search() async {
    final query = _text;
    if (_flying != null || query.isEmpty) return;
    setState(() => _flying = query);
    BridgePullRequestOutcome outcome;
    try {
      outcome = await widget.search(query);
    } on Object {
      outcome = unansweredOutcome;
    }
    if (!mounted) return;
    setState(() {
      _flying = null;
      _answer = (
        query: query,
        names: outcome.bases,
        failure: outcome.kind == BridgePullRequestOutcomeKind.done
            ? null
            : _searchFailure(outcome),
      );
    });
    // Text typed meanwhile is searched now, unless its pause is still running.
    if (_text != query && !(_pause?.isActive ?? false)) unawaited(_search());
  }

  /// GitHub's answer for the text now typed, once it has come.
  _Answer? get _current => switch (_answer) {
    final answer? when answer.query == _text => answer,
    _ => null,
  };

  /// GitHub's branches for the text typed, else the first branches containing it.
  List<BridgeName> get _matches {
    final answer = _current;
    if (answer != null && answer.failure == null) return answer.names;
    final query = _text.toLowerCase();
    return [
      for (final name in widget.names)
        if (name.shown.toLowerCase().contains(query)) name,
    ];
  }

  /// Chooses [name], and drops a search still waiting for its pause.
  void _choose(BridgeName name) {
    _pause?.cancel();
    Navigator.of(context).pop(name);
  }

  void _submit() {
    if (_matches.firstOrNull case final first?) {
      _choose(first);
    } else if (_current?.failure != null && widget.more) {
      // GitHub could not be asked: the name is checked when the plan is read again.
      _choose(BridgeName(name: _text, shown: _text));
    }
  }

  /// Why no branch is listed; none while GitHub has yet to answer the text typed.
  String? get _emptyNote {
    final answer = _current;
    if (answer == null) return _text.isEmpty ? _noMatch : null;
    if (answer.failure == null) {
      return 'No branch named “${answer.query}” in ${widget.target}.';
    }
    return _noMatch;
  }

  String get _noMatch => widget.more
      ? 'No match among the first ${widget.names.length} branches. Press Enter to use the name as typed.'
      : 'No branch matches.';

  @override
  Widget build(BuildContext context) {
    final matches = _matches;
    final answer = _current;
    final note = _emptyNote;
    final text = DangerDialog.text(context);
    return ActionDialog(
      title: 'Base branch',
      actions: const [],
      children: [
        TextField(
          key: const Key('base-filter'),
          controller: _query,
          autofocus: true,
          decoration: InputDecoration(
            hintText: 'Filter branches',
            suffixText: _flying == null ? null : 'Searching ${widget.target}…',
          ),
          onSubmitted: (_) => _submit(),
        ),
        for (final name in matches)
          _ChoiceRow(label: name.shown, onTap: () => _choose(name)),
        if (answer?.failure case final failure?)
          Text(
            'Could not search GitHub: $failure. The list shows the first ${widget.names.length} branches.',
            style: text,
          ),
        if (matches.isEmpty && note != null) Text(note, style: text),
        Text(
          'The base is the branch you want these commits to join.${widget.unknown ? " Liquid has no record of where this branch started, and GitHub's default could not be read." : ''} Only branches in ${widget.target} are listed.',
          style: text,
        ),
      ],
    );
  }
}

/// Why GitHub did not answer a base search: gh's message, else the plain words
/// [pullRequestProblem] gives for the outcome.
String _searchFailure(BridgePullRequestOutcome outcome) =>
    outcome.detail.isEmpty
    ? pullRequestProblem(outcome, device: null) ?? 'GitHub did not answer.'
    : outcome.detail;

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
