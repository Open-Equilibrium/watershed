import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../diff/diff_panel.dart';
import '../diff/diff_settings.dart';
import '../diff/turn_scope.dart';
import '../session/session_state.dart';
import '../shortcuts/keymap.dart';
import '../theme/tokens.g.dart';
import 'session_card.dart';
import 'session_card_layout.dart';
import 'session_card_settings.dart';

class CardVisibility extends ChangeNotifier {
  bool _inline = true;
  bool _popover = false;
  bool _beside = true;
  bool _notifyQueued = false;
  bool _disposed = false;

  final FocusNode card = FocusNode(
    debugLabel: 'session card',
    canRequestFocus: false,
    skipTraversal: true,
  );

  bool get beside => _beside;

  bool get shown => _beside ? _inline : _popover;

  void focusFirstControl() {
    card.traversalDescendants
        .where((node) => node.canRequestFocus)
        .firstOrNull
        ?.requestFocus();
  }

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

class SessionCardArea extends StatelessWidget {
  const SessionCardArea({
    super.key,
    required this.state,
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
  final SessionCardSettingsController settings;
  final CardVisibility visibility;

  final DiffSettingsController diffSettings;

  final double viewportWidth;

  final ValueChanged<DiffScope> onChanges;

  final VoidCallback onDismiss;

  final Widget child;

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
        settings: settings,
        onChanges: onChanges,
        beside: visibility.beside,
        onOutsidePress: visibility.closePopover,
      ),
    ),
  );
}

enum _Area { chat, card, diff }

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
