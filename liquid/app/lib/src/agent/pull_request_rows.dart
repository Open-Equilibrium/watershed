import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'session_card_rows.dart';
import 'session_menus.dart';

class PullRequestRows extends StatelessWidget {
  const PullRequestRows({super.key, required this.state});

  final SessionState state;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<GitState>(
    valueListenable: state.git,
    builder: (context, git, _) => ValueListenableBuilder<BridgeGitStatus?>(
      valueListenable: git.status,
      builder: (context, status, _) {
        final pr = status?.pullRequest;
        if (status == null || pr == null) return const SizedBox.shrink();
        return _PullRequestRow(
          key: ObjectKey(git),
          state: state,
          git: git,
          terms: terminologyFor(status),
          pr: pr,
        );
      },
    ),
  );
}

class _PullRequestRow extends StatefulWidget {
  const _PullRequestRow({
    super.key,
    required this.state,
    required this.git,
    required this.terms,
    required this.pr,
  });

  final SessionState state;
  final GitState git;
  final Terminology terms;
  final BridgeBranchPullRequest pr;

  @override
  State<_PullRequestRow> createState() => _PullRequestRowState();
}

class _PullRequestRowState extends State<_PullRequestRow> {
  BridgePullRequestDetail? _detail;
  Timer? _timer;

  int _attempt = 0;

  bool _busy = false;

  @override
  void initState() {
    super.initState();
    unawaited(_read());
  }

  @override
  void didUpdateWidget(_PullRequestRow old) {
    super.didUpdateWidget(old);
    final pr = widget.pr;
    if (old.pr.number != pr.number ||
        old.pr.state != pr.state ||
        old.pr.draft != pr.draft) {
      unawaited(_read());
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  Future<void> _read() async {
    _timer?.cancel();
    final attempt = ++_attempt;
    try {
      final detail = await widget.git.backend.pullRequest(widget.pr.number);
      if (mounted && attempt == _attempt) setState(() => _detail = detail);
    } on Object catch (_) {}
    if (mounted && attempt == _attempt) _schedule();
  }

  void _schedule() {
    _timer?.cancel();
    if (widget.pr.state != 'open') return;
    final checks = _detail?.checks ?? const <BridgePullRequestCheck>[];
    final waiting =
        checks.isEmpty ||
        checks.any(
          (check) =>
              check.status == 'pending' || check.status == 'action-required',
        );
    _timer = Timer(
      Duration(seconds: waiting ? 45 : 60),
      () => unawaited(_read()),
    );
  }

  Future<void> _act(String action, {String? mergeMethod}) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      await widget.git.backend.runPullRequestAction(
        widget.pr.number,
        action,
        mergeMethod: mergeMethod,
      );
      widget.state.notify(
        NoticeLevel.info,
        action == 'ready' ? 'Marked ready for review' : 'Merge requested',
      );
    } on Object catch (error) {
      widget.state.notify(
        NoticeLevel.error,
        action == 'ready'
            ? 'Could not mark this ready for review. $error'
            : 'Could not merge this ${widget.terms.singular}. $error',
      );
    }
    if (!mounted) return;
    setState(() => _busy = false);
    await _read();
  }

  Future<void> _checkOut(String prompt) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      final checked = await widget.git.backend.pullRequestWorktree(
        widget.pr.url,
      );
      await widget.state.draftFor(checked);
      widget.state.draft.value = prompt;
    } on Object catch (error) {
      widget.state.notify(
        NoticeLevel.error,
        'Could not prepare the pull request checkout. $error',
      );
    }
    if (mounted) setState(() => _busy = false);
  }

  Future<void> _confirmMerge(BridgePullRequestDetail detail) async {
    final merge = await showDialog<bool>(
      context: context,
      builder: (context) => ActionDialog(
        title: 'Merge ${widget.terms.singular}?',
        actions: [
          PrimaryButton(
            label: 'Merge',
            onPressed: () => Navigator.of(context).pop(true),
          ),
        ],
        children: [
          Text(
            'This merges #${detail.number} using ${detail.mergeMethod}.',
            style: DangerDialog.text(context),
          ),
        ],
      ),
    );
    if (merge == true) {
      await _act('merge', mergeMethod: detail.mergeMethod);
    }
  }

  _Trailing? _trailing(BridgePullRequestDetail detail) {
    final singular = widget.terms.singular;
    return switch (detail.action) {
      'resolve' => (
        label: 'Resolve',
        pending: 'Preparing...',
        tooltip:
            'Check the branch out and resolve the conflicts in a new thread',
        destructive: true,
        run: () => unawaited(_checkOut(detail.resolvePrompt)),
      ),
      'ready' => (
        label: 'Ready',
        pending: 'Marking...',
        tooltip: 'Mark this $singular as ready for review',
        destructive: false,
        run: () => unawaited(_act('ready')),
      ),
      'fix' => (
        label: 'Fix',
        pending: 'Preparing...',
        tooltip: 'Fix the failing checks in a new thread',
        destructive: true,
        run: () => unawaited(_checkOut(detail.fixPrompt)),
      ),
      'merge' => (
        label: 'Merge',
        pending: 'Merging...',
        tooltip: 'Merge this $singular (${detail.mergeMethod})',
        destructive: false,
        run: () => unawaited(_confirmMerge(detail)),
      ),
      _ => null,
    };
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final detail = _detail;
    final pr = widget.pr;
    final presentation = _presentation(
      detail?.state ?? pr.state,
      detail?.draft ?? pr.draft,
      ws,
    );
    final tooltip = detail == null
        ? '${_capital(widget.terms.singular)} #${pr.number}'
        : _tooltip(detail, presentation.label);
    final trailing = detail == null ? null : _trailing(detail);
    final showChecks =
        detail != null &&
        detail.checksState != 'none' &&
        !detail.conflicting &&
        !detail.draft;
    if (!showChecks && trailing == null) {
      return Tooltip(
        message: tooltip,
        child: CardRow(
          onTap: () => unawaited(openInBrowser(widget.state, pr.url)),
          children: [
            Icon(presentation.icon, size: 16, color: presentation.tone),
            Flexible(
              child: Text(
                '#${pr.number}: ${detail?.title ?? pr.title}',
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        ),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Tooltip(
          message: tooltip,
          child: InkWell(
            borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            hoverColor: ws.hover,
            highlightColor: Colors.transparent,
            splashColor: Colors.transparent,
            onTap: () => unawaited(openInBrowser(widget.state, pr.url)),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(10, 6, 10, 0),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 10,
                children: [
                  Padding(
                    padding: const EdgeInsets.only(top: 2),
                    child: Icon(
                      presentation.icon,
                      size: 16,
                      color: presentation.tone,
                    ),
                  ),
                  Expanded(
                    child: Text(
                      detail?.title ?? pr.title,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: WsMetrics.fsCompact,
                        fontWeight: WsMetrics.fwMedium,
                        color: ws.text,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(36, 2, 10, 6),
          child: Row(
            spacing: 4,
            children: [
              Text(
                '#${pr.number}',
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textMuted,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
              if (detail case final detail? when showChecks)
                _ChecksBadge(state: widget.state, detail: detail),
              const Spacer(),
              if (trailing case final action?)
                _TrailingButton(action: action, busy: _busy),
            ],
          ),
        ),
      ],
    );
  }
}

typedef _Trailing = ({
  String label,
  String pending,
  String tooltip,
  bool destructive,
  VoidCallback run,
});

class _TrailingButton extends StatelessWidget {
  const _TrailingButton({required this.action, required this.busy});

  final _Trailing action;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final tone = action.destructive ? ws.danger : ws.accent;
    return Tooltip(
      message: action.tooltip,
      child: InkWell(
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        onTap: busy ? null : action.run,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          child: Text(
            busy ? action.pending : action.label,
            style: TextStyle(
              fontSize: WsMetrics.fsSmall,
              fontWeight: WsMetrics.fwMedium,
              color: busy ? ws.textFaint : tone,
            ),
          ),
        ),
      ),
    );
  }
}

class _ChecksBadge extends StatelessWidget {
  const _ChecksBadge({required this.state, required this.detail});

  final SessionState state;
  final BridgePullRequestDetail detail;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final checks = detail.checks;
    final incomplete = checks
        .where(
          (check) =>
              check.status == 'pending' || check.status == 'action-required',
        )
        .length;
    final failed = checks
        .where(
          (check) => check.status == 'failure' || check.status == 'cancelled',
        )
        .length;
    final count = incomplete > 0
        ? '${checks.length - incomplete}/${checks.length} checks'
        : failed > 0
        ? '$failed failed'
        : null;
    final (icon, tone) = _rollupIcon(detail.checksState, ws);
    return Semantics(
      button: true,
      label: 'Open checks: ${detail.checksSummary}',
      excludeSemantics: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        onTap: () => unawaited(_showChecks(context)),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          spacing: 4,
          children: [
            Icon(icon, size: 14, color: tone),
            if (count case final count?)
              Text(
                count,
                style: TextStyle(
                  fontSize: WsMetrics.fsSmall,
                  color: ws.textMuted,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
          ],
        ),
      ),
    );
  }

  Future<void> _showChecks(BuildContext context) async {
    final badge = context.findRenderObject()! as RenderBox;
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final at = badge.localToGlobal(Offset.zero, ancestor: overlay);
    await showMenu<void>(
      context: context,
      position: RelativeRect.fromRect(
        at & badge.size,
        Offset.zero & overlay.size,
      ),
      items: [
        PopupMenuItem<void>(
          enabled: false,
          child: _ChecksBody(state: state, detail: detail),
        ),
      ],
    );
  }
}

class _ChecksBody extends StatefulWidget {
  const _ChecksBody({required this.state, required this.detail});

  final SessionState state;
  final BridgePullRequestDetail detail;

  @override
  State<_ChecksBody> createState() => _ChecksBodyState();
}

class _ChecksBodyState extends State<_ChecksBody> {
  bool _showAll = false;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final detail = widget.detail;
    bool finished(BridgePullRequestCheck check) =>
        check.status == 'success' ||
        check.status == 'skipped' ||
        check.status == 'neutral';
    final unfinished = [
      for (final check in detail.checks)
        if (!finished(check)) check,
    ];
    final done = [
      for (final check in detail.checks)
        if (finished(check)) check,
    ];
    final collapsible = unfinished.isNotEmpty && done.isNotEmpty;
    return SizedBox(
      width: 360,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            _rollupLabel(detail.checksState),
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontWeight: WsMetrics.fwMedium,
              color: ws.text,
            ),
          ),
          if (detail.checksSummary.isNotEmpty)
            Text(
              detail.checksSummary,
              style: TextStyle(
                fontSize: WsMetrics.fsSmall,
                color: ws.textMuted,
              ),
            ),
          const SizedBox(height: WsMetrics.space2),
          for (final check in [
            ...unfinished,
            if (_showAll || !collapsible) ...done,
          ])
            _CheckRow(state: widget.state, check: check),
          if (collapsible)
            InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
              onTap: () => setState(() => _showAll = !_showAll),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: WsMetrics.space2),
                child: Text(
                  _showAll ? 'Show less' : 'Show all',
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    fontWeight: WsMetrics.fwMedium,
                    color: ws.accent,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _CheckRow extends StatelessWidget {
  const _CheckRow({required this.state, required this.check});

  final SessionState state;
  final BridgePullRequestCheck check;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final (icon, tone) = _checkIcon(check.status, ws);
    final url = check.url;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        spacing: 8,
        children: [
          Icon(icon, size: 14, color: tone),
          Expanded(
            child: Tooltip(
              message: check.description ?? check.name,
              child: Text(
                check.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(fontSize: WsMetrics.fsCompact, color: ws.text),
              ),
            ),
          ),
          Text(
            _checkLabel(check),
            style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.textMuted),
          ),
          if (url case final link?)
            InkWell(
              onTap: () => unawaited(openInBrowser(state, link)),
              child: Text(
                'Details',
                style: TextStyle(fontSize: WsMetrics.fsSmall, color: ws.accent),
              ),
            ),
        ],
      ),
    );
  }
}

({String label, IconData icon, Color tone}) _presentation(
  String state,
  bool draft,
  WsPalette ws,
) {
  if (state == 'open' && draft) {
    return (
      label: 'Draft',
      icon: LucideIcons.gitPullRequestDraft,
      tone: ws.textMuted,
    );
  }
  return switch (state) {
    'open' => (
      label: 'Open',
      icon: LucideIcons.gitPullRequestArrow,
      tone: ws.success,
    ),
    'closed' => (
      label: 'Closed',
      icon: LucideIcons.gitPullRequestClosed,
      tone: ws.danger,
    ),
    _ => (label: 'Merged', icon: LucideIcons.gitMerge, tone: ws.accent),
  };
}

(IconData, Color) _rollupIcon(String checksState, WsPalette ws) =>
    switch (checksState) {
      'failing' => (LucideIcons.circleX, ws.danger),
      'pending' => (LucideIcons.circleDot, ws.warning),
      _ => (LucideIcons.checkCircle, ws.success),
    };

String _rollupLabel(String checksState) => switch (checksState) {
  'failing' => 'Some checks were not successful',
  'pending' => "Some checks haven't completed yet",
  _ => 'All checks have passed',
};

(IconData, Color) _checkIcon(String status, WsPalette ws) => switch (status) {
  'pending' => (LucideIcons.loader, ws.warning),
  'action-required' => (LucideIcons.circleDot, ws.warning),
  'success' => (LucideIcons.checkCircle, ws.success),
  'failure' || 'cancelled' => (LucideIcons.circleX, ws.danger),
  'skipped' => (LucideIcons.circleSlash, ws.textFaint),
  _ => (LucideIcons.circleDashed, ws.textFaint),
};

String _checkLabel(BridgePullRequestCheck check) {
  if (check.status == 'action-required' &&
      RegExp(r'/actions/runs/\d+(/|$)').hasMatch(check.url ?? '')) {
    return 'Awaiting approval';
  }
  return switch (check.status) {
    'pending' => 'Running',
    'action-required' => 'Awaiting action',
    'success' => 'Passed',
    'failure' => 'Failed',
    'cancelled' => 'Cancelled',
    'skipped' => 'Skipped',
    _ => 'Neutral',
  };
}

String _tooltip(BridgePullRequestDetail detail, String stateLabel) => [
  '${detail.title} #${detail.number}',
  stateLabel,
  '${detail.base} ← ${detail.head}',
  if (detail.state == 'open' && detail.checksState != 'none')
    detail.checksSummary,
  if (detail.draft && detail.conflicting) 'Merge conflicts with ${detail.base}',
  '${detail.files} ${detail.files == 1 ? 'file' : 'files'} '
      '+${detail.added} −${detail.removed}',
].join('\n');

String _capital(String text) => '${text[0].toUpperCase()}${text.substring(1)}';
