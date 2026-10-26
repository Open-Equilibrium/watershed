import 'package:flutter/material.dart';

import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'shortcut_reveal.dart';
import 'sidebar.dart';

/// Run status, tokens, cost and extension status entries. A notification takes the status
/// slot (approved session-flow mockup); a click lists the latest ones.
class StatusBar extends StatefulWidget {
  const StatusBar({super.key, required this.state, required this.keys});

  final SessionState state;

  /// The keyboard keys of the active view.
  final String keys;

  @override
  State<StatusBar> createState() => _StatusBarState();
}

class _StatusBarState extends State<StatusBar> {
  final _list = OverlayPortalController();
  final _anchor = LayerLink();

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
    final text = TextStyle(
      color: ws.textMuted,
      fontSize: WsMetrics.fsSmall,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    return Container(
      height: 30,
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space5),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: ws.border)),
      ),
      child: ListenableBuilder(
        listenable: Listenable.merge([
          state.status,
          state.totals,
          state.statuses,
          state.failure,
          state.notice,
          state.notices,
        ]),
        builder: (context, _) {
          final (label, color) = statusLabel(context, state.status.value);
          final totals = state.totals.value;
          final failure = state.failure.value;
          final notice = failure == null ? state.notice.value : null;
          final more = state.notices.value.length - 1;
          return Row(
            children: [
              Flexible(
                child: CompositedTransformTarget(
                  link: _anchor,
                  child: OverlayPortal(
                    controller: _list,
                    overlayChildBuilder: (context) => _NoticeList(
                      state: state,
                      link: _anchor,
                      onClose: _list.hide,
                    ),
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: _list.toggle,
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          if (notice != null) ...[
                            Icon(
                              _mark(notice.level),
                              size: 13,
                              color: _tone(context, notice.level),
                            ),
                            const SizedBox(width: WsMetrics.space2),
                          ],
                          Flexible(
                            child: Text(
                              notice?.text ?? '● ${failure ?? label}',
                              key: const Key('status'),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: text.copyWith(
                                color: notice == null
                                    ? color
                                    : _tone(context, notice.level),
                              ),
                            ),
                          ),
                          if (notice != null && more > 0) ...[
                            const SizedBox(width: WsMetrics.space2),
                            Text(
                              '· $more more',
                              style: text.copyWith(color: ws.textFaint),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: WsMetrics.space6),
              Text(
                '${_tokens(totals.input)} in · ${_tokens(totals.output)} out',
                style: text,
              ),
              const SizedBox(width: WsMetrics.space6),
              Text(
                '≈ \$${(totals.costMicroUsd / 1e6).toStringAsFixed(2)}',
                style: text,
              ),
              if (state.statuses.value.isNotEmpty) ...[
                const SizedBox(width: WsMetrics.space6),
                Flexible(
                  child: Text(
                    [
                      for (final entry in state.statuses.value.entries)
                        '${entry.key} ${entry.value}',
                    ].join(' · '),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: text,
                  ),
                ),
              ],
              const Spacer(),
              // The view's keys show only while the reveal key is held (ADR-0204).
              if (ShortcutReveal.shown(context))
                Text(widget.keys, style: text.copyWith(color: ws.textFaint)),
            ],
          );
        },
      ),
    );
  }

  static String _tokens(int count) =>
      count >= 1000 ? '${(count / 1000).toStringAsFixed(1)}k' : '$count';
}

/// The symbol that tells a notification's level apart without color.
IconData _mark(BridgeLevel level) => switch (level) {
  BridgeLevel.info => LucideIcons.info,
  BridgeLevel.warning || BridgeLevel.error => LucideIcons.triangleAlert,
};

Color _tone(BuildContext context, BridgeLevel level) => switch (level) {
  BridgeLevel.info => context.ws.textMuted,
  BridgeLevel.warning => context.ws.warning,
  BridgeLevel.error => context.ws.danger,
};

/// The latest notifications above the status bar.
class _NoticeList extends StatelessWidget {
  const _NoticeList({
    required this.state,
    required this.link,
    required this.onClose,
  });

  final SessionState state;
  final LayerLink link;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    return CompositedTransformFollower(
      link: link,
      targetAnchor: Alignment.topLeft,
      followerAnchor: Alignment.bottomLeft,
      offset: const Offset(-WsMetrics.space2, -WsMetrics.space3),
      child: Align(
        alignment: Alignment.bottomLeft,
        child: Material(
          color: ws.canvas,
          elevation: 0,
          borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
          child: Container(
            width: 320,
            constraints: const BoxConstraints(maxHeight: 320),
            padding: const EdgeInsets.all(WsMetrics.space2),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
              boxShadow: ws.shadow,
            ),
            child: ValueListenableBuilder(
              valueListenable: state.notices,
              builder: (context, notices, _) => Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    height: WsMetrics.rowNav,
                    child: Row(
                      children: [
                        const SizedBox(width: WsMetrics.space2),
                        Text('Notifications', style: small),
                        const Spacer(),
                        GestureDetector(
                          onTap: () {
                            state.clearNotices();
                            onClose();
                          },
                          child: Text('Clear all', style: small),
                        ),
                        const SizedBox(width: WsMetrics.space2),
                      ],
                    ),
                  ),
                  Flexible(
                    child: ListView(
                      shrinkWrap: true,
                      children: [
                        for (final notice in notices)
                          SizedBox(
                            height: WsMetrics.rowNav,
                            child: Row(
                              children: [
                                const SizedBox(width: WsMetrics.space2),
                                Icon(
                                  _mark(notice.level),
                                  size: 13,
                                  color: _tone(context, notice.level),
                                ),
                                const SizedBox(width: WsMetrics.space3),
                                Expanded(
                                  child: Text(
                                    notice.text,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: const TextStyle(
                                      fontSize: WsMetrics.fsCompact,
                                    ),
                                  ),
                                ),
                                Text(
                                  '${notice.at.hour.toString().padLeft(2, '0')}:'
                                  '${notice.at.minute.toString().padLeft(2, '0')}',
                                  style: small,
                                ),
                                const SizedBox(width: WsMetrics.space2),
                              ],
                            ),
                          ),
                      ],
                    ),
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
