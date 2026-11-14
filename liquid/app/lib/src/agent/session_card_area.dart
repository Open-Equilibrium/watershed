import 'package:flutter/material.dart';

import '../session/session_state.dart';
import '../theme/tokens.g.dart';
import 'session_card.dart';
import 'session_card_layout.dart';
import 'session_card_settings.dart';

/// The chat with the session card: beside the chat where the chat keeps its width, else as a
/// popover at its top right. `open` shows or hides the card; a press outside the popover closes
/// it, as in T3 Code.
class SessionCardArea extends StatefulWidget {
  const SessionCardArea({
    super.key,
    required this.state,
    required this.device,
    required this.settings,
    required this.open,
    required this.diffOpen,
    required this.onChanges,
    required this.child,
  });

  final SessionState state;

  /// This computer's name, which a pull request summary reads from.
  final String device;
  final SessionCardSettingsController settings;
  final ValueNotifier<bool> open;

  /// Whether the Diff panel is open, which moves the card into a popover.
  final bool diffOpen;

  /// Opens the Diff panel with the latest turn, as the Changes row does.
  final VoidCallback onChanges;

  /// The chat and its input area.
  final Widget child;

  @override
  State<SessionCardArea> createState() => _SessionCardAreaState();
}

class _SessionCardAreaState extends State<SessionCardArea> {
  /// Where the layout last put the card, which a press outside it reads.
  final _placement = CardPlacement();

  @override
  Widget build(BuildContext context) => ValueListenableBuilder(
    valueListenable: widget.open,
    builder: (context, shown, _) => CustomMultiChildLayout(
      key: const Key('chat-column'),
      delegate: _CardArea(
        diffOpen: widget.diffOpen,
        shown: shown,
        placement: _placement,
      ),
      children: [
        LayoutId(id: _Area.chat, child: widget.child),
        if (shown)
          LayoutId(
            id: _Area.card,
            child: SessionCard(
              state: widget.state,
              device: widget.device,
              settings: widget.settings,
              onChanges: widget.onChanges,
              placement: _placement,
              onOutsidePress: () => widget.open.value = false,
            ),
          ),
      ],
    ),
  );
}

/// The parts of the card area.
enum _Area { chat, card }

/// Places the chat's lane and the card. Beside the chat, the card sits at the top right; as a
/// popover, it sits a little lower.
class _CardArea extends MultiChildLayoutDelegate {
  _CardArea({
    required this.diffOpen,
    required this.shown,
    required this.placement,
  });

  final bool diffOpen;
  final bool shown;
  final CardPlacement placement;

  @override
  void performLayout(Size size) {
    final beside = !diffOpen && cardFitsBeside(size);
    placement.beside = beside;
    final lane = chatLane(size.width, beside: beside);
    // The lane is widened by the input area's margin on each side, so the input area keeps the
    // lane's width and stays centred on it.
    layoutChild(
      _Area.chat,
      BoxConstraints.tightFor(
        width: lane.width + 2 * WsMetrics.space4,
        height: size.height,
      ),
    );
    positionChild(_Area.chat, Offset(lane.left - WsMetrics.space4, 0));
    if (!shown || !hasChild(_Area.card)) return;
    final top = beside ? cardInset : WsMetrics.space1;
    layoutChild(
      _Area.card,
      BoxConstraints(
        minWidth: cardWidth,
        maxWidth: cardWidth,
        maxHeight: size.height - top - cardInset,
      ),
    );
    positionChild(_Area.card, Offset(size.width - cardInset - cardWidth, top));
  }

  @override
  bool shouldRelayout(_CardArea oldDelegate) =>
      oldDelegate.diffOpen != diffOpen ||
      oldDelegate.shown != shown ||
      !identical(oldDelegate.placement, placement);
}
