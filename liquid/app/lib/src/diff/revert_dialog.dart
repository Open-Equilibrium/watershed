import 'package:flutter/material.dart';

import '../controls.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';

Future<void> confirmRevert(
  BuildContext context,
  SessionState state,
  int turn,
) async {
  if (state.status.value != RunStatus.idle) return;
  final restorable = state.turns.value.any(
    (t) => t.number == turn && t.checkpoint,
  );
  final restoreFiles = await showDialog<bool>(
    context: context,
    builder: (context) => ActionDialog(
      title: 'Edit from here?',
      actions: [
        PrimaryButton(
          label: 'Revert and keep changes',
          onPressed: () => Navigator.of(context).pop(false),
        ),
        if (restorable)
          SecondaryButton(
            label: 'Revert files too',
            color: context.ws.danger,
            onPressed: () => Navigator.of(context).pop(true),
          ),
      ],
      children: [
        Text(
          'Rewind chat to before this message. Your prompt and attachments return to the '
          'composer. “Revert files too” also undoes your own edits.',
          style: DangerDialog.text(context),
        ),
      ],
    ),
  );
  if (restoreFiles != null && context.mounted) {
    await state.revert(turn, restoreFiles: restoreFiles);
  }
}
