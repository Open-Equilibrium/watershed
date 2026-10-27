import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../shortcuts/shortcut_reveal.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
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
  final _listFocus = FocusNode(debugLabel: 'notifications');

  /// Opens the list with the keys, so esc closes it, or closes it.
  void _toggle() {
    if (_list.isShowing) return _list.hide();
    _list.show();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_list.isShowing) _listFocus.requestFocus();
    });
  }

  @override
  void initState() {
    super.initState();
    widget.state.requests.addListener(_asked);
  }

  @override
  void dispose() {
    widget.state.requests.removeListener(_asked);
    _listFocus.dispose();
    super.dispose();
  }

  /// The list would cover an approval or a question in the input area.
  void _asked() {
    if (widget.state.asking) _list.hide();
  }

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
                    // A click outside the list or its opener closes it.
                    overlayChildBuilder: (context) => TapRegion(
                      groupId: _list,
                      onTapOutside: (_) => _list.hide(),
                      child: _NoticeList(
                        state: state,
                        link: _anchor,
                        focus: _listFocus,
                        onClose: _list.hide,
                      ),
                    ),
                    // Focusable, so the list opens from the keyboard too.
                    child: TapRegion(
                      groupId: _list,
                      child: InkWell(
                        onTap: _toggle,
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
              ),
              const SizedBox(width: WsMetrics.space6),
              Text(
                '${formatCompact(totals.input)} in · ${formatCompact(totals.output)} out',
                style: text,
              ),
              const SizedBox(width: WsMetrics.space6),
              Text('≈ ${formatCost(totals.costMicroUsd)}', style: text),
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
class _NoticeList extends StatefulWidget {
  const _NoticeList({
    required this.state,
    required this.link,
    required this.focus,
    required this.onClose,
  });

  final SessionState state;
  final LayerLink link;
  final FocusNode focus;
  final VoidCallback onClose;

  @override
  State<_NoticeList> createState() => _NoticeListState();
}

class _NoticeListState extends State<_NoticeList> {
  /// The entry that shows its whole text.
  Notice? _open;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
    // The list takes the keys while open, so esc closes it.
    return CallbackShortcuts(
      bindings: {escapeKey: widget.onClose},
      child: Focus(
        focusNode: widget.focus,
        child: CompositedTransformFollower(
          link: widget.link,
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
                            InkWell(
                              onTap: () {
                                state.clearNotices();
                                widget.onClose();
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
                            for (final notice in notices) ...[
                              // A click or ⏎ shows the whole text (approved mockup).
                              Shortcuts(
                                shortcuts: activateKeys,
                                child: InkWell(
                                  onTap: () => setState(
                                    () =>
                                        _open = _open == notice ? null : notice,
                                  ),
                                  borderRadius: BorderRadius.circular(
                                    WsMetrics.radiusMd,
                                  ),
                                  child: Ink(
                                    height: WsMetrics.rowNav,
                                    decoration: BoxDecoration(
                                      color: _open == notice ? ws.hover : null,
                                      borderRadius: BorderRadius.circular(
                                        WsMetrics.radiusMd,
                                      ),
                                    ),
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
                                ),
                              ),
                              if (_open == notice)
                                Container(
                                  margin: const EdgeInsets.fromLTRB(
                                    WsMetrics.space7,
                                    WsMetrics.space1,
                                    WsMetrics.space2,
                                    WsMetrics.space2,
                                  ),
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: WsMetrics.space3,
                                    vertical: WsMetrics.space2,
                                  ),
                                  decoration: BoxDecoration(
                                    color: ws.subtle,
                                    borderRadius: BorderRadius.circular(
                                      WsMetrics.radiusMd,
                                    ),
                                  ),
                                  child: SelectableText(
                                    notice.text,
                                    style: monoStyle(context)
                                        .copyWith(fontSize: WsMetrics.fsSmall),
                                  ),
                                ),
                            ],
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
