import 'dart:async';

import 'package:flutter/foundation.dart';
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
import 'commit_files.dart';
import 'git_actions.dart';
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
  final next = await showDialog<_Next>(
    context: context,
    builder: (_) =>
        _PullRequestDialog(state: state, device: device, compose: compose),
  );
  if (!context.mounted) return;
  switch (next) {
    case _Next.push:
      await pushBranch(context, state, device: device);
    case _Next.branch:
      await showBranchMenu(context, state);
    case _Next.commit:
      await showCommitDialog(context, state);
    case _Next.compose:
      await showPullRequestDialog(
        context,
        state,
        device: device,
        compose: true,
      );
    case null:
  }
}

/// What the dialog hands off to when the branch needs another step first.
enum _Next { push, branch, commit, compose }

/// Where the title and description came from.
enum _Text { commit, written, failed }

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
  _Text _text = _Text.commit;
  bool _writing = false;

  /// The commits the model last wrote for, as `mergeBase..head`, and how many they were.
  String? _range;
  int _written = 0;
  int _writes = 0;

  /// Runs while Create waits after the model changed the fields, so a click meant for the
  /// earlier text does not send unread text.
  Timer? _settling;

  /// What the dialog last put in the fields; a field that differs is the user's.
  String _filledTitle = '';
  String _filledBody = '';

  /// "Commit, push & create PR": the uncommitted files, the selection and message, and the
  /// steps done so far, which a later run skips. Once pushed, the dialog creates as for a
  /// pushed branch.
  late final bool _compose = widget.compose;
  BridgeCommitPlan? _files;
  bool _loadingFiles = false;
  final _selected = <String>{};
  bool _editing = false;
  Set<String> _editStart = const {};
  final _message = TextEditingController();
  final _messageFocus = FocusNode();
  String? _committed;
  BridgeGitTransfer? _pushed;
  bool _running = false;

  /// What the steps did and what remains, after one failed.
  String? _result;

  PullRequests get _pulls => widget.state.pulls;

  bool get _hasFiles => _files?.files.isNotEmpty ?? false;

  /// Before its push, "Commit, push & create PR" plans from Liquid's checkout; after it, from
  /// GitHub.
  bool get _beforePush => _compose && _pushed == null;

  List<String> get _paths => [
    for (final file in _files?.files ?? const <BridgeCommitFile>[])
      if (_selected.contains(file.change.path)) file.change.path,
  ];

  @override
  void initState() {
    super.initState();
    _title.addListener(() => setState(() {}));
    _message.addListener(() => setState(() {}));
    _bodyFocus.addListener(() => setState(() {}));
    _messageFocus.addListener(() => setState(() {}));
    if (_compose && (widget.state.git.status.value?.changed ?? 0) > 0) {
      unawaited(_readFiles());
    }
    unawaited(_read());
  }

  Future<void> _readFiles() async {
    _loadingFiles = true;
    BridgeCommitPlan? files;
    try {
      files = await widget.state.git.commitPlan();
    } on Object catch (error) {
      widget.state.notify(
        BridgeLevel.error,
        'Could not read uncommitted changes: $error',
      );
    }
    if (!mounted) return;
    setState(() {
      _loadingFiles = false;
      _files = files;
      _selected.addAll([
        for (final file in files?.files ?? const <BridgeCommitFile>[])
          file.change.path,
      ]);
      if (_plan case final plan?) _write(plan);
    });
  }

  @override
  void dispose() {
    _stopWriting();
    _settling?.cancel();
    _elapsed?.cancel();
    _title.dispose();
    _body.dispose();
    _message.dispose();
    _messageFocus.dispose();
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
      _title.text = _filledTitle = plan.subject;
      _body.text = _filledBody = plan.body;
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
  /// commits and, before a commit, selection of files.
  void _write(BridgePullRequestPlan plan) {
    final model = _model;
    if (_compose) {
      // After the push the text stays as the user reviewed it.
      if (!_beforePush || _running || _loadingFiles) return;
      if (model == null || plan.open != null) {
        _stopWriting();
        return;
      }
      final paths = _paths;
      // Liquid's remote has the base only for the head's own repository, not a fork's parent.
      final base = plan.target == plan.identity.headRepository
          ? plan.base?.name
          : null;
      _ask(
        plan,
        'draft ${base ?? ''}\n${paths.join('\n')}',
        () => _pulls.writeDraft(model, base: base, paths: paths),
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
      _stopWriting();
      return;
    }
    _ask(
      plan,
      '${comparison.mergeBase}..${plan.identity.headCommit}',
      () => _pulls.write(model, plan),
    );
  }

  /// Asks the model through `write` unless it already wrote for `range`.
  void _ask(
    BridgePullRequestPlan plan,
    String range,
    Future<BridgePullRequestText> Function() write,
  ) {
    if (range == _range) return;
    _range = range;
    final asked = ++_writes;
    _writing = true;
    unawaited(() async {
      BridgePullRequestText? text;
      try {
        text = await write();
      } on Object {
        text = null;
      }
      if (!mounted || asked != _writes) return;
      setState(() {
        _writing = false;
        _written = text?.commits.toInt() ?? 0;
        _text = text == null ? _Text.failed : _Text.written;
        // Without an answer, fields that still show earlier model text show the commit again.
        _fill(text?.title ?? plan.subject, text?.body ?? plan.body);
      });
    }());
  }

  /// Puts `title` and `body` in the fields the user has not edited.
  void _fill(String title, String body) {
    var changed = false;
    if (_title.text == _filledTitle && title != _filledTitle) {
      _title.text = _filledTitle = title;
      changed = true;
    }
    if (_body.text == _filledBody && body != _filledBody) {
      _body.text = _filledBody = body;
      changed = true;
    }
    if (!changed) return;
    _settling?.cancel();
    _settling = Timer(
      const Duration(milliseconds: 500),
      () => setState(() => _settling = null),
    );
  }

  /// Stops waiting for the model; what it would have written is dropped, and the same commits
  /// may be written for again.
  void _stopWriting() {
    if (!_writing) return;
    _writes++;
    _writing = false;
    _range = null;
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
        !_running &&
        _settling == null &&
        plan.target != null &&
        plan.base != null &&
        !plan.baseMissing &&
        (_beforePush
            ? _commitReady
            : plan.comparison != null && plan.comparison!.commits != 0) &&
        !_sameBranch &&
        _title.text.trim().isNotEmpty;
  }

  /// The commit step can run, is done, or is not needed.
  bool get _commitReady =>
      !_loadingFiles &&
      (_committed != null ||
          !_hasFiles ||
          (_selected.isNotEmpty &&
              _message.text.trim().isNotEmpty &&
              _files!.running.isEmpty &&
              widget.state.git.status.value?.merging != true));

  /// Commits the selected files, then pushes, then creates as for a pushed branch; a step
  /// done in an earlier run is skipped, and a failed step stops the rest.
  Future<void> _run() async {
    if (!_ready) return;
    setState(() {
      _stopWriting();
      _running = true;
      _result = null;
    });
    try {
      if (_hasFiles && _committed == null) {
        final outcome = await widget.state.git.commit(
          _paths,
          _message.text,
          snapshot: _files!.snapshot,
        );
        if (!mounted) return;
        if (outcome?.kind != BridgeGitOutcomeKind.done &&
            outcome?.kind != BridgeGitOutcomeKind.hookChanged) {
          setState(
            () => _result = 'Could not commit: nothing was committed. Nothing was pushed and no pull request was created.',
          );
          return;
        }
        _committed = widget.state.git.status.value?.commit ?? '';
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
    if (_ready) await _create();
    if (!mounted || _unknown || (_error == null && !_stale)) return;
    // The pushed steps stay done; the dialog now creates as for a pushed branch.
    setState(() {
      _result =
          '$_steps✕ Pull request not created: ${_error?.replaceFirst(RegExp(r'\.$'), '') ?? 'GitHub could not compare the branch'}. Remaining: Create pull request…, with the title and description kept.';
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

  String get _dialogTitle => !_compose
      ? 'Create pull request'
      : _hasFiles || _loadingFiles
      ? 'Commit, push & create PR'
      : 'Push & create PR';

  /// Where the title and description came from (mockup §2's generating and failure states).
  Widget? _textNote(TextStyle faint) {
    if (_writing) {
      return Text(
        '◌ $_writer is writing the title and description… The fields stay editable.',
        style: faint,
      );
    }
    return switch (_text) {
      _Text.commit => null,
      _Text.written => Text(
        'Written by $_writer from ${_compose
            ? _hasFiles
                  ? "the branch's commits and the files above"
                  : "the branch's ${plural(_written, 'commit')}"
            : _written == 1
            ? 'the commit and its diff'
            : 'the $_written commits and their diff'}. Edit before creating.',
        style: faint,
      ),
      _Text.failed => WarningNotice(
        "Could not write the text with $_writer. Filled from the latest commit's subject and body instead; edit before creating.",
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
        [
          PrimaryButton(label: 'Push…', onPressed: () => next(_Next.push)),
          SecondaryButton(
            label: 'Commit, push & create PR…',
            onPressed: () => next(_Next.compose),
          ),
        ],
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

    final enter = !_bodyFocus.hasFocus && !_messageFocus.hasFocus;
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
        if (_unknown || _stale || plan.comparisonFailed)
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
              _reading ? null : retarget,
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
        if (_result case final result?) WarningNotice(result),
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
  /// files and message, or the commits to push when there is nothing to commit.
  List<Widget> _branchPart(
    BuildContext context,
    Widget Function(String, Widget, [String?]) row,
    Widget Function(String) mono,
    TextStyle faint,
  ) {
    final ws = context.ws;
    final status = widget.state.git.status.value;
    final files = _files;
    final editable = !_running && _committed == null;
    return [
      Container(
        padding: const EdgeInsets.all(WsMetrics.space3),
        decoration: BoxDecoration(
          color: ws.subtle,
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
            if (_loadingFiles)
              Text('Reading uncommitted changes…', style: faint)
            else if (files != null && files.files.isNotEmpty)
              CommitFileList(
                files: files.files,
                selected: _selected,
                editing: _editing,
                enabled: editable,
                onEditing: () => setState(() {
                  if (!_editing) {
                    _editStart = {..._selected};
                  } else if (!setEquals(_editStart, _selected)) {
                    // The text describes the files that will be committed.
                    _write(_plan!);
                  }
                  _editing = !_editing;
                }),
                onSelected: (selected) => setState(
                  () => _selected
                    ..clear()
                    ..addAll(selected),
                ),
              )
            else
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
      if (files != null && files.running.isNotEmpty)
        WarningNotice(
          '${whoRuns(files.running)}. ${stopFirst(files.running)}, then commit.',
        ),
      if (_hasFiles) ...[
        const Text('Commit message'),
        TextField(
          key: const Key('commit-message'),
          controller: _message,
          focusNode: _messageFocus,
          enabled: editable,
          minLines: 2,
          maxLines: 5,
          textInputAction: TextInputAction.newline,
          style: const TextStyle(fontSize: WsMetrics.fsCompact),
          decoration: const InputDecoration(
            hintText: 'Describe the change: a short first line, details below',
          ),
        ),
      ],
    ];
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
