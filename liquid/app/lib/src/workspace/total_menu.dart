import 'package:flutter/material.dart';

import '../controls.dart';
import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

String totalName(BridgeTotal total) => switch (total) {
  BridgeTotal.sum => 'Sum',
  BridgeTotal.average => 'Average',
  BridgeTotal.count => 'Count',
  BridgeTotal.min => 'Min',
  BridgeTotal.max => 'Max',
  BridgeTotal.empty => 'Empty',
  BridgeTotal.filled => 'Filled',
};

bool sameTotals(List<BridgeFieldTotal> a, List<BridgeFieldTotal> b) =>
    a.length == b.length &&
    [for (final (at, total) in a.indexed) total == b[at]].every((same) => same);

class TotalMenu extends StatelessWidget {
  const TotalMenu({
    super.key,
    required this.numeric,
    required this.chosen,
    required this.onPick,
  });

  final bool numeric;
  final BridgeTotal? chosen;
  final ValueChanged<BridgeTotal?> onPick;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final choices = <BridgeTotal?>[
      null,
      for (final total in BridgeTotal.values)
        if (numeric ||
            total == BridgeTotal.count ||
            total == BridgeTotal.empty ||
            total == BridgeTotal.filled)
          total,
    ];
    return FloatingPanel(
      key: const Key('total menu'),
      width: 180,
      padding: const EdgeInsets.all(WsMetrics.space1),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final total in choices)
            InkWell(
              borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
              onTap: () => onPick(total),
              child: Container(
                height: 30,
                padding: const EdgeInsets.symmetric(
                  horizontal: WsMetrics.space3,
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        total == null ? 'None' : totalName(total),
                        style: TextStyle(
                          fontSize: WsMetrics.fsUi,
                          color: ws.text,
                        ),
                      ),
                    ),
                    if (total == chosen)
                      Icon(LucideIcons.check, size: 14, color: ws.accent),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}
