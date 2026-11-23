import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/source.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'source_kind.dart';

class SourceListing extends StatelessWidget {
  const SourceListing({
    super.key,
    required this.entries,
    required this.inside,
    required this.onOpen,
  });

  final List<BridgeFileEntry> entries;
  final String inside;
  final ValueChanged<String> onOpen;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final faint = TextStyle(fontSize: WsMetrics.fsCompact, color: ws.textFaint);
    final day = DateFormat.MMMd('en_US');
    Widget row(List<Widget> cells, {VoidCallback? onTap, Key? key}) => InkWell(
      key: key,
      onTap: onTap,
      child: Container(
        height: WsMetrics.rowTable,
        padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
        decoration: BoxDecoration(
          border: Border(bottom: BorderSide(color: ws.border)),
        ),
        child: Row(children: cells),
      ),
    );
    return Container(
      key: const Key('source listing'),
      clipBehavior: Clip.antiAlias,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        border: Border.all(color: ws.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          row([
            Expanded(child: Text('Name', style: faint)),
            SizedBox(width: 110, child: Text('Kind', style: faint)),
            SizedBox(
              width: 90,
              child: Text('Size', style: faint, textAlign: TextAlign.right),
            ),
            const SizedBox(width: WsMetrics.space4),
            SizedBox(width: 80, child: Text('Modified', style: faint)),
          ]),
          if (entries.isEmpty)
            Padding(
              padding: const EdgeInsets.all(WsMetrics.space4),
              child: Text('This folder is empty.', style: faint),
            ),
          for (final entry in entries)
            row(
              key: ValueKey('source entry ${entry.name}'),
              onTap: () =>
                  onOpen(inside.isEmpty ? entry.name : '$inside/${entry.name}'),
              [
                Icon(
                  sourceIcon(entry.name, folder: entry.folder),
                  size: 14,
                  color: ws.textFaint,
                ),
                const SizedBox(width: WsMetrics.space2),
                Expanded(
                  child: Text(
                    entry.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                SizedBox(
                  width: 110,
                  child: Text(entry.folder ? 'Folder' : kindOf(entry.name)),
                ),
                SizedBox(
                  width: 90,
                  child: Text(
                    entry.folder ? '—' : fileSize(entry.size.toInt()),
                    textAlign: TextAlign.right,
                  ),
                ),
                const SizedBox(width: WsMetrics.space4),
                SizedBox(
                  width: 80,
                  child: Text(
                    entry.modified == null
                        ? ''
                        : day.format(
                            DateTime.fromMillisecondsSinceEpoch(
                              entry.modified!.toInt(),
                            ),
                          ),
                  ),
                ),
              ],
            ),
        ],
      ),
    );
  }
}
