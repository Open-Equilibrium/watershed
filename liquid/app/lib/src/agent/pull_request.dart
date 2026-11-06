import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../session/pull_requests.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'commit_dialog.dart';
import 'git_actions.dart';
import 'session_menus.dart';

/// The approved "Create pull request" dialog for a pushed branch (pull request mockups §2–3).
/// Its title and description start from the latest commit, and the Chat's model rewrites the
/// fields the user has not edited.
Future<void> showPullRequestDialog(
  BuildContext context,
  SessionState state, {
  required String device,
}) async {
  final next = await showDialog<_Next>(
    context: context,
    builder: (_) => _PullRequestDialog(state: state, device: device),
  );
  if (!context.mounted) return;
  switch (next) {
    case _Next.push:
      await pushBranch(context, state, device: device);
    case _Next.branch:
      await showBranchMenu(context, state);
    case _Next.commit:
      await showCommitDialog(context, state);
    case null:
  }
}

/// What the dialog hands off to when the branch needs another step first.
enum _Next { push, branch, commit }

/// Where the title and description came from.
enum _Text { commit, writing, written, failed, noModel }

class _PullRequestDialog extends StatefulWidget {
  const _PullRequestDialog({required this.state, required this.device});

  final SessionState state;
  final String device;

  @override
  State<_PullRequestDialog> createState() => _PullRequestDialogState();
}

class _PullRequestDialogState extends State<_PullRequestDialog> {
  final _title = TextEditingController();
  final _body = TextEditingController();
  final _titleFocus = FocusNode();
  final _bodyFocus = FocusNode();
  bool _draft = false;

  /// GitHub is being read.
  bool _reading = true;

  /// Why no plan could be read, shown while there is none.
  BridgePullRequestOutcome? _refused;

  /// The latest plan; its title and description fill the fields only once.
  BridgePullRequestPlan? _plan;
  String? _recorded;
  BridgePullRequest? _existing;
  bool _creating = false;
  int _seconds = 0;
  Timer? _elapsed;

  /// Why the last read or creation failed, and whether a creation's result is unknown.
  String? _error;
  bool _unknown = false;

  /// The last read failed, so the plan shown may be outdated; Create waits for another.
  bool _stale = false;
  bool _changed = false;
  int _reads = 0;

  /// The Chat's model as `provider/id`, read when the dialog opened.
  late final String? _model = widget.state.model.value;
  _Text _text = _Text.commit;

  /// The commits the model last wrote for, as `mergeBase..head`.
  String? _range;
  int _writes = 0;

  /// What the dialog last put in the fields; a field that differs is the user's.
  String _filledTitle = '';
  String _filledBody = '';

  PullRequests get _pulls => widget.state.pulls;

  @override
  void initState() {
    super.initState();
    _title.addListener(() => setState(() {}));
    _bodyFocus.addListener(() => setState(() {}));
    unawaited(_read());
  }

  @override
  void dispose() {
    _stopWriting();
    _elapsed?.cancel();
    _title.dispose();
    _body.dispose();
    _titleFocus.dispose();
    _bodyFocus.dispose();
    super.dispose();
  }

  Future<void> _read({String? target, String? base}) async {
    final read = ++_reads;
    setState(() {
      _reading = true;
      _error = null;
      _unknown = false;
      _stale = false;
      if (_plan == null) _refused = null;
    });
    BridgePullRequestOutcome outcome;
    try {
      _recorded ??= await _pulls.recordedBase();
      outcome = await _pulls.plan(target: target, base: base);
    } on Object {
      outcome = unansweredOutcome;
    }
    if (!mounted || read != _reads) return;
    setState(() {
      _reading = false;
      if (outcome.plan != null) {
        _accept(outcome.plan!);
      } else if (_plan != null) {
        // The last plan stays in view; Create waits for a read that succeeds.
        _stale = true;
        _error = pullRequestProblem(outcome, device: widget.device);
      } else {
        _refused = outcome;
      }
    });
  }

  /// Reads again what the dialog shows: the chosen target and base.
  Future<void> _checkAgain() =>
      _read(target: _plan?.target, base: _plan?.base?.name);

  void _accept(BridgePullRequestPlan plan) {
    if (_plan == null) {
      _title.text = _filledTitle = plan.subject;
      _body.text = _filledBody = plan.body;
      if (_model == null) _text = _Text.noModel;
      // The form replaces the reading state, whose dialog took the focus first.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _titleFocus.requestFocus();
      });
    }
    _plan = plan;
    _refused = null;
    _existing = plan.open;
    _write(plan);
  }

  String get _writer => _model!.substring(_model.indexOf('/') + 1);

  /// Has the Chat's model write the fields the user has not edited, once for each range of
  /// commits.
  void _write(BridgePullRequestPlan plan) {
    final model = _model;
    final comparison = plan.comparison;
    if (model == null ||
        plan.open != null ||
        comparison == null ||
        comparison.commits == 0 ||
        comparison.mergeBase.isEmpty) {
      return;
    }
    final range = '${comparison.mergeBase}..${plan.identity.headCommit}';
    if (range == _range) return;
    _range = range;
    final write = ++_writes;
    _text = _Text.writing;
    unawaited(() async {
      BridgePullRequestText? text;
      try {
        text = await _pulls.write(model, plan);
      } on Object {
        text = null;
      }
      if (!mounted || write != _writes) return;
      setState(() {
        if (text == null) {
          _text = _Text.failed;
          return;
        }
        _text = _Text.written;
        if (_title.text == _filledTitle) {
          _title.text = _filledTitle = text.title;
        }
        if (_body.text == _filledBody) {
          _body.text = _filledBody = text.body;
        }
      });
    }());
  }

  /// Stops the model writing; what it would have written is dropped.
  void _stopWriting() {
    if (_text != _Text.writing) return;
    _writes++;
    _text = _Text.commit;
    unawaited(_pulls.stopWriting().catchError((Object _) {}));
  }

  bool get _sameBranch {
    final plan = _plan!;
    return plan.base?.name == plan.identity.headBranch &&
        plan.target == plan.identity.headRepository;
  }

  bool get _ready {
    final plan = _plan;
    return plan != null &&
        !_reading &&
        !_stale &&
        !_creating &&
        plan.target != null &&
        plan.base != null &&
        !plan.baseMissing &&
        plan.comparison != null &&
        plan.comparison!.commits != 0 &&
        !_sameBranch &&
        _title.text.trim().isNotEmpty;
  }

  Future<void> _create() async {
    if (!_ready) return;
    final plan = _plan!;
    setState(() {
      _stopWriting();
      _creating = true;
      _seconds = 0;
      _error = null;
      _unknown = false;
      _changed = false;
    });
    _elapsed = Timer.periodic(
      const Duration(seconds: 1),
      (timer) => setState(() => _seconds = timer.tick),
    );
    BridgePullRequestOutcome outcome;
    try {
      outcome = await _pulls.create(
        BridgePullRequestDraft(
          identity: plan.identity,
          target: plan.target!,
          base: plan.base!.name,
          title: _title.text.trim(),
          body: _body.text,
          draft: _draft,
        ),
      );
    } on Object {
      outcome = outcomeOf(BridgePullRequestOutcomeKind.unknown);
    } finally {
      _elapsed?.cancel();
      if (mounted) setState(() => _creating = false);
    }
    if (!mounted) return;
    switch (outcome.kind) {
      case BridgePullRequestOutcomeKind.done:
        final pull = outcome.pullRequest!;
        widget.state.notify(
          BridgeLevel.info,
          'Created ${pull.state == BridgePullRequestState.draft ? 'draft ' : ''}pull request #${pull.number}: “${pull.title}”',
        );
        Navigator.of(context).pop();
      case BridgePullRequestOutcomeKind.exists:
        setState(() => _existing = outcome.pullRequest);
      case BridgePullRequestOutcomeKind.changed:
        setState(() {
          _accept(outcome.plan!);
          _changed = true;
        });
      default:
        final problem = pullRequestProblem(outcome, device: widget.device)!;
        setState(() {
          _unknown = outcome.kind == BridgePullRequestOutcomeKind.unknown;
          _error =
              outcome.kind == BridgePullRequestOutcomeKind.unknown ||
                  outcome.kind == BridgePullRequestOutcomeKind.notCreated
              ? problem
              : '$problem No pull request was created.';
        });
    }
  }

  Future<void> _chooseBase() async {
    final plan = _plan!;
    final target = plan.target;
    if (target == null) return;
    BridgePullRequestOutcome listed;
    try {
      listed = await _pulls.bases(plan.identity.host, target);
    } on Object {
      listed = unansweredOutcome;
    }
    if (!mounted) return;
    if (listed.kind != BridgePullRequestOutcomeKind.done) {
      setState(
        () => _error = pullRequestProblem(listed, device: widget.device),
      );
      return;
    }
    final chosen = await showDialog<BridgeName>(
      context: context,
      builder: (context) => _BaseChoice(
        target: target,
        names: listed.bases,
        more: listed.moreBases,
        unknown: _recorded == null && plan.defaultBranch == null,
      ),
    );
    if (chosen != null && mounted) {
      await _read(target: target, base: chosen.name);
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = DangerDialog.text(context);
    if (_existing case final pull?) {
      return ActionDialog(
        title: 'Pull request #${pull.number} already exists',
        dismiss: 'Close',
        actions: [
          PrimaryButton(
            label: 'Open in browser ↗',
            onPressed: () => unawaited(openInBrowser(widget.state, pull.url)),
          ),
        ],
        children: [
          Text(
            '“${pull.title}” is open from ${pull.head} to ${pull.base} in ${pull.repository}. Pushing this branch updates it.',
            style: text,
          ),
        ],
      );
    }
    final plan = _plan;
    if (plan != null) return _form(context, plan);
    if (_refused case final refused?) return _refusal(context, refused);
    return ActionDialog(
      title: 'Create pull request',
      actions: const [],
      children: [
        Row(
          spacing: WsMetrics.space3,
          children: [
            const SizedBox.square(
              dimension: 14,
              child: CircularProgressIndicator(strokeWidth: 1.5),
            ),
            Text('Reading GitHub…', style: text),
          ],
        ),
      ],
    );
  }

  /// Where the title and description came from (mockup §2's generating and failure states).
  Widget? _textNote(
    BuildContext context,
    BridgePullRequestPlan plan,
    TextStyle faint,
  ) {
    final commits = plan.comparison?.commits.toInt() ?? 0;
    return switch (_text) {
      _Text.commit => null,
      _Text.writing => Text(
        '◌ $_writer is writing the title and description… The fields stay editable.',
        style: faint,
      ),
      _Text.written => Text(
        'Written by $_writer from ${commits == 1 ? 'the commit and its diff' : 'the $commits commits and their diff'}. Edit before creating.',
        style: faint,
      ),
      _Text.failed => WarningNotice(
        "Could not write the text with $_writer. Filled from the latest commit's subject and body instead; edit before creating.",
      ),
      _Text.noModel => Text(
        "Filled from the latest commit's subject and body; the Chat has no model selected.",
        style: faint,
      ),
    };
  }

  Widget _refusal(BuildContext context, BridgePullRequestOutcome outcome) {
    final text = DangerDialog.text(context);
    final problem = pullRequestProblem(outcome, device: widget.device)!;
    final uncommitted = widget.state.git.status.value?.changed ?? 0;
    void next(_Next next) => Navigator.of(context).pop(next);
    final checkAgain = SecondaryButton(
      label: 'Check again',
      onPressed: _reading ? null : _checkAgain,
    );
    final (title, actions) = switch (outcome.kind) {
      BridgePullRequestOutcomeKind.needsPush => (
        'Push before creating a pull request',
        [PrimaryButton(label: 'Push…', onPressed: () => next(_Next.push))],
      ),
      BridgePullRequestOutcomeKind.noBranch => (
        'Detached HEAD',
        [
          PrimaryButton(
            label: 'Create branch…',
            onPressed: () => next(_Next.branch),
          ),
        ],
      ),
      BridgePullRequestOutcomeKind.noCommit => (
        'Nothing committed',
        [
          PrimaryButton(
            label: 'Commit changes…',
            onPressed: () => next(_Next.commit),
          ),
        ],
      ),
      BridgePullRequestOutcomeKind.behind => (
        'Pull before creating a pull request',
        <Widget>[],
      ),
      BridgePullRequestOutcomeKind.notGitHub when outcome.page.isNotEmpty => (
        'Create pull request',
        [
          PrimaryButton(
            label: 'Open in browser ↗',
            onPressed: () =>
                unawaited(openInBrowser(widget.state, outcome.page)),
          ),
          checkAgain,
        ],
      ),
      _ => ('Create pull request', [checkAgain]),
    };
    return ActionDialog(
      title: title,
      actions: actions,
      children: [
        Text(problem, style: text),
        if (outcome.kind == BridgePullRequestOutcomeKind.needsPush &&
            uncommitted > 0)
          Text(
            '${plural(uncommitted, 'uncommitted file')} ${uncommitted == 1 ? 'is' : 'are'} not part of the pull request. Commit them first if you want to include them.',
            style: text,
          ),
      ],
    );
  }

  Widget _form(BuildContext context, BridgePullRequestPlan plan) {
    final ws = context.ws;
    final text = DangerDialog.text(context);
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    final fork = plan.targets.length > 1;
    // Targets and the head repository are owner/name values GitHub validated, which show as
    // they are; the head branch shows only as `plan.head`.
    final target = plan.target;
    final base = plan.base;
    final remote = widget.state.git.status.value?.remote ?? 'the remote';
    Widget row(String label, Widget value, [String? hint]) => Row(
      spacing: WsMetrics.space3,
      children: [
        SizedBox(width: 64, child: Text(label, style: faint)),
        Flexible(child: value),
        if (hint != null) Text(hint, style: faint),
      ],
    );
    Widget mono(String value) => Text(
      value,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: monoStyle(context, color: ws.text),
    );
    Widget chooser(String label, String value, VoidCallback? onTap) =>
        Semantics(
          button: true,
          enabled: onTap != null,
          label: '$label: $value',
          excludeSemantics: true,
          onTap: onTap,
          child: InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
            onTap: onTap,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              spacing: WsMetrics.space1,
              children: [
                Flexible(child: mono(value)),
                Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
              ],
            ),
          ),
        );
    final comparison = plan.comparison;
    final compared = switch ((base, comparison)) {
      _ when plan.baseMissing =>
        '⚠ ${base?.shown} does not exist in $target on GitHub. Choose another base.',
      (_?, _) when _sameBranch =>
        '⚠ A pull request cannot merge ${base!.shown} into itself. Create a branch or choose a different base.',
      (_?, _) when plan.comparisonFailed =>
        '⚠ GitHub could not compare ${plan.head} with ${base!.shown}. Check again, or choose another base.',
      (_?, final comparison?) when comparison.commits == 0 =>
        '⚠ ${plan.head} has no commits beyond ${base!.shown}. Choose another base or commit a change.',
      (_, final comparison?) =>
        '${plural(comparison.commits.toInt(), 'pushed commit')} · ${plural(comparison.files.toInt(), 'changed file')} · +${comparison.added} −${comparison.removed}',
      _ => null,
    };
    final baseHint = base == null
        ? null
        : base.name == _recorded
        ? 'created from ${base.shown}'
        : base.name == plan.defaultBranch?.name
        ? "GitHub's default"
        : null;
    void retarget() async {
      final chosen = await showDialog<String>(
        context: context,
        builder: (context) => _TargetChoice(plan: plan),
      );
      if (chosen != null && chosen != target && mounted) {
        await _read(target: chosen, base: base?.name);
      }
    }

    final enter = !_bodyFocus.hasFocus;
    return ActionDialog(
      title: 'Create pull request',
      bindings: {
        if (_ready && enter)
          const Keys(LogicalKeyboardKey.enter).activator: () =>
              unawaited(_create()),
      },
      actions: [
        PrimaryButton(
          label: _draft ? 'Create draft pull request' : 'Create pull request',
          shortcut: enter ? '⏎' : null,
          onPressed: _ready ? _create : null,
        ),
        if (_unknown || _stale || plan.comparisonFailed)
          SecondaryButton(
            label: 'Check again',
            onPressed: _reading || _creating ? null : _checkAgain,
          ),
      ],
      children: [
        if (fork || plan.parentUnknown)
          row(
            'Target',
            chooser(
              'Target',
              target ?? 'Choose a repository…',
              _reading ? null : retarget,
            ),
            target != null && fork && target == plan.targets.first
                ? 'parent of ${plan.identity.headRepository}'
                : null,
          )
        else
          row('GitHub', mono(target ?? ''), widget.device),
        row('Head', mono(plan.head)),
        row(
          'Base',
          chooser(
            'Base',
            base?.shown ?? 'Choose a branch…',
            _reading || target == null ? null : _chooseBase,
          ),
          baseHint,
        ),
        if (plan.parentUnknown)
          const WarningNotice(
            "Could not read the fork's parent from GitHub, so only the fork itself can be targeted. Choose it as the target; Liquid does not guess the parent.",
          )
        else if (fork)
          Text(
            '$remote is the fork ${plan.identity.headRepository}; GitHub names ${plan.targets.first} as its parent. Pushes still go only to $remote.',
            style: text,
          ),
        if (_reading)
          Text('Comparing on GitHub…', style: faint)
        else if (compared != null)
          Row(
            spacing: WsMetrics.space3,
            children: [
              Flexible(child: Text(compared, style: text)),
              if (comparison != null && comparison.url.startsWith('https://'))
                Semantics(
                  button: true,
                  child: InkWell(
                    onTap: () =>
                        unawaited(openInBrowser(widget.state, comparison.url)),
                    child: Text(
                      'Compare on GitHub ↗',
                      style: text.copyWith(color: ws.accent),
                    ),
                  ),
                ),
            ],
          ),
        TextField(
          key: const Key('pull-request-title'),
          controller: _title,
          focusNode: _titleFocus,
          enabled: !_creating,
          onSubmitted: (_) => unawaited(_create()),
          style: const TextStyle(fontSize: WsMetrics.fsCompact),
          decoration: const InputDecoration(
            labelText: 'Title',
            hintText: 'Describe the change',
          ),
        ),
        TextField(
          key: const Key('pull-request-description'),
          controller: _body,
          focusNode: _bodyFocus,
          enabled: !_creating,
          minLines: 3,
          maxLines: 8,
          textInputAction: TextInputAction.newline,
          style: const TextStyle(fontSize: WsMetrics.fsCompact),
          decoration: const InputDecoration(
            labelText: 'Description',
            alignLabelWithHint: true,
          ),
        ),
        ?_textNote(context, plan, faint),
        if (plan.uncommitted > 0)
          Text(
            '${plural(plan.uncommitted.toInt(), 'uncommitted file')} ${plan.uncommitted == 1 ? 'is' : 'are'} not part of the pull request. Commit them first if you want to include them.',
            style: faint,
          ),
        InkWell(
          onTap: _creating ? null : () => setState(() => _draft = !_draft),
          child: Row(
            children: [
              Checkbox(
                value: _draft,
                semanticLabel: 'Create as draft',
                onChanged: _creating
                    ? null
                    : (value) => setState(() => _draft = value ?? false),
              ),
              ExcludeSemantics(child: Text('Create as draft', style: text)),
            ],
          ),
        ),
        if (_changed)
          WarningNotice(
            '${plan.head} changed since this dialog opened. Review the updated commits before creating the pull request.',
          ),
        if (_error case final error?) WarningNotice(error),
        if (_creating)
          Text(
            'Creating pull request… $_seconds s · Create is disabled while this request runs.',
            style: faint,
          )
        else if (base == null || _title.text.trim().isEmpty)
          Text(
            'Choose a base and enter a title before creating the pull request.',
            style: faint,
          )
        else
          Text(
            'Creates the pull request on GitHub through gh on ${widget.device}.',
            style: faint,
          ),
      ],
    );
  }
}

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
