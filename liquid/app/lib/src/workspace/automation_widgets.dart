part of 'automation_editor.dart';

String lastRun(BridgeLastRun? last) {
  if (last == null) return 'Not run yet';
  final at = DateTime.fromMillisecondsSinceEpoch(last.at.toInt() * 1000);
  final now = DateTime.now();
  final day = DateUtils.isSameDay(at, now)
      ? 'today'
      : DateFormat.yMMMd('en_US').format(at);
  String rows(int count, String verb) =>
      '$verb $count ${count == 1 ? 'row' : 'rows'}';
  final did = [
    if (last.set_ > 0) rows(last.set_.toInt(), 'set'),
    if (last.added > 0) rows(last.added.toInt(), 'added'),
    if (last.notified > 0)
      '${last.notified} ${last.notified == 1 ? 'notification' : 'notifications'}',
  ];
  return [
    '$day ${DateFormat.Hm('en_US').format(at)}',
    if (did.isEmpty) 'changed nothing' else did.join(' · '),
  ].join(' · ');
}

class _Attention extends StatelessWidget {
  const _Attention({required this.attention});

  final BridgeAttention attention;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final reason = switch (attention) {
      BridgeAttention(kind: BridgeAttentionKind.tableDeleted, :final table) =>
        'Turned off: its Table “$table” was deleted. Choose another Table, '
            'then turn it on.',
      BridgeAttention(:final table, :final field) =>
        'Turned off: its field “$field” of “$table” was removed, with the '
            'conditions and actions that used it. Choose another field where '
            'one is missing, then turn it on.',
    };
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: ws.warningSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusLg),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        spacing: WsMetrics.space3,
        children: [
          Icon(LucideIcons.triangleAlert, size: 16, color: ws.warning),
          Expanded(
            child: Text(reason, style: TextStyle(color: ws.warning)),
          ),
        ],
      ),
    );
  }
}

class _Facts extends StatelessWidget {
  const _Facts({required this.facts});

  final List<(String, Widget)> facts;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: WsMetrics.space2,
      children: [
        for (final (label, value) in facts)
          Row(
            children: [
              SizedBox(
                width: 120,
                child: Text(label, style: TextStyle(color: ws.textMuted)),
              ),
              Expanded(child: value),
            ],
          ),
      ],
    );
  }
}

class _Line extends StatelessWidget {
  const _Line({
    required this.keyword,
    required this.children,
    this.onRemove,
    this.removeKey,
  });

  final String keyword;
  final List<Widget> children;
  final VoidCallback? onRemove;
  final String? removeKey;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return DefaultTextStyle.merge(
      style: TextStyle(fontSize: WsMetrics.fsProse, color: ws.text),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 64,
            height: 30,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                keyword,
                style: TextStyle(
                  color: ws.textMuted,
                  fontWeight: WsMetrics.fwSemibold,
                ),
              ),
            ),
          ),
          Expanded(
            child: Wrap(
              spacing: WsMetrics.space2,
              runSpacing: WsMetrics.space2,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                ...children,
                if (onRemove case final remove?)
                  Tooltip(
                    message: 'Remove',
                    child: InkWell(
                      key: Key(removeKey ?? 'remove'),
                      borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                      onTap: remove,
                      child: SizedBox.square(
                        dimension: 30,
                        child: Icon(
                          LucideIcons.x,
                          size: 14,
                          color: ws.textFaint,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
