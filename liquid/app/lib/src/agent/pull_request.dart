import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../session/pull_requests.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'branch_menu.dart';
import 'commit_dialog.dart';
import 'git_actions.dart';

/// The approved "Create pull request" dialog for a pushed branch (pull request mockups §2–3).
/// Its title and description start from the latest commit; a model writing them is a later
/// step.
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
  bool _draft = false;

  /// The latest outcome; null while GitHub is read.
  BridgePullRequestOutcome? _outcome;

  /// The latest plan; its title and description fill the fields only once.
  BridgePullRequestPlan? _plan;
  String? _recorded;
  BridgePullRequest? _existing;
  bool _creating = false;
  int _seconds = 0;
  Timer? _elapsed;

  /// Why the last creation failed, and whether its result is unknown.
  String? _error;
  bool _unknown = false;
  bool _changed = false;
  int _reads = 0;

  PullRequests get _pulls => widget.state.pulls;

  @override
  void initState() {
    super.initState();
    _title.addListener(() => setState(() {}));
    unawaited(_read());
  }

  @override
  void dispose() {
    _elapsed?.cancel();
    _title.dispose();
    _body.dispose();
    super.dispose();
  }

  Future<void> _read({String? target, String? base}) async {
    final read = ++_reads;
    setState(() => _outcome = null);
    final BridgePullRequestOutcome outcome;
    try {
      _recorded ??= await _pulls.recordedBase();
      outcome = await _pulls.plan(target: target, base: base);
    } on Object catch (error) {
      if (mounted && read == _reads) {
        setState(() {
          _outcome = null;
          _error = 'Could not read GitHub: $error';
        });
      }
      return;
    }
    if (!mounted || read != _reads) return;
    setState(() => _accept(outcome));
  }

  void _accept(BridgePullRequestOutcome outcome) {
    _outcome = outcome;
    final plan = outcome.plan;
    if (plan == null) return;
    if (_plan == null) {
      _title.text = plan.subject;
      _body.text = plan.body;
    }
    _plan = plan;
    _existing = plan.open;
  }

  bool get _sameBranch {
    final plan = _plan!;
    return plan.base?.name == plan.identity.headBranch &&
        plan.target == plan.identity.headRepository;
  }

  bool get _ready {
    final plan = _plan;
    return plan != null &&
        _outcome != null &&
        !_creating &&
        plan.target != null &&
        plan.base != null &&
        !plan.baseMissing &&
        plan.comparison?.commits != 0 &&
        !_sameBranch &&
        _title.text.trim().isNotEmpty;
  }

  Future<void> _create() async {
    if (!_ready) return;
    final plan = _plan!;
    setState(() {
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
    final BridgePullRequestOutcome outcome;
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
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _error = 'Could not create the pull request: $error';
          _unknown = true;
        });
      }
      return;
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
          _plan = outcome.plan;
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
    List<BridgeName> names;
    try {
      names = await _pulls.bases(plan.identity.host, target);
    } on Object catch (error) {
      setState(() => _error = 'Could not list the branches of $target: $error');
      return;
    }
    if (!mounted) return;
    final chosen = await showDialog<BridgeName>(
      context: context,
      builder: (context) => _BaseChoice(target: target, names: names),
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
    final outcome = _outcome;
    final plan = _plan;
    if (plan == null) {
      if (outcome == null) {
        return ActionDialog(
          title: 'Create pull request',
          actions: const [],
          children: [
            if (_error case final error?)
              WarningNotice(error)
            else
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
      return _refusal(context, outcome);
    }
    return _form(context, plan);
  }

  Widget _refusal(BuildContext context, BridgePullRequestOutcome outcome) {
    final text = DangerDialog.text(context);
    final problem = pullRequestProblem(outcome, device: widget.device)!;
    final uncommitted = widget.state.git.status.value?.changed ?? 0;
    void next(_Next next) => Navigator.of(context).pop(next);
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
      _ => (
        'Create pull request',
        [SecondaryButton(label: 'Check again', onPressed: _read)],
      ),
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
    final reading = _outcome == null;
    final fork = plan.targets.length > 1;
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
    Widget chooser(String label, VoidCallback? onTap) => InkWell(
      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      onTap: onTap,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        spacing: WsMetrics.space1,
        children: [
          Flexible(child: mono(label)),
          Icon(LucideIcons.chevronDown, size: 13, color: ws.textFaint),
        ],
      ),
    );
    final comparison = plan.comparison;
    final head = plan.identity.headBranch;
    final compared = switch ((base, comparison)) {
      _ when plan.baseMissing =>
        '⚠ ${base?.shown} does not exist in $target on GitHub. Choose another base.',
      (_?, _) when _sameBranch =>
        '⚠ A pull request cannot merge ${base!.shown} into itself. Create a branch or choose a different base.',
      (_?, final comparison?) when comparison.commits == 0 =>
        '⚠ $head has no commits beyond ${base!.shown}. Choose another base or commit a change.',
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

    return ActionDialog(
      title: 'Create pull request',
      actions: [
        PrimaryButton(
          label: _draft ? 'Create draft pull request' : 'Create pull request',
          shortcut: '⏎',
          onPressed: _ready ? _create : null,
        ),
        if (_unknown) SecondaryButton(label: 'Check again', onPressed: _read),
      ],
      children: [
        if (fork || plan.parentUnknown)
          row(
            'Target',
            chooser(
              target ?? 'Choose a repository…',
              reading ? null : retarget,
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
            base?.shown ?? 'Choose a branch…',
            reading ? null : _chooseBase,
          ),
          baseHint,
        ),
        if (plan.parentUnknown)
          const WarningNotice(
            "Could not read the fork's parent from GitHub. Choose the target repository; Liquid does not guess it.",
          )
        else if (fork)
          Text(
            '$remote is the fork ${plan.identity.headRepository}; GitHub names ${plan.targets.first} as its parent. Pushes still go only to $remote.',
            style: text,
          ),
        if (reading)
          Text('Comparing on GitHub…', style: faint)
        else if (compared != null)
          Row(
            spacing: WsMetrics.space3,
            children: [
              Flexible(child: Text(compared, style: text)),
              if (comparison != null && comparison.url.startsWith('https://'))
                InkWell(
                  onTap: () =>
                      unawaited(openInBrowser(widget.state, comparison.url)),
                  child: Text(
                    'Compare on GitHub ↗',
                    style: text.copyWith(color: ws.accent),
                  ),
                ),
            ],
          ),
        TextField(
          key: const Key('pull-request-title'),
          controller: _title,
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
                onChanged: _creating
                    ? null
                    : (value) => setState(() => _draft = value ?? false),
              ),
              Text('Create as draft', style: text),
            ],
          ),
        ),
        if (_changed)
          WarningNotice(
            '${plan.identity.headBranch} changed since this dialog opened. Review the updated commits before creating the pull request.',
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
  const _BaseChoice({required this.target, required this.names});

  final String target;
  final List<BridgeName> names;

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

  @override
  Widget build(BuildContext context) {
    final query = _query.text.trim().toLowerCase();
    return ActionDialog(
      title: 'Base branch',
      actions: const [],
      children: [
        TextField(
          controller: _query,
          autofocus: true,
          decoration: const InputDecoration(
            hintText: 'Search branches on GitHub',
          ),
        ),
        for (final name in widget.names)
          if (name.shown.toLowerCase().contains(query))
            _ChoiceRow(
              label: name.shown,
              onTap: () => Navigator.of(context).pop(name),
            ),
        Text(
          'The base is the branch you want these commits to join. Only branches in ${widget.target} are listed.',
          style: DangerDialog.text(context),
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

/// The PR chip beside the branch chip, and its summary (pull request mockups §4).
class PullRequestChip extends StatelessWidget {
  const PullRequestChip({super.key, required this.state, required this.device});

  final SessionState state;
  final String device;

  @override
  Widget build(BuildContext context) {
    final pulls = state.pulls;
    return ListenableBuilder(
      listenable: Listenable.merge([pulls.current, pulls.failure]),
      builder: (context, _) {
        final pull = pulls.current.value;
        if (pull == null) return const SizedBox.shrink();
        final ws = context.ws;
        final (mark, color) = _mark(context, pull.state);
        void open() => unawaited(
          showDialog<void>(
            context: context,
            barrierColor: Colors.transparent,
            builder: (_) => _Summary(state: state),
          ),
        );
        return Padding(
          padding: const EdgeInsets.only(left: WsMetrics.space2),
          child: Semantics(
            key: const Key('pull-request-chip'),
            button: true,
            label: 'Pull request ${stateLabel(pull.state)} #${pull.number}',
            excludeSemantics: true,
            onTap: open,
            child: InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: open,
              child: Container(
                height: 28,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                decoration: BoxDecoration(
                  border: Border.all(color: ws.borderStrong),
                  borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  spacing: WsMetrics.space2,
                  children: [
                    Text(mark, style: TextStyle(color: color)),
                    Text(
                      '${stateLabel(pull.state)} #${pull.number}',
                      style: TextStyle(
                        color: ws.text,
                        fontSize: WsMetrics.fsCompact,
                      ),
                    ),
                    if (pulls.failure.value != null)
                      Icon(
                        LucideIcons.triangleAlert,
                        size: 13,
                        color: ws.warning,
                      ),
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

String stateLabel(BridgePullRequestState state) => switch (state) {
  BridgePullRequestState.open => 'Open',
  BridgePullRequestState.draft => 'Draft',
  BridgePullRequestState.merged => 'Merged',
  BridgePullRequestState.closed => 'Closed',
};

(String, Color) _mark(BuildContext context, BridgePullRequestState state) {
  final ws = context.ws;
  return switch (state) {
    BridgePullRequestState.open => ('●', ws.success),
    BridgePullRequestState.draft => ('◌', ws.textMuted),
    BridgePullRequestState.merged => ('✓', ws.accent),
    BridgePullRequestState.closed => ('×', ws.danger),
  };
}

/// Checks by outcome, failed first; none reported differs from passed.
String checksText(BridgePullRequest pull) {
  final parts = [
    if (pull.checksFailed > 0) '${pull.checksFailed} failed',
    if (pull.checksPending > 0) '${pull.checksPending} pending',
    if (pull.checksPassed > 0) '${pull.checksPassed} passed',
  ];
  return 'Checks: ${parts.isEmpty ? 'none reported' : parts.join(', ')}';
}

String reviewText(BridgePullRequest pull) =>
    'Review: ${switch (pull.review) {
      BridgeReviewDecision.approved => 'approved',
      BridgeReviewDecision.changesRequested => 'changes requested',
      BridgeReviewDecision.required_ => 'required',
      null when pull.state == BridgePullRequestState.draft => 'draft, not requested',
      null => 'no reviews',
    }}';

class _Summary extends StatefulWidget {
  const _Summary({required this.state});

  final SessionState state;

  @override
  State<_Summary> createState() => _SummaryState();
}

class _SummaryState extends State<_Summary> {
  @override
  void initState() {
    super.initState();
    // Status is read when the summary opens, never by polling.
    unawaited(widget.state.pulls.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final pulls = widget.state.pulls;
    final ws = context.ws;
    final text = DangerDialog.text(context);
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Align(
      alignment: Alignment.topRight,
      child: Padding(
        padding: const EdgeInsets.only(
          top: WsMetrics.rowHeader,
          right: WsMetrics.space7,
        ),
        child: Material(
          color: ws.surface,
          elevation: 4,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 380),
            child: ListenableBuilder(
              listenable: Listenable.merge([
                pulls.current,
                pulls.updated,
                pulls.failure,
              ]),
              builder: (context, _) {
                final pull = pulls.current.value;
                if (pull == null) return const SizedBox.shrink();
                final (mark, color) = _mark(context, pull.state);
                final updated = pulls.updated.value;
                final at = updated == null
                    ? null
                    : formatClockMs(updated.millisecondsSinceEpoch);
                return Padding(
                  padding: const EdgeInsets.all(WsMetrics.space4),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    spacing: WsMetrics.space2,
                    children: [
                      Row(
                        spacing: WsMetrics.space2,
                        children: [
                          Text(mark, style: TextStyle(color: color)),
                          Text(
                            '${stateLabel(pull.state)} #${pull.number}',
                            style: text.copyWith(color: ws.text),
                          ),
                          Text(pull.repository, style: faint),
                        ],
                      ),
                      Text(
                        pull.title,
                        style: text.copyWith(
                          color: ws.text,
                          fontWeight: WsMetrics.fwMedium,
                        ),
                      ),
                      Text(
                        '${pull.head} → ${pull.base}',
                        style: monoStyle(context, color: ws.textMuted),
                      ),
                      Text(switch (pull.state) {
                        BridgePullRequestState.merged =>
                          'Merged into ${pull.base} on GitHub. The branch remains until explicitly removed.',
                        BridgePullRequestState.closed =>
                          'Closed without merging. The branch stays available.',
                        _ =>
                          '${checksText(pull)} · ${reviewText(pull)}${pull.review == BridgeReviewDecision.approved ? " GitHub's merge rules still apply." : ''}',
                      }, style: text),
                      if (pulls.failure.value case final failure?)
                        WarningNotice(
                          'Status unavailable. Last known: ${stateLabel(pull.state)} #${pull.number}${at == null ? '' : ' · $at'}. $failure',
                        ),
                      SecondaryButton(
                        label: 'Open in browser ↗',
                        onPressed: () =>
                            unawaited(openInBrowser(widget.state, pull.url)),
                      ),
                      Row(
                        children: [
                          Flexible(
                            child: Text(
                              at == null
                                  ? 'Not yet updated from GitHub · '
                                  : 'Updated from GitHub at $at · ',
                              style: faint,
                            ),
                          ),
                          InkWell(
                            onTap: () => unawaited(pulls.refresh()),
                            child: Text(
                              'Refresh',
                              style: faint.copyWith(color: ws.accent),
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}

/// After a push, offers a pull request directly above the input area until dismissed or one
/// exists (pull request mockups §1).
class PushedOffer extends StatelessWidget {
  const PushedOffer({super.key, required this.state, required this.device});

  final SessionState state;
  final String device;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: state.pulls.offer,
    builder: (context, transfer, _) {
      if (transfer == null) return const SizedBox.shrink();
      final ws = context.ws;
      final text = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsSmall);
      return Padding(
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
        child: Row(
          spacing: WsMetrics.space3,
          children: [
            Flexible(
              child: Text(
                transfer.firstPush || transfer.gone
                    ? 'Pushed ${transfer.branch} to ${transfer.remote}'
                    : 'Pushed ${plural(transfer.ahead.toInt(), 'commit')} to ${transfer.upstream}',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: text,
              ),
            ),
            InkWell(
              onTap: () => unawaited(
                showPullRequestDialog(context, state, device: device),
              ),
              child: Text(
                'Create pull request…',
                style: text.copyWith(color: ws.accent),
              ),
            ),
            IconButton(
              tooltip: 'Dismiss',
              iconSize: 13,
              visualDensity: VisualDensity.compact,
              onPressed: state.pulls.dismissOffer,
              icon: const Icon(LucideIcons.x),
            ),
          ],
        ),
      );
    },
  );
}

/// Opens a GitHub page in the system's browser; only HTTPS pages, and a failure is reported.
Future<void> openInBrowser(SessionState state, String url) async {
  if (!url.startsWith('https://')) {
    state.notify(BridgeLevel.error, 'Could not open $url: not an HTTPS page.');
    return;
  }
  try {
    final result = await Process.run(Platform.isMacOS ? 'open' : 'xdg-open', [
      url,
    ]);
    if (result.exitCode != 0) {
      state.notify(BridgeLevel.error, 'Could not open $url: ${result.stderr}');
    }
  } on ProcessException catch (error) {
    state.notify(BridgeLevel.error, 'Could not open $url: ${error.message}');
  }
}
