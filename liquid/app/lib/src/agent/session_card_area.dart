import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../diff/diff_panel.dart';
import '../diff/diff_settings.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/tokens.g.dart';
import 'session_card.dart';
import 'session_card_layout.dart';
import 'session_card_settings.dart';

/// Whether the session card shows, and where: beside the chat with its inline flag, or over the
/// chat as a popover with its own. Both flags are kept apart, as T3 Code keeps them, so the
/// popover waits closed until its button opens it.
class CardVisibility extends ChangeNotifier {
  bool _inline = true;
  bool _popover = false;
  bool _beside = true;
  bool _notifyQueued = false;
  bool _disposed = false;

  /// Holds the card in the focus tree, so the card's first control can be found. It is not a
  /// scope, which would keep Tab inside the card.
  final FocusNode card = FocusNode(
    debugLabel: 'session card',
    canRequestFocus: false,
    skipTraversal: true,
  );

  /// Whether the card sits beside the chat, as the layout last placed it.
  bool get beside => _beside;

  /// Whether the card is drawn: the flag of the place it sits in.
  bool get shown => _beside ? _inline : _popover;

  /// Moves keyboard focus to the card's first control.
  void focusFirstControl() {
    card.traversalDescendants
        .where((node) => node.canRequestFocus)
        .firstOrNull
        ?.requestFocus();
  }

  /// The sliders button: opens or closes the card where it sits.
  void toggle() {
    if (_beside) {
      _inline = !_inline;
    } else {
      _popover = !_popover;
    }
    notifyListeners();
  }

  void closePopover() {
    if (!_popover) return;
    _popover = false;
    notifyListeners();
  }

  /// Records where the layout puts the card. A popover that the layout places beside the chat
  /// closes, as T3 Code's does. Listeners hear of a change after the frame, since the layout is
  /// still running.
  void place(bool beside) {
    if (beside == _beside) return;
    _beside = beside;
    if (beside) _popover = false;
    if (_notifyQueued) return;
    _notifyQueued = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _notifyQueued = false;
      if (!_disposed) notifyListeners();
    });
  }

  @override
  void dispose() {
    _disposed = true;
    card.dispose();
    super.dispose();
  }
}

/// The chat area: the chat with the session card, beside the chat where the chat keeps its width,
/// else as a popover at its top right. A press outside the popover closes it; Escape closes it
/// too, and `onDismiss` returns focus to its button. The Diff panel, when open, sits at the right
/// and takes the width the Diff settings keep.
class SessionCardArea extends StatelessWidget {
  const SessionCardArea({
    super.key,
    required this.state,
    required this.device,
    required this.settings,
    required this.visibility,
    required this.diffSettings,
    required this.viewportWidth,
    required this.onChanges,
    required this.onDismiss,
    required this.child,
    this.diff,
  });

  final SessionState state;

  /// This computer's name, which a pull request summary reads from.
  final String device;
  final SessionCardSettingsController settings;
  final CardVisibility visibility;

  /// The Diff settings, which keep the Diff panel's width.
  final DiffSettingsController diffSettings;

  /// The window's width, which bounds the Diff panel.
  final double viewportWidth;

  /// Opens the Diff panel with the latest turn, as the Changes row does.
  final VoidCallback onChanges;

  /// Closes the popover and gives focus back to its button.
  final VoidCallback onDismiss;

  /// The chat and its input area.
  final Widget child;

  /// The Diff panel, or null while it is closed; the card becomes a popover while it is open.
  final Widget? diff;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([visibility, diffSettings]),
    builder: (context, _) => CustomMultiChildLayout(
      key: const Key('chat-column'),
      delegate: _CardArea(
        shown: visibility.shown,
        hasDiff: diff != null,
        diffWidth: diffSettings.value.width,
        viewportWidth: viewportWidth,
        visibility: visibility,
      ),
      children: [
        LayoutId(id: _Area.chat, child: child),
        if (visibility.shown) LayoutId(id: _Area.card, child: _card()),
        if (diff case final diff?) LayoutId(id: _Area.diff, child: diff),
      ],
    ),
  );

  Widget _card() => CallbackShortcuts(
    bindings: {
      if (!visibility.beside)
        const Keys(LogicalKeyboardKey.escape).activator: onDismiss,
    },
    child: Focus(
      focusNode: visibility.card,
      canRequestFocus: false,
      skipTraversal: true,
      child: SessionCard(
        state: state,
        device: device,
        settings: settings,
        onChanges: onChanges,
        beside: visibility.beside,
        onOutsidePress: visibility.closePopover,
      ),
    ),
  );
}

/// The parts of the chat area.
enum _Area { chat, card, diff }

/// Places the chat's lane, the card and the Diff panel. The Diff panel takes its width, clamped to
/// what the window and the chat allow, at the right; the chat lane and the card share what is left.
/// Beside the chat, the card sits at the top right; as a popover, it sits a little lower.
class _CardArea extends MultiChildLayoutDelegate {
  _CardArea({
    required this.shown,
    required this.hasDiff,
    required this.diffWidth,
    required this.viewportWidth,
    required this.visibility,
  });

  final bool shown;
  final bool hasDiff;
  final double? diffWidth;
  final double viewportWidth;
  final CardVisibility visibility;

  @override
  void performLayout(Size size) {
    var areaWidth = size.width;
    if (hasChild(_Area.diff)) {
      final max = DiffPanel.maxWidthOf(viewportWidth, size.width);
      final width = (diffWidth ?? DiffPanel.defaultWidth).clamp(
        DiffPanel.minWidth,
        max,
      );
      layoutChild(
        _Area.diff,
        BoxConstraints.tightFor(width: width, height: size.height),
      );
      areaWidth = size.width - width;
      positionChild(_Area.diff, Offset(areaWidth, 0));
    }
    final area = Size(areaWidth, size.height);
    visibility.place(!hasChild(_Area.diff) && cardFitsBeside(area));
    final beside = visibility.beside;
    final lane = chatLane(area.width, beside: beside);
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
    positionChild(_Area.card, Offset(area.width - cardInset - cardWidth, top));
  }

  @override
  bool shouldRelayout(_CardArea oldDelegate) =>
      oldDelegate.shown != shown ||
      oldDelegate.hasDiff != hasDiff ||
      oldDelegate.diffWidth != diffWidth ||
      oldDelegate.viewportWidth != viewportWidth;
}
