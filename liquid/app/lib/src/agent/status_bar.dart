import 'package:flutter/material.dart';

import '../controls.dart';
import '../format.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../shortcuts/shortcut_reveal.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'sidebar.dart';

class StatusBar extends StatefulWidget {
  const StatusBar({super.key, required this.state, required this.keys});

  final SessionState state;

  final String keys;

  @override
  State<StatusBar> createState() => _StatusBarState();
}

class _StatusBarState extends State<StatusBar> {
  final _list = OverlayPortalController();
  final _anchor = LayerLink();
  final _listFocus = FocusNode(debugLabel: 'notifications');

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
    widget.state.request.addListener(_asked);
  }

  @override
  void dispose() {
    widget.state.request.removeListener(_asked);
    _listFocus.dispose();
    super.dispose();
  }

  void _asked() {
    if (widget.state.request.value != null) _list.hide();
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
          state.notice,
          state.notices,
        ]),
        builder: (context, _) {
          final (label, color) = statusLabel(context, state.status.value);
          final totals = state.totals.value;
          final notice = state.notice.value;
          final more = state.notices.value.length - 1;
          return Row(
            children: [
              Flexible(
                child: CompositedTransformTarget(
                  link: _anchor,
                  child: OverlayPortal(
                    controller: _list,
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
                                notice?.text ?? '● $label',
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
                '${formatTokens(totals.input)} in · ${formatTokens(totals.output)} out',
                style: text,
              ),
              const Spacer(),
              if (ShortcutReveal.shown(context))
                Text(widget.keys, style: text.copyWith(color: ws.textFaint)),
            ],
          );
        },
      ),
    );
  }
}

IconData _mark(NoticeLevel level) => switch (level) {
  NoticeLevel.info => LucideIcons.info,
  NoticeLevel.warning || NoticeLevel.error => LucideIcons.triangleAlert,
};

Color _tone(BuildContext context, NoticeLevel level) => switch (level) {
  NoticeLevel.info => context.ws.textMuted,
  NoticeLevel.warning => context.ws.warning,
  NoticeLevel.error => context.ws.danger,
};

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
  Notice? _open;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final state = widget.state;
    final small = TextStyle(color: ws.textFaint, fontSize: WsMetrics.fsSmall);
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
                                          formatClock(notice.at),
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
