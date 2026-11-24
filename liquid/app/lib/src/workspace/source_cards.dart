import 'dart:io';

import 'package:flutter/material.dart';

import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import '../controls.dart';

class SourceCard extends StatelessWidget {
  const SourceCard({
    super.key,
    required this.text,
    required this.path,
    required this.onOpen,
  });

  final String text;
  final String path;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final name = path.split(Platform.pathSeparator).last;
    return Container(
      key: const Key('source card'),
      padding: const EdgeInsets.all(WsMetrics.space4),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
        border: Border.all(color: ws.border),
      ),
      child: Row(
        children: [
          Icon(LucideIcons.file, size: 22, color: ws.textFaint),
          const SizedBox(width: WsMetrics.space3),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(name),
                Text(
                  text,
                  style: TextStyle(
                    fontSize: WsMetrics.fsSmall,
                    color: ws.textFaint,
                  ),
                ),
              ],
            ),
          ),
          SecondaryButton(label: 'Open in default app', onPressed: onOpen),
        ],
      ),
    );
  }
}
