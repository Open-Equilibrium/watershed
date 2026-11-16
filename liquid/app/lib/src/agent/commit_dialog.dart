import 'dart:async';

import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../session/session_state.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// T3 Code's "Commit changes" dialog: the checkout's uncommitted files, with those to leave out,
/// and an optional message. Committing closes it and runs the commit.
Future<void> showCommitDialog(BuildContext context, SessionState state) {
  final git = state.git.value;
  return showDialog<void>(
    context: context,
    builder: (_) => _CommitDialog(git: git),
  );
}

class _CommitDialog extends StatefulWidget {
  const _CommitDialog({required this.git});

  final GitState git;

  @override
  State<_CommitDialog> createState() => _CommitDialogState();
}

class _CommitDialogState extends State<_CommitDialog> {
  final _message = TextEditingController();
  final _excluded = <String>{};
  bool _editing = false;

  @override
  void dispose() {
    _message.dispose();
    super.dispose();
  }

  /// Commits the files not left out; the whole set passes no file list.
  void _commit({bool onNewBranch = false}) {
    final files = widget.git.status.value?.changes ?? const [];
    final chosen = [
      for (final file in files)
        if (!_excluded.contains(file.path)) file.path,
    ];
    final text = _message.text.trim();
    Navigator.of(context).pop();
    unawaited(
      widget.git.run(
        BridgeGitAction.commit,
        message: text.isEmpty ? null : text,
        files: chosen.length == files.length ? null : chosen,
        featureBranch: onNewBranch,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.git.status,
    builder: (context, _) {
      final ws = context.ws;
      final status = widget.git.status.value;
      final files = status?.changes ?? const <BridgeFileChange>[];
      final chosen = [
        for (final file in files)
          if (!_excluded.contains(file.path)) file,
      ];
      final all = chosen.length == files.length;
      final none = chosen.isEmpty;
      final muted = TextStyle(
        color: ws.textMuted,
        fontSize: WsMetrics.fsCompact,
      );
      return ActionDialog(
        title: 'Commit changes',
        actions: [
          PrimaryButton(
            label: 'Commit',
            onPressed: none ? null : () => _commit(),
          ),
          SecondaryButton(
            label: 'Commit on new branch',
            onPressed: none ? null : () => _commit(onNewBranch: true),
          ),
        ],
        children: [
          Text(
            'Review and confirm your commit. Leave the message blank to auto-generate one.',
            style: DangerDialog.text(context),
          ),
          Container(
            padding: const EdgeInsets.all(WsMetrics.space4),
            decoration: BoxDecoration(
              color: ws.subtle,
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: WsMetrics.space3,
              children: [
                Row(
                  spacing: WsMetrics.space4,
                  children: [
                    Text('Branch', style: muted),
                    Expanded(
                      child: Text(
                        status?.branch ?? '(detached HEAD)',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(
                          color: ws.text,
                          fontSize: WsMetrics.fsCompact,
                          fontWeight: WsMetrics.fwMedium,
                        ),
                      ),
                    ),
                    if (status?.defaultBranch ?? false)
                      Text(
                        'Default branch',
                        style: TextStyle(
                          color: ws.warning,
                          fontSize: WsMetrics.fsSmall,
                        ),
                      ),
                  ],
                ),
                Row(
                  spacing: WsMetrics.space2,
                  children: [
                    if (_editing && files.isNotEmpty)
                      Checkbox(
                        tristate: true,
                        visualDensity: VisualDensity.compact,
                        materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        value: none ? false : (all ? true : null),
                        onChanged: (_) => setState(() {
                          if (all) {
                            _excluded.addAll(files.map((file) => file.path));
                          } else {
                            _excluded.clear();
                          }
                        }),
                      ),
                    Text('Files', style: muted),
                    if (!all && !_editing)
                      Text(
                        '(${chosen.length} of ${files.length})',
                        style: muted,
                      ),
                    const Spacer(),
                    if (files.isNotEmpty)
                      GhostButton(
                        label: _editing ? 'Done' : 'Edit',
                        onPressed: () => setState(() => _editing = !_editing),
                      ),
                  ],
                ),
                if (files.isEmpty)
                  Text('none', style: TextStyle(fontSize: WsMetrics.fsCompact))
                else
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Container(
                        height: 176,
                        decoration: BoxDecoration(
                          color: ws.surface,
                          borderRadius: BorderRadius.circular(
                            WsMetrics.radiusMd,
                          ),
                        ),
                        child: ListView(
                          padding: const EdgeInsets.all(WsMetrics.space2),
                          children: [
                            for (final file in files) _row(context, file),
                          ],
                        ),
                      ),
                      Align(
                        alignment: Alignment.centerRight,
                        child: Text.rich(
                          TextSpan(
                            children: [
                              TextSpan(
                                text:
                                    '+${chosen.fold(0, (sum, file) => sum + file.added)}',
                                style: TextStyle(color: ws.diffAddStrong),
                              ),
                              TextSpan(text: ' / ', style: muted),
                              TextSpan(
                                text:
                                    '-${chosen.fold(0, (sum, file) => sum + file.removed)}',
                                style: TextStyle(color: ws.diffDelStrong),
                              ),
                            ],
                          ),
                          style: monoStyle(context),
                        ),
                      ),
                    ],
                  ),
              ],
            ),
          ),
          Text(
            'Commit message (optional)',
            style: TextStyle(
              fontSize: WsMetrics.fsCompact,
              fontWeight: WsMetrics.fwMedium,
              color: ws.text,
            ),
          ),
          TextField(
            controller: _message,
            minLines: 1,
            maxLines: 4,
            decoration: const InputDecoration(
              isDense: true,
              hintText: 'Leave empty to auto-generate',
            ),
          ),
        ],
      );
    },
  );

  /// One file: its path, and its added and removed lines or Excluded.
  Widget _row(BuildContext context, BridgeFileChange file) {
    final ws = context.ws;
    final excluded = _excluded.contains(file.path);
    final muted = TextStyle(color: ws.textMuted, fontSize: WsMetrics.fsCompact);
    return Row(
      spacing: WsMetrics.space2,
      children: [
        if (_editing)
          Checkbox(
            visualDensity: VisualDensity.compact,
            materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
            value: !excluded,
            onChanged: (_) => setState(() {
              if (excluded) {
                _excluded.remove(file.path);
              } else {
                _excluded.add(file.path);
              }
            }),
          ),
        Expanded(
          child: Text(
            file.shown,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: monoStyle(context, color: excluded ? ws.textMuted : null),
          ),
        ),
        if (excluded)
          Text('Excluded', style: muted)
        else
          Text.rich(
            TextSpan(
              children: [
                TextSpan(
                  text: '+${file.added}',
                  style: TextStyle(color: ws.diffAddStrong),
                ),
                TextSpan(text: ' / ', style: muted),
                TextSpan(
                  text: '-${file.removed}',
                  style: TextStyle(color: ws.diffDelStrong),
                ),
              ],
            ),
            style: monoStyle(context),
          ),
      ],
    );
  }
}
