import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/git.dart';
import '../session/pull_requests.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'pull_request.dart';
import 'session_menus.dart';

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
        final stale = pulls.failure.value != null;
        void open() {
          final box = context.findRenderObject()! as RenderBox;
          final chip = box.localToGlobal(Offset.zero) & box.size;
          unawaited(
            showDialog<void>(
              context: context,
              barrierColor: Colors.transparent,
              builder: (_) =>
                  _Summary(state: state, device: device, below: chip),
            ),
          );
        }

        return Padding(
          padding: const EdgeInsets.only(left: WsMetrics.space2),
          child: Semantics(
            key: const Key('pull-request-chip'),
            button: true,
            label:
                'Pull request ${stateLabel(pull.state)} #${pull.number}${stale ? ', status unavailable' : ''}',
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
                    if (stale)
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
  const _Summary({
    required this.state,
    required this.device,
    required this.below,
  });

  final SessionState state;
  final String device;

  /// The chip the summary opens below, in global coordinates.
  final Rect below;

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
    final width = MediaQuery.sizeOf(context).width;
    return Stack(
      children: [
        Positioned(
          top: widget.below.bottom + WsMetrics.space1,
          right: (width - widget.below.right).clamp(0, width),
          child: Container(
            constraints: const BoxConstraints(maxWidth: 380),
            decoration: BoxDecoration(
              color: ws.surface,
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              boxShadow: ws.shadow,
            ),
            child: Material(
              type: MaterialType.transparency,
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
                  final failure = switch (pulls.failure.value) {
                    final failed? => pullRequestProblem(
                      failed,
                      device: widget.device,
                    ),
                    null => null,
                  };
                  return Padding(
                    padding: const EdgeInsets.all(WsMetrics.space4),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.start,
                      spacing: WsMetrics.space2,
                      children: [
                        Row(
                          mainAxisSize: MainAxisSize.min,
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
                          BridgePullRequestState.closed => 'Closed without merging. The branch stays available.',
                          _ =>
                            '${checksText(pull)} · ${reviewText(pull)}${pull.review == BridgeReviewDecision.approved ? " GitHub's merge rules still apply." : ''}',
                        }, style: text),
                        if (failure != null)
                          WarningNotice(
                            'Status unavailable. Last known: ${stateLabel(pull.state)} #${pull.number}${at == null ? '' : ' · $at'}. $failure',
                          ),
                        SecondaryButton(
                          label: 'Open in browser ↗',
                          onPressed: () =>
                              unawaited(openInBrowser(widget.state, pull.url)),
                        ),
                        Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Flexible(
                              child: Text(
                                at == null
                                    ? 'Not yet updated from GitHub · '
                                    : 'Updated from GitHub at $at · ',
                                style: faint,
                              ),
                            ),
                            Semantics(
                              button: true,
                              child: InkWell(
                                onTap: () => unawaited(pulls.refresh()),
                                child: Text(
                                  'Refresh',
                                  style: faint.copyWith(color: ws.accent),
                                ),
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
      ],
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
      return Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: WsMetrics.widthReading),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
            child: Row(
              spacing: WsMetrics.space3,
              children: [
                Expanded(
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: WsMetrics.space4,
                      vertical: WsMetrics.space3,
                    ),
                    decoration: BoxDecoration(
                      color: ws.subtle,
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    ),
                    child: Row(
                      spacing: WsMetrics.space2,
                      children: [
                        Icon(LucideIcons.info, size: 13, color: ws.textMuted),
                        Flexible(
                          child: Text(
                            transfer.firstPush || transfer.gone
                                ? 'Pushed ${transfer.branch} to ${transfer.remote}'
                                : 'Pushed ${plural(transfer.ahead.toInt(), 'commit')} to ${transfer.upstream}',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              color: ws.textMuted,
                              fontSize: WsMetrics.fsCompact,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
                PrimaryButton(
                  label: 'Create pull request…',
                  onPressed: () => unawaited(
                    showPullRequestDialog(context, state, device: device),
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
          ),
        ),
      );
    },
  );
}
