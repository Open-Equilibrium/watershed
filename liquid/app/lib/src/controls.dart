import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'shortcuts/keymap.dart';
import 'shortcuts/shortcut_reveal.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

/// Width of the reading column in the chat and the input area.
const double readingWidth = WsMetrics.widthReading;

/// A contextual shortcut hint, shown only while the reveal key is held. It takes no space, so
/// nothing moves when it appears.
class Hint extends StatelessWidget {
  const Hint(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    if (text.isEmpty || !ShortcutReveal.shown(context)) {
      return const SizedBox.shrink();
    }
    return SizedBox.shrink(
      child: OverflowBox(
        alignment: Alignment.centerLeft,
        maxWidth: double.infinity,
        maxHeight: double.infinity,
        child: KeyBadge(text),
      ),
    );
  }
}

/// The look of a shortcut hint: small, dark and above what it labels (approved mockup).
class KeyBadge extends StatelessWidget {
  const KeyBadge(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space1),
      decoration: BoxDecoration(
        color: ws.text,
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
        boxShadow: ws.shadow,
      ),
      child: Text(
        text,
        style: TextStyle(
          fontSize: WsMetrics.fsCaption,
          height: WsMetrics.lhCaption / WsMetrics.fsCaption,
          fontWeight: WsMetrics.fwSemibold,
          color: ws.surface,
        ),
      ),
    );
  }
}

/// A one-character status badge.
class StatusBadge extends StatelessWidget {
  const StatusBadge({
    super.key,
    required this.text,
    required this.color,
    required this.background,
  });

  final String text;
  final Color color;
  final Color background;

  @override
  Widget build(BuildContext context) => Container(
    width: 22,
    height: 22,
    alignment: Alignment.center,
    decoration: BoxDecoration(
      color: background,
      borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
    ),
    child: Text(
      text,
      style: TextStyle(
        color: color,
        fontWeight: WsMetrics.fwBold,
        fontSize: WsMetrics.fsSmall,
      ),
    ),
  );
}

class _Button extends StatelessWidget {
  const _Button({
    required this.label,
    required this.onPressed,
    required this.foreground,
    required this.background,
    this.outline,
    this.icon,
    this.shortcut,
    this.focusNode,
  });

  final String label;
  final String? shortcut;

  /// None shows the button disabled.
  final VoidCallback? onPressed;
  final Color foreground;
  final Color background;
  final Color? outline;

  /// A Lucide icon before the label.
  final IconData? icon;

  /// Makes the button take the keyboard focus; others leave it where it is.
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final button = Material(
      color: background,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        side: outline == null ? BorderSide.none : BorderSide(color: outline!),
      ),
      child: InkWell(
        onTap: onPressed,
        focusNode: focusNode,
        canRequestFocus: focusNode != null,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
        child: Container(
          height: 28,
          padding: const EdgeInsets.symmetric(horizontal: _buttonPadding),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            // Centered when a layout stretches the button.
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              if (icon case final icon?) ...[
                Icon(icon, size: 16, color: foreground),
                const SizedBox(width: WsMetrics.space2),
              ],
              // Cut short only where a layout leaves the button too little room.
              Flexible(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: _buttonLabel.copyWith(color: foreground),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    if (onPressed == null) return Opacity(opacity: 0.45, child: button);
    final hint = shortcut;
    if (hint == null || hint.isEmpty || !ShortcutReveal.shown(context)) {
      return button;
    }
    // The hint sits on the button's corner, so the button keeps its size.
    return Stack(
      clipBehavior: Clip.none,
      children: [
        button,
        Positioned(top: -9, right: -6, child: KeyBadge(hint)),
      ],
    );
  }
}

const _buttonLabel = TextStyle(
  fontWeight: WsMetrics.fwMedium,
  fontSize: WsMetrics.fsCompact,
);
const _buttonPadding = WsMetrics.space4;

/// The width a shared button with `label` takes, for layouts that must know before drawing it.
double buttonWidth(BuildContext context, String label) =>
    measureText(
      label,
      DefaultTextStyle.of(context).style.merge(_buttonLabel),
    ).width +
    2 * _buttonPadding;

class PrimaryButton extends StatelessWidget {
  const PrimaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
    this.focusNode,
  });

  final String label;
  final String? shortcut;

  /// None shows the button disabled.
  final VoidCallback? onPressed;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    shortcut: shortcut,
    onPressed: onPressed,
    focusNode: focusNode,
    foreground: context.ws.onAccent,
    background: context.ws.accent,
  );
}

/// The primary action when it cannot be undone from Liquid, such as reverting files.
class DangerButton extends StatelessWidget {
  const DangerButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
  });

  final String label;
  final String? shortcut;

  /// None shows the button disabled.
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    shortcut: shortcut,
    onPressed: onPressed,
    foreground: context.ws.onAccent,
    background: context.ws.danger,
  );
}

/// Asks before an action Liquid cannot undo (approved checkpoint mockups): ⏎ confirms while
/// it is allowed and esc cancels; it closes with `true` when confirmed. Without `action` it
/// only says why nothing can be done.
class DangerDialog extends StatelessWidget {
  const DangerDialog({
    super.key,
    required this.title,
    required this.children,
    this.action,
    this.allowed = true,
  });

  final String title;
  final List<Widget> children;
  final String? action;
  final bool allowed;

  /// The style of a dialog's text.
  static TextStyle text(BuildContext context) => TextStyle(
    color: context.ws.textMuted,
    fontSize: WsMetrics.fsCompact,
    height: WsMetrics.lhCompact / WsMetrics.fsCompact,
  );

  @override
  Widget build(BuildContext context) {
    final confirm = action != null && allowed;
    void confirmed() => Navigator.of(context).pop(true);
    return ActionDialog(
      title: title,
      bindings: {
        if (confirm) const Keys(LogicalKeyboardKey.enter).activator: confirmed,
      },
      cancelled: false,
      actions: [
        if (action case final label?)
          DangerButton(
            label: label,
            shortcut: confirm ? '⏎' : null,
            onPressed: confirm ? confirmed : null,
          ),
      ],
      children: children,
    );
  }
}

/// A dialog with a title, its text, its actions and Cancel, which esc also takes, closing
/// with `cancelled`; without actions Cancel is Close.
class ActionDialog extends StatelessWidget {
  const ActionDialog({
    super.key,
    required this.title,
    required this.children,
    required this.actions,
    this.bindings = const {},
    this.cancelled,
    this.dismiss,
  });

  final String title;
  final List<Widget> children;
  final List<Widget> actions;
  final Map<ShortcutActivator, VoidCallback> bindings;
  final Object? cancelled;

  /// The label of the button esc takes, if not Cancel or, without actions, Close.
  final String? dismiss;

  @override
  Widget build(BuildContext context) => CallbackShortcuts(
    bindings: {
      ...bindings,
      const Keys(LogicalKeyboardKey.escape).activator: () =>
          Navigator.of(context).pop(cancelled),
    },
    child: Focus(
      autofocus: true,
      child: Dialog(
        backgroundColor: context.ws.surface,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
        ),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 520),
          child: Padding(
            padding: const EdgeInsets.all(WsMetrics.space5),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              spacing: WsMetrics.space4,
              children: [
                Text(
                  title,
                  style: Theme.of(context).textTheme.titleSmall
                      ?.copyWith(fontWeight: WsMetrics.fwSemibold),
                ),
                Flexible(
                  child: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      spacing: WsMetrics.space4,
                      children: children,
                    ),
                  ),
                ),
                // Long labels wrap below each other rather than overflow.
                OverflowBar(
                  alignment: MainAxisAlignment.spaceBetween,
                  spacing: WsMetrics.space3,
                  overflowSpacing: WsMetrics.space3,
                  children: [
                    ...actions,
                    SecondaryButton(
                      label: dismiss ?? (actions.isEmpty ? 'Close' : 'Cancel'),
                      shortcut: 'esc',
                      onPressed: () => Navigator.of(context).pop(cancelled),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

/// A warning inside a dialog or menu, such as which agent holds an action up.
class WarningNotice extends StatelessWidget {
  const WarningNotice(this.text, {super.key, this.danger = false});

  final String text;

  /// Something failed rather than needs attention.
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: danger ? ws.dangerSubtle : ws.warningSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Text(
        text,
        style: TextStyle(
          color: danger ? ws.danger : ws.warning,
          fontSize: WsMetrics.fsCompact,
        ),
      ),
    );
  }
}

class SecondaryButton extends StatelessWidget {
  const SecondaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.icon,
    this.shortcut,
    this.color,
    this.focusNode,
  });

  final String label;
  final IconData? icon;
  final String? shortcut;
  final Color? color;
  final VoidCallback? onPressed;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    icon: icon,
    shortcut: shortcut,
    onPressed: onPressed,
    focusNode: focusNode,
    foreground: color ?? context.ws.text,
    background: context.ws.canvas,
    outline: context.ws.borderStrong,
  );
}

/// A quiet button without a frame, for view options beside the main actions.
class GhostButton extends StatelessWidget {
  const GhostButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.icon,
    this.shortcut,
    this.focusNode,
  });

  final String label;
  final IconData? icon;
  final String? shortcut;
  final VoidCallback onPressed;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    icon: icon,
    shortcut: shortcut,
    onPressed: onPressed,
    focusNode: focusNode,
    foreground: context.ws.textMuted,
    background: Colors.transparent,
  );
}

/// The input area's card; controls in it show focus and hover above its background.
class InputCard extends StatelessWidget {
  const InputCard({
    super.key,
    this.ring,
    required this.padding,
    required this.child,
  });

  final Color? ring;
  final EdgeInsetsGeometry padding;
  final Widget child;

  @override
  Widget build(BuildContext context) => Container(
    decoration: BoxDecoration(
      color: context.ws.surface,
      borderRadius: BorderRadius.circular(WsMetrics.radiusXl),
      boxShadow: [
        if (ring case final ring?) BoxShadow(color: ring, spreadRadius: 1),
        ...context.ws.shadow,
      ],
    ),
    padding: padding,
    child: Material(type: MaterialType.transparency, child: child),
  );
}

/// The size `text` takes in `style`; the measuring painter is released at once.
Size measureText(String text, TextStyle style) {
  final painter = TextPainter(
    text: TextSpan(text: text, style: style),
    textDirection: TextDirection.ltr,
  )..layout();
  final size = painter.size;
  painter.dispose();
  return size;
}

/// ⏎ and Space activate the focused control, before a view's own keys take them.
const activateKeys = <ShortcutActivator, Intent>{
  SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
};

/// A choice of one among a few, as the segments of one control (approved mockups).
class Segments<T> extends StatelessWidget {
  const Segments({
    super.key,
    required this.options,
    required this.selected,
    required this.onSelect,
    this.dense = false,
  });

  /// Each option with its label, in order.
  final List<(T, String)> options;
  final T selected;
  final ValueChanged<T> onSelect;

  /// Smaller, for a table header.
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: ws.subtle,
        border: Border.all(color: ws.borderStrong),
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd + 1),
      ),
      // Focus shows above the group's background.
      child: Material(
        type: MaterialType.transparency,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final (value, label) in options)
              _Segment(
                label: label,
                selected: value == selected,
                dense: dense,
                onTap: () => onSelect(value),
              ),
          ],
        ),
      ),
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.label,
    required this.selected,
    required this.dense,
    required this.onTap,
  });

  final String label;
  final bool selected;
  final bool dense;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    // Which segment is chosen is told to assistive technology, not only drawn; return and space
    // press the focused segment before a view's own keys take them.
    return Semantics(
      selected: selected,
      inMutuallyExclusiveGroup: true,
      child: Shortcuts(
        shortcuts: activateKeys,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
          // Ink, so the focus highlight shows above the selected segment.
          child: Ink(
            height: dense ? 20 : 24,
            padding: EdgeInsets.symmetric(
              horizontal: dense ? WsMetrics.space2 : WsMetrics.space4,
            ),
            decoration: selected
                ? BoxDecoration(
                    color: ws.canvas,
                    borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
                    border: Border.all(color: ws.borderStrong),
                  )
                : null,
            child: Center(
              widthFactor: 1,
              child: Text(
                label,
                style: TextStyle(
                  fontSize: dense ? WsMetrics.fsCaption : WsMetrics.fsCompact,
                  color: selected ? ws.text : ws.textMuted,
                  fontWeight: selected
                      ? WsMetrics.fwMedium
                      : WsMetrics.fwRegular,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
