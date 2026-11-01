import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../rust/api/history.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// Asks before reverting the files to before `turn` and then reverts them, so the session
/// continues on a new branch (approved checkpoint mockups). The dialog lists every file that
/// changes and refuses while an agent runs in the checkout.
Future<void> confirmRevert(
  BuildContext context,
  SessionState state,
  BridgeTurn turn,
) async {
  final BridgeRevertPlan plan;
  try {
    plan = await state.revertPlan(turn);
  } on Object catch (error) {
    state.notify(
      BridgeLevel.error,
      'Could not read the files to revert: $error',
    );
    return;
  }
  if (!context.mounted) return;
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => _RevertDialog(turn: turn, plan: plan),
  );
  if (confirmed == true) await state.revert(turn);
}

class _RevertDialog extends StatelessWidget {
  const _RevertDialog({required this.turn, required this.plan});

  final BridgeTurn turn;
  final BridgeRevertPlan plan;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final name = 'turn ${turn.number}';
    final muted = TextStyle(
      color: ws.textMuted,
      fontSize: WsMetrics.fsCompact,
      height: WsMetrics.lhCompact / WsMetrics.fsCompact,
    );
    final files = plan.files;
    final shown = plan.state == BridgeChangesState.shown;
    final allowed = shown && plan.running.isEmpty;
    void revert() => Navigator.of(context).pop(true);
    final lead = switch (plan.state) {
      BridgeChangesState.outsideGit => 'This project has no Git repository, so it has no checkpoints to revert to.',
      BridgeChangesState.removed =>
        'This turn’s checkpoint was removed, so its files cannot be reverted.',
      BridgeChangesState.shown => switch (files.length) {
        0 => 'No file differs from how it was when $name started.',
        1 => 'This file reverts to how it was when $name started.',
        final count =>
          'These $count files revert to how they were when $name started.',
      },
    };
    final branch = plan.branch;
    return CallbackShortcuts(
      bindings: {
        if (allowed) const Keys(LogicalKeyboardKey.enter).activator: revert,
      },
      child: Focus(
        autofocus: true,
        child: Dialog(
          backgroundColor: ws.surface,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
          ),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 520),
            child: Padding(
              padding: const EdgeInsets.all(WsMetrics.space5),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                spacing: WsMetrics.space4,
                children: [
                  Text(
                    'Revert to before $name?',
                    style: Theme.of(context).textTheme.titleSmall
                        ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                  ),
                  Text(lead, style: muted),
                  if (files.isNotEmpty) _Files(turn: turn, files: files),
                  if (shown && plan.running.isNotEmpty)
                    _Running(running: plan.running)
                  else if (shown) ...[
                    Text.rich(
                      TextSpan(
                        children: [
                          if (branch == null)
                            const TextSpan(
                              text: 'Commits, the checked-out commit',
                            )
                          else ...[
                            const TextSpan(text: 'Commits, the branch '),
                            TextSpan(
                              text: branch,
                              style: monoStyle(context, color: ws.text),
                            ),
                          ],
                          const TextSpan(
                            text: ' and files Git ignores stay as they are.',
                          ),
                        ],
                      ),
                      style: muted,
                    ),
                    Text(
                      'The session continues on a new branch from before $name, and '
                      '$name’s prompt returns to the input area. Turn ${turn.number} stays '
                      'on its own branch in the session tree.',
                      style: muted,
                    ),
                  ],
                  Row(
                    spacing: WsMetrics.space3,
                    children: [
                      if (shown)
                        DangerButton(
                          label: 'Revert and branch',
                          shortcut: allowed ? '⏎' : null,
                          onPressed: allowed ? revert : null,
                        ),
                      const Spacer(),
                      SecondaryButton(
                        label: 'Cancel',
                        shortcut: 'esc',
                        onPressed: () => Navigator.of(context).pop(false),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The files the revert changes, each with what happens to it.
class _Files extends StatelessWidget {
  const _Files({required this.turn, required this.files});

  final BridgeTurn turn;
  final List<BridgeRevertFile> files;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final started = turn.startedMs;
    final back = started == null ? 'back' : 'back to ${formatClockMs(started)}';
    final faint = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    Widget row(BridgeRevertFile file) {
      final (letter, color, outcome) = switch (file.status) {
        BridgeChangeStatus.modified => ('M', ws.warning, back),
        BridgeChangeStatus.added => (
          'A',
          ws.success,
          'deleted since then, restored',
        ),
        BridgeChangeStatus.deleted => (
          'D',
          ws.danger,
          file.ignored
              ? 'ignored by Git, in the way, deleted'
              : 'new since then, deleted',
        ),
      };
      final edit = switch ((file.edited, file.modifiedMs)) {
        (false, _) => null,
        (true, final at?) => 'also your edit at ${formatClockMs(at)}',
        (true, null) => 'also your edit',
      };
      return SizedBox(
        height: 26,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
          child: Row(
            spacing: WsMetrics.space3,
            children: [
              SizedBox(
                width: 14,
                child: Text(
                  letter,
                  style: TextStyle(
                    color: color,
                    fontSize: WsMetrics.fsSmall,
                    fontWeight: WsMetrics.fwSemibold,
                  ),
                ),
              ),
              Expanded(
                flex: 3,
                child: Text(
                  file.shown,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: monoStyle(context),
                ),
              ),
              Flexible(
                flex: 2,
                child: Text(
                  edit == null ? outcome : '$outcome · $edit',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.end,
                  style: faint,
                ),
              ),
            ],
          ),
        ),
      );
    }

    return Container(
      constraints: const BoxConstraints(maxHeight: 240),
      padding: const EdgeInsets.all(WsMetrics.space2),
      decoration: BoxDecoration(
        color: ws.subtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: ListView(
        shrinkWrap: true,
        children: [for (final file in files) row(file)],
      ),
    );
  }
}

/// Says which agent holds the revert up.
class _Running extends StatelessWidget {
  const _Running({required this.running});

  final List<BridgeRunningAgent> running;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final first = running.first;
    final name = switch ((first.own, first.title)) {
      (true, _) => 'This session’s agent',
      (false, final title?) => '“$title”',
      (false, null) => 'Another agent session',
    };
    final more = running.length - 1;
    final text = more == 0
        ? '$name is running${first.own ? '' : ' in this checkout'}. Stop it or wait until '
              'it finishes, then revert the files.'
        : '$name and $more more are running in this checkout. Stop them or wait until they '
              'finish, then revert the files.';
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: ws.warningSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Text(
        text,
        style: TextStyle(color: ws.warning, fontSize: WsMetrics.fsCompact),
      ),
    );
  }
}
