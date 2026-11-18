import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/moves.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

class MovingNotice extends StatelessWidget {
  const MovingNotice({super.key, required this.state, required this.move});

  final SessionState state;
  final BridgeMoving move;

  Future<void> _confirmCancel(BuildContext context) async {
    final cancel = await showDialog<bool>(
      context: context,
      builder: (dialogContext) => ActionDialog(
        title: 'Cancel the move to ${move.targetName}?',
        actions: [
          SecondaryButton(
            label: 'Keep waiting',
            onPressed: () => Navigator.of(dialogContext).pop(false),
          ),
          DangerButton(
            label: 'Cancel move',
            onPressed: () => Navigator.of(dialogContext).pop(true),
          ),
        ],
        children: [
          Text(
            'The session continues here. If ${move.targetName} already started it, it runs on both hosts until you delete one of them.',
          ),
        ],
      ),
    );
    if (cancel == true) await state.cancelMove();
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final target = move.targetName;
    final waits = state.moveWaits.value;
    final waitingFor = waits?.transfer == move.transfer
        ? waits?.waitingFor
        : null;
    return Padding(
      padding: const EdgeInsets.all(WsMetrics.space4),
      child: state.runningMoves.contains(move.transfer)
          ? Row(
              children: [
                const SizedBox.square(
                  dimension: 14,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
                const SizedBox(width: WsMetrics.space3),
                Text(
                  'Moving to $target…',
                  style: TextStyle(color: ws.textMuted),
                ),
              ],
            )
          : Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: WsMetrics.space2,
              children: [
                Text(
                  'Moving to $target.',
                  style: TextStyle(
                    color: ws.text,
                    fontWeight: WsMetrics.fwMedium,
                  ),
                ),
                Text(
                  waitingFor == null
                      ? 'Liquid waits five minutes after the move started before it finishes it.'
                      : '$waitingFor does not answer, so this session waits for it.',
                  style: TextStyle(color: ws.textMuted),
                ),
                Row(
                  children: [
                    SecondaryButton(
                      label: 'Try again',
                      onPressed: state.retryMove,
                    ),
                    const SizedBox(width: WsMetrics.space3),
                    GhostButton(
                      label: 'Cancel move…',
                      onPressed: () => unawaited(_confirmCancel(context)),
                    ),
                  ],
                ),
              ],
            ),
    );
  }
}
