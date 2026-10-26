import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../controls.dart';
import '../rust/api/events.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';

/// A tool call waiting for approval: Enter allows once, Cmd+Enter for the session, Esc rejects.
class ApprovalCard extends StatefulWidget {
  const ApprovalCard({super.key, required this.request, required this.state});

  final BridgeRequest request;
  final SessionState state;

  @override
  State<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends State<ApprovalCard> {
  final _focus = FocusNode(debugLabel: 'approval');
  final _code = ScrollController();

  /// Whether the card was shown long enough to be read; until then no key decides (ADR-0207).
  bool _armed = false;
  Timer? _arming;

  /// Starts the wait again whenever Liquid comes back in front, since the card may have
  /// appeared while the user worked elsewhere.
  late final AppLifecycleListener _front;

  @override
  void initState() {
    super.initState();
    // The card takes the keyboard as soon as it appears, so the decision needs no mouse.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
    _front = AppLifecycleListener(onStateChange: (_) => _arm());
    _arm();
  }

  /// Takes keys 0.5 s after Liquid is in front with the card.
  void _arm() {
    _armed = false;
    _arming?.cancel();
    if (WidgetsBinding.instance.lifecycleState
        case null || AppLifecycleState.resumed) {
      _arming = Timer(const Duration(milliseconds: 500), () => _armed = true);
    }
  }

  @override
  void dispose() {
    _arming?.cancel();
    _front.dispose();
    _focus.dispose();
    _code.dispose();
    super.dispose();
  }

  /// Moves the call by a line, or by a page of its view.
  void _scroll(int direction, {bool page = false}) {
    if (!_code.hasClients) return;
    final position = _code.position;
    final step = page
        ? position.viewportDimension
        : _lineHeight(monoStyle(context));
    _code.jumpTo(
      (position.pixels + direction * step).clamp(
        position.minScrollExtent,
        position.maxScrollExtent,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final state = widget.state;
    final id = widget.request.id;
    void decide(BridgeDecision decision) => state.decide(id, decision);
    final approval = widget.request.approval!;
    final ws = context.ws;
    final title = switch (approval.kind) {
      BridgeRequestKind.commandExecution => 'Run a shell command?',
      BridgeRequestKind.fileChange => 'Change files?',
      BridgeRequestKind.tool => 'Call ${approval.tool}?',
    };
    final keys = Keymap.of(context);
    final decisions = {
      Command.allowOnce: () => decide(BridgeDecision.allowOnce),
      Command.allowSession: () => decide(BridgeDecision.allowSession),
      Command.reject: () => decide(BridgeDecision.reject),
    };
    return CallbackShortcuts(
      bindings: {
        ...keys.bind(decisions),
        ...Fixed.scrollCall.bind([
          () => _scroll(-1),
          () => _scroll(1),
          () => _scroll(-1, page: true),
          () => _scroll(1, page: true),
        ]),
      },
      child: Focus(
        focusNode: _focus,
        // A key pressed before the card was read decides nothing; a held key never does.
        onKeyEvent: (node, event) =>
            !_armed &&
                decisions.keys
                    .expand(keys.bindings)
                    .any(
                      (decision) => decision.activator.accepts(
                        event,
                        HardwareKeyboard.instance,
                      ),
                    )
            ? KeyEventResult.handled
            : KeyEventResult.ignored,
        child: InputCard(
          ring: ws.warningBorder,
          padding: const EdgeInsets.all(WsMetrics.space4),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  StatusBadge(
                    text: '!',
                    color: ws.warning,
                    background: ws.warningSubtle,
                  ),
                  const SizedBox(width: WsMetrics.space4),
                  // A long tool name wraps rather than lose a part.
                  Expanded(
                    child: Text(
                      title,
                      style: Theme.of(context).textTheme.labelMedium,
                    ),
                  ),
                  // Any other tool is named in the title already.
                  if (approval.kind != BridgeRequestKind.tool) ...[
                    const SizedBox(width: WsMetrics.space4),
                    Text(
                      approval.tool,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                  ],
                ],
              ),
              const SizedBox(height: WsMetrics.space3),
              Expanded(child: _ScrollingCode(approval.text, controller: _code)),
              const SizedBox(height: WsMetrics.space3),
              Row(
                children: [
                  PrimaryButton(
                    label: 'Allow once',
                    shortcut: keys.label(Command.allowOnce),
                    onPressed: () => decide(BridgeDecision.allowOnce),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Allow for this session',
                    shortcut: keys.label(Command.allowSession),
                    onPressed: () => decide(BridgeDecision.allowSession),
                  ),
                  Expanded(
                    child: Text(
                      'your draft is kept',
                      textAlign: TextAlign.end,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: ws.textFaint,
                        fontSize: WsMetrics.fsSmall,
                      ),
                    ),
                  ),
                  const SizedBox(width: WsMetrics.space3),
                  SecondaryButton(
                    label: 'Reject',
                    shortcut: keys.label(Command.reject),
                    color: ws.danger,
                    onPressed: () => decide(BridgeDecision.reject),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Code that scrolls inside a fixed height and says how many lines are below, so nothing is
/// hidden without a hint (approved approval mockup).
class _ScrollingCode extends StatefulWidget {
  const _ScrollingCode(this.text, {required this.controller});

  final String text;
  final ScrollController controller;

  @override
  State<_ScrollingCode> createState() => _ScrollingCodeState();
}

class _ScrollingCodeState extends State<_ScrollingCode> {
  int _below = 0;

  bool _measure(ScrollMetrics metrics, double lineHeight) {
    final below = (metrics.extentAfter / lineHeight).ceil();
    if (below != _below) setState(() => _below = below);
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final style = monoStyle(context);
    final lineHeight = _lineHeight(style);
    return Container(
      decoration: BoxDecoration(
        color: ws.code,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      // The hint sits below the code, so it never covers a line of the call.
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: NotificationListener<ScrollMetricsNotification>(
              onNotification: (notification) =>
                  _measure(notification.metrics, lineHeight),
              child: NotificationListener<ScrollUpdateNotification>(
                onNotification: (notification) =>
                    _measure(notification.metrics, lineHeight),
                child: SingleChildScrollView(
                  controller: widget.controller,
                  padding: const EdgeInsets.symmetric(
                    horizontal: WsMetrics.space4,
                    vertical: WsMetrics.space2,
                  ),
                  child: SizedBox(
                    width: double.infinity,
                    child: Text(widget.text, style: style),
                  ),
                ),
              ),
            ),
          ),
          if (_below > 0)
            Padding(
              padding: const EdgeInsets.only(
                right: WsMetrics.space3,
                bottom: WsMetrics.space1,
              ),
              child: Text(
                '↓ $_below more line${_below == 1 ? '' : 's'}',
                textAlign: TextAlign.end,
                style: TextStyle(
                  color: ws.textFaint,
                  fontSize: WsMetrics.fsSmall,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

double _lineHeight(TextStyle style) => (TextPainter(
  text: TextSpan(text: ' ', style: style),
  textDirection: TextDirection.ltr,
)..layout()).height;
