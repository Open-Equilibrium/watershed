import 'package:flutter/material.dart';

import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'diff_model.dart';
import 'diff_settings.dart';
import 'diff_view.dart';

/// The Diff as T3 Code's right panel beside the chat: its tab with a close button, then the
/// changes.
class DiffPanel extends StatelessWidget {
  const DiffPanel({
    super.key,
    required this.state,
    required this.settings,
    required this.parse,
    required this.onClose,
  });

  /// The panel's width, as T3 Code's right panel has.
  static const double width = 560;

  final SessionState state;
  final DiffSettingsController settings;
  final PatchParser parse;

  /// Closes the panel; the close button and ⌘D both do.
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return SizedBox(
      width: width,
      child: DecoratedBox(
        decoration: BoxDecoration(
          border: Border(left: BorderSide(color: ws.border)),
        ),
        child: Column(
          children: [
            Container(
              height: 36,
              alignment: Alignment.centerLeft,
              padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space3),
              child: Semantics(
                selected: true,
                child: Container(
                  key: const Key('diff-panel'),
                  height: 26,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space3,
                  ),
                  decoration: BoxDecoration(
                    color: ws.hover,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    spacing: WsMetrics.space2,
                    children: [
                      Icon(LucideIcons.fileDiff, size: 16, color: ws.textFaint),
                      Text(
                        'Diff',
                        style: TextStyle(
                          color: ws.text,
                          fontSize: WsMetrics.fsCompact,
                          fontWeight: WsMetrics.fwMedium,
                        ),
                      ),
                      InkWell(
                        key: const Key('diff-panel-close'),
                        onTap: onClose,
                        child: Icon(
                          LucideIcons.x,
                          size: 11,
                          color: ws.textFaint,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            Expanded(
              child: DiffView(state: state, settings: settings, parse: parse),
            ),
          ],
        ),
      ),
    );
  }
}
