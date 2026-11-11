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
import 'commit_draft.dart';
import 'git_actions.dart';
import 'pull_request_choice.dart';
import 'pull_request_refusal.dart';
import 'pull_request_writing.dart';
import 'session_menus.dart';

/// The approved "Create pull request" dialog for a pushed branch (pull request mockups §2–3),
/// or with `compose` "Commit, push & create PR", which commits the selected files, pushes and
/// then creates, stopping at the first step that fails. Its title and description start from
/// the latest commit, and the Chat's model rewrites the fields the user has not edited.
Future<void> showPullRequestDialog(
  BuildContext context,
  SessionState state, {
  required String device,
  bool compose = false,
}) async {
  final next = await showDialog<PullRequestNext>(
    context: context,
    builder: (_) =>
        _PullRequestDialog(state: state, device: device, compose: compose),
  );
  if (!context.mounted) return;
  switch (next) {
    case PullRequestNext.push:
      await pushBranch(context, state, device: device);
    case PullRequestNext.branch:
      await showBranchMenu(context, state);
    case PullRequestNext.commit:
      await showCommitDialog(context, state);
    case PullRequestNext.compose:
      await showPullRequestDialog(
        context,
        state,
        device: device,
        compose: true,
      );
    case null:
  }
}

class _PullRequestDialog extends StatefulWidget {
  const _PullRequestDialog({
    required this.state,
    required this.device,
    required this.compose,
  });

  final SessionState state;
  final String device;
  final bool compose;

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

  /// The title and description the model writes into the fields the user has not edited.
  late final PullRequestWriting _writing;

  /// "Commit, push & create PR": the uncommitted files, the selection and message, and the
  /// steps done so far, which a later run skips. Once pushed, the dialog creates as for a
  /// pushed branch.
  late final bool _compose = widget.compose;
  CommitDraft? _files;
  String? _committed;
  BridgeGitTransfer? _pushed;
  bool _running = false;

  /// What the steps did and what remains, after one failed.
  String? _result;

  PullRequests get _pulls => widget.state.pulls;

  bool get _hasFiles => _files?.plan?.files.isNotEmpty ?? false;

  bool get _loadingFiles =>
      _files != null && _files!.plan == null && !_files!.failed;

  /// Before its push, "Commit, push & create PR" plans from Liquid's checkout; after it, from
  /// GitHub.
  bool get _beforePush => _compose && _pushed == null;

  @override
  void initState() {
    super.initState();
    _title.addListener(() => setState(() {}));
    _bodyFocus.addListener(() => setState(() {}));
    _writing = PullRequestWriting(pulls: _pulls, title: _title, body: _body)
      ..addListener(() => setState(() {}));
    if (_compose && (widget.state.git.status.value?.changed ?? 0) > 0) {
      _files = CommitDraft(widget.state)..addListener(_filesChanged);
    }
    unawaited(_read());
  }

  bool _filesRead = false;

  void _filesChanged() {
    if (!mounted) return;
    setState(() {
      // The model writes once the files it describes are known.
      if (!_filesRead && _files!.plan != null) {
        _filesRead = true;
        if (_plan case final plan?) _write(plan);
      }
    });
  }

  @override
  void dispose() {
    _writing.dispose();
    _elapsed?.cancel();
    _title.dispose();
    _body.dispose();
    _files?.dispose();
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
      outcome = _beforePush
          ? await _pulls.prepare(target: target, base: base)
          : await _pulls.plan(target: target, base: base);
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
      _writing.fillFromCommit(plan.subject, plan.body);
      // The form replaces the reading state, whose dialog took the focus first.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        // The commit message comes first when there is one to write.
        if (mounted) {
          (_hasFiles || _loadingFiles ? _files!.messageFocus : _titleFocus)
              .requestFocus();
        }
      });
    }
    _plan = plan;
    _refused = null;
    _existing = plan.open;
    _write(plan);
  }

  String get _writer => _model!.substring(_model.indexOf('/') + 1);

  /// Has the Chat's model write the fields the user has not edited, once for each range of
  /// commits and, before a commit, selection of files.
  void _write(BridgePullRequestPlan plan) {
    final model = _model;
    if (_compose) {
      // After the push the text stays as the user reviewed it.
      if (!_beforePush || _running || _loadingFiles) return;
      if (model == null || plan.open != null) {
        _writing.stop();
        return;
      }
      final paths = _files?.paths ?? const <String>[];
      // Liquid's remote has the base only for the head's own repository, not a fork's parent.
      final base = plan.target == plan.identity.headRepository
          ? plan.base?.name
          : null;
      _writing.ask(
        plan,
        'draft ${base ?? ''}\n${paths.join('\n')}',
        files: paths.isNotEmpty,
        () => _pulls.writeDraft(
          model,
          base: base,
          snapshot: _files?.plan?.snapshot ?? '',
          paths: paths,
        ),
      );
      return;
    }
    final comparison = plan.comparison;
    if (model == null ||
        plan.open != null ||
        comparison == null ||
        comparison.commits == 0 ||
        comparison.mergeBase.isEmpty) {
      // Text for other commits would not fit this plan.
      _writing.stop();
      return;
    }
    _writing.ask(
      plan,
      '${comparison.mergeBase}..${plan.identity.headCommit}',
      () => _pulls.write(model, plan),
    );
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
        !_running &&
        !_writing.settling &&
        plan.target != null &&
        plan.base != null &&
        !plan.baseMissing &&
        (_beforePush
            ? _commitReady
            : plan.comparison != null && plan.comparison!.commits != 0) &&
        !_sameBranch &&
        _title.text.trim().isNotEmpty;
  }

  /// The commit step can run, is done, or is not needed; it waits while the selection is
  /// edited, as the text describes the selection that will be committed.
  bool get _commitReady {
    final files = _files;
    if (files == null || _committed != null) return true;
    if (files.plan == null) return false;
    return files.plan!.files.isEmpty || (files.ready && !files.editing);
  }

  /// Commits the selected files, then pushes, then creates as for a pushed branch; a step
  /// done in an earlier run is skipped, and a failed step stops the rest.
  Future<void> _run() async {
    if (!_ready) return;
    setState(() {
      _writing.stop();
      _running = true;
      _result = null;
    });
    try {
      if (_hasFiles && _committed == null) {
        final outcome = await _files!.commit();
        if (!mounted) return;
        switch (outcome?.kind) {
          case BridgeGitOutcomeKind.done || BridgeGitOutcomeKind.hookChanged:
            _committed = widget.state.git.status.value?.commit ?? '';
          case null:
            // Git may have committed before Liquid lost its answer.
            setState(
              () => _result = "The commit's result is unknown; check the branch before running again. Nothing was pushed and no pull request was created.",
            );
            return;
          default:
            setState(
              () => _result = 'Could not commit: nothing was committed. Nothing was pushed and no pull request was created.',
            );
            return;
        }
      }
      final pushed = await pushBranch(
        context,
        widget.state,
        device: widget.device,
        offer: false,
      );
      if (!mounted) return;
      if (pushed?.kind != BridgeGitOutcomeKind.done) {
        setState(
          () => _result =
              '$_steps✕ Not pushed${_notPushed(pushed)} · Pull request not created. Remaining: Push, then Create pull request…',
        );
        return;
      }
      _pushed = pushed!.transfer;
    } finally {
      if (mounted) setState(() => _running = false);
    }
    final plan = _plan!;
    await _read(target: plan.target, base: plan.base?.name);
    if (!mounted || _existing != null) return;
    final outcome = _ready ? await _create() : null;
    if (!mounted ||
        {
          BridgePullRequestOutcomeKind.done,
          BridgePullRequestOutcomeKind.exists,
          BridgePullRequestOutcomeKind.changed,
          BridgePullRequestOutcomeKind.unknown,
        }.contains(outcome?.kind)) {
      return;
    }
    // The pushed steps stay done; the dialog now creates as for a pushed branch.
    final problem = outcome == null
        ? _error
        : pullRequestProblem(outcome, device: widget.device);
    setState(() {
      _result =
          '$_steps✕ Pull request not created${problem == null ? '' : ': ${problem.replaceFirst(RegExp(r'\.$'), '')}'}. Remaining: Create pull request…, with the title and description kept.';
      _error = null;
    });
  }

  /// The steps done so far, as the result of a failed run names them.
  String get _steps {
    final pushed = _pushed;
    return [
      if (_committed case final commit?) '✓ Committed $commit · ',
      if (pushed != null)
        pushed.firstPush || pushed.gone
            ? '✓ Pushed ${pushed.branch} · '
            : '✓ Pushed ${plural(pushed.ahead.toInt(), 'commit')} · ',
    ].join();
  }

  /// Why the push did not happen, as a clause after "Not pushed".
  String _notPushed(BridgeGitOutcome? outcome) {
    final remote = outcome?.transfer?.remote ?? 'the remote';
    return switch (outcome?.kind) {
      BridgeGitOutcomeKind.diverged => ': $remote rejected the push',
      BridgeGitOutcomeKind.signInFailed => ': Git could not sign in to $remote',
      BridgeGitOutcomeKind.fetchFailed => ': Git could not fetch from $remote',
      BridgeGitOutcomeKind.agentRunning => ': an agent runs in this checkout',
      BridgeGitOutcomeKind.transferFailed ||
      BridgeGitOutcomeKind.preflightFailed => ': Git failed',
      _ => '',
    };
  }

  /// Creates the reviewed pull request; none when it is not ready.
  Future<BridgePullRequestOutcome?> _create() async {
    if (!_ready) return null;
    final plan = _plan!;
    setState(() {
      _writing.stop();
      _result = null;
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
    final problem = pullRequestProblem(outcome, device: widget.device);
    final error = switch (outcome.kind) {
      _ when problem == null => null,
      BridgePullRequestOutcomeKind.unknown ||
      BridgePullRequestOutcomeKind.notCreated => problem,
      _ => '$problem No pull request was created.',
    };
    // A question closed the dialog meanwhile, so the status bar says what happened.
    if (!mounted && outcome.kind != BridgePullRequestOutcomeKind.done) {
      widget.state.notify(
        error == null ? BridgeLevel.warning : BridgeLevel.error,
        error ??
            (outcome.kind == BridgePullRequestOutcomeKind.exists
                ? 'Pull request #${outcome.pullRequest!.number} already exists.'
                : '${plan.head} changed since this dialog opened. No pull request was created.'),
      );
      return outcome;
    }
    switch (outcome.kind) {
      case BridgePullRequestOutcomeKind.done:
        final pull = outcome.pullRequest!;
        widget.state.notify(
          BridgeLevel.info,
          'Created ${pull.state == BridgePullRequestState.draft ? 'draft ' : ''}pull request #${pull.number}: “${pull.title}”',
        );
        if (mounted) Navigator.of(context).pop();
      case BridgePullRequestOutcomeKind.exists:
        setState(() => _existing = outcome.pullRequest);
      case BridgePullRequestOutcomeKind.changed:
        setState(() {
          _accept(outcome.plan!);
          _changed = true;
        });
      default:
        setState(() {
          _unknown = outcome.kind == BridgePullRequestOutcomeKind.unknown;
          _error = error;
        });
    }
    return outcome;
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
    final chosen = await showBaseChoice(
      context,
      target: target,
      names: listed.bases,
      more: listed.moreBases,
      unknown: _recorded == null && plan.defaultBranch == null,
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
    if (_refused case final refused?) {
      return PullRequestRefusal(
        state: widget.state,
        device: widget.device,
        outcome: refused,
        compose: _compose,
        beforePush: _beforePush,
        composeTitle: _dialogTitle,
        reading: _reading,
        onCheckAgain: _checkAgain,
      );
    }
    return ActionDialog(
      title: _dialogTitle,
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

  /// The action the dialog opened for, which a commit it made keeps.
  String get _dialogTitle => !_compose
      ? 'Create pull request'
      : _files != null &&
            (_committed != null || _files!.plan == null || _hasFiles)
      ? 'Commit, push & create PR'
      : 'Push & create PR';

  /// Where the title and description came from (mockup §2's generating and failure states).
  Widget? _textNote(TextStyle faint) {
    if (_writing.writing) {
      return Text(
        '◌ $_writer is writing the title and description… The fields stay editable.',
        style: faint,
      );
    }
    return switch (_writing.source) {
      TextSource.commit => null,
      TextSource.written => Text(
        'Written by $_writer from ${_compose
            ? _writing.writtenFiles
                  ? _writing.written == 0
                        ? 'the files above'
                        : "the branch's commits and the files above"
                  : "the branch's ${plural(_writing.written, 'commit')}"
            : _writing.written == 1
            ? 'the commit and its diff'
            : 'the ${_writing.written} commits and their diff'}. Edit before creating.',
        style: faint,
      ),
      TextSource.failed => WarningNotice(
        "Could not write the text with $_writer. Filled from the latest commit's subject and body instead; edit before creating.",
      ),
    };
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
      final chosen = await showTargetChoice(context, plan);
      if (chosen != null && chosen != target && mounted) {
        await _read(target: chosen, base: base?.name);
      }
    }

    final enter =
        !_bodyFocus.hasFocus && !(_files?.messageFocus.hasFocus ?? false);
    final act = _beforePush ? _run : _create;
    return ActionDialog(
      title: _dialogTitle,
      bindings: {
        if (_ready && enter)
          const Keys(LogicalKeyboardKey.enter).activator: () =>
              unawaited(act()),
      },
      actions: [
        PrimaryButton(
          label: !_beforePush
              ? _draft
                    ? 'Create draft pull request'
                    : 'Create pull request'
              : _hasFiles && _committed == null
              ? 'Commit, push & create PR'
              : 'Push & create PR',
          shortcut: enter ? '⏎' : null,
          onPressed: _ready ? act : null,
        ),
        if (_unknown ||
            _stale ||
            plan.comparisonFailed ||
            (_result != null && !_beforePush))
          SecondaryButton(
            label: 'Check again',
            onPressed: _reading || _creating ? null : _checkAgain,
          ),
      ],
      children: [
        if (_compose) ..._branchPart(context, row, mono, faint),
        if (fork || plan.parentUnknown)
          row(
            'Target',
            chooser(
              'Target',
              target ?? 'Choose a repository…',
              _reading || _running || _creating ? null : retarget,
            ),
            target != null && fork && target == plan.targets.first
                ? 'parent of ${plan.identity.headRepository}'
                : null,
          )
        else
          row('GitHub', mono(target ?? ''), widget.device),
        if (!_compose) row('Head', mono(plan.head)),
        row(
          'Base',
          chooser(
            'Base',
            base?.shown ?? 'Choose a branch…',
            _reading || _running || _creating || target == null
                ? null
                : _chooseBase,
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
          onSubmitted: (_) => unawaited(act()),
          style: const TextStyle(fontSize: WsMetrics.fsCompact),
          decoration: InputDecoration(
            labelText: _compose ? 'Pull request title' : 'Title',
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
        ?_textNote(faint),
        if (plan.uncommitted > 0 && !_compose)
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
        if (_result case final result?)
          WarningNotice('⚠ $result', danger: true),
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
        else if (!_beforePush)
          Text(
            'Creates the pull request on GitHub through gh on ${widget.device}.',
            style: faint,
          ),
      ],
    );
  }

  /// "Commit, push & create PR"'s branch and what it pushes: the approved commit dialog's
  /// fields, or the commits to push when there is nothing to commit.
  List<Widget> _branchPart(
    BuildContext context,
    Widget Function(String, Widget, [String?]) row,
    Widget Function(String) mono,
    TextStyle faint,
  ) {
    final status = widget.state.git.status.value;
    final files = _files;
    if (files != null && (files.plan == null || _hasFiles)) {
      return commitDraftFields(
        context,
        files,
        trailing: Text('→ ${status?.remote ?? ''}', style: faint),
        enabled: !_running && _committed == null,
        // The text describes the files that will be committed.
        onEdited: () => _write(_plan!),
      );
    }
    return [
      Container(
        padding: const EdgeInsets.all(WsMetrics.space3),
        decoration: BoxDecoration(
          color: context.ws.subtle,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          spacing: WsMetrics.space2,
          children: [
            row(
              'Branch',
              mono(status?.branch ?? ''),
              '→ ${status?.remote ?? ''}',
            ),
            row(
              'Push',
              Text(
                status == null || status.upstream == null || status.gone
                    ? 'new branch'
                    : plural(status.ahead.toInt(), 'commit'),
              ),
              'nothing to commit',
            ),
          ],
        ),
      ),
    ];
  }
}
