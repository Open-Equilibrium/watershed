import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../rust/api/git.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// The approved commit dialog's whole files with their attribution and line counts; Edit
/// shows checkboxes to leave files out.
class CommitFileList extends StatelessWidget {
  const CommitFileList({
    super.key,
    required this.files,
    required this.selected,
    required this.editing,
    required this.enabled,
    required this.onEditing,
    required this.onSelected,
  });

  final List<BridgeCommitFile> files;
  final Set<String> selected;
  final bool editing;
  final bool enabled;

  /// Switches between Edit and Done.
  final VoidCallback onEditing;

  /// Receives the new selection.
  final ValueChanged<Set<String>> onSelected;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final included = files.where((file) => selected.contains(file.change.path));
    final added = included.fold(0, (sum, file) => sum + file.change.added);
    final removed = included.fold(0, (sum, file) => sum + file.change.removed);
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
          child: Row(
            children: [
              if (editing)
                Checkbox(
                  key: const Key('commit-select-all'),
                  semanticLabel: 'All files',
                  materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  visualDensity: VisualDensity.compact,
                  tristate: true,
                  value: selected.isEmpty
                      ? false
                      : selected.length == files.length
                      ? true
                      : null,
                  onChanged: enabled
                      ? (_) => onSelected(
                          selected.length == files.length
                              ? {}
                              : {for (final file in files) file.change.path},
                        )
                      : null,
                ),
              Text('Files', style: small),
              const SizedBox(width: WsMetrics.space2),
              Text(
                selected.length == files.length
                    ? '${files.length}'
                    : '${selected.length} of ${files.length}',
                style: small,
              ),
              const SizedBox(width: WsMetrics.space2),
              Text('· +$added', style: small.copyWith(color: ws.success)),
              const SizedBox(width: WsMetrics.space2),
              Text('−$removed', style: small.copyWith(color: ws.danger)),
              const Spacer(),
              TextButton(
                onPressed: enabled ? onEditing : null,
                child: Text(editing ? 'Done' : 'Edit'),
              ),
            ],
          ),
        ),
        ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 240),
          child: ColoredBox(
            color: ws.canvas,
            child: ListView(
              shrinkWrap: true,
              children: [for (final file in files) _file(context, file)],
            ),
          ),
        ),
      ],
    );
  }

  Widget _file(BuildContext context, BridgeCommitFile file) {
    final ws = context.ws;
    final change = file.change;
    final included = selected.contains(change.path);
    final turns = file.turns;
    final attribution = turns.isEmpty
        ? 'not this session'
        : '${turns.length == 1 ? 'turn' : 'turns'} ${turns.join(', ')}${file.edited ? ' and you' : ''}';
    final (letter, color) = switch (change.status) {
      BridgeChangeStatus.added => ('A', ws.success),
      BridgeChangeStatus.deleted => ('D', ws.danger),
      BridgeChangeStatus.modified => ('M', ws.warning),
    };
    final small = TextStyle(
      fontSize: WsMetrics.fsSmall,
      color: included ? ws.textFaint : ws.textMuted,
    );
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space3,
        vertical: WsMetrics.space2,
      ),
      child: Row(
        spacing: WsMetrics.space2,
        children: [
          if (editing)
            Checkbox(
              key: Key('commit-select-${change.path}'),
              semanticLabel: change.shown,
              materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
              visualDensity: VisualDensity.compact,
              value: included,
              onChanged: enabled
                  ? (value) => onSelected(
                      value == true
                          ? {...selected, change.path}
                          : ({...selected}..remove(change.path)),
                    )
                  : null,
            ),
          Text(
            letter,
            style: small.copyWith(color: included ? color : ws.textFaint),
          ),
          Expanded(
            flex: 2,
            child: Text(
              change.shown,
              style: monoStyle(
                context,
                color: included ? ws.text : ws.textFaint,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (!included)
            Text('Excluded', style: small)
          else ...[
            if (!editing && file.attributionAvailable) ...[
              Flexible(
                child: Tooltip(
                  message: attribution,
                  child: Text(
                    attribution,
                    style: small,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
              Text('·', style: small),
            ],
            Text('+${change.added}', style: small.copyWith(color: ws.success)),
            Text('−${change.removed}', style: small.copyWith(color: ws.danger)),
          ],
        ],
      ),
    );
  }
}
