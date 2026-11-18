import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'shortcuts/keymap.dart';
import 'shortcuts/shortcut_reveal.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

const double readingWidth = WsMetrics.widthReading;

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
    this.semanticsLabel,
  });

  final String label;
  final String? shortcut;

  final String? semanticsLabel;

  final VoidCallback? onPressed;
  final Color foreground;
  final Color background;
  final Color? outline;

  final IconData? icon;

  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final button = Semantics(
      button: true,
      enabled: onPressed != null,
      label: semanticsLabel,
      child: Material(
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
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                if (icon case final icon?) ...[
                  Icon(icon, size: 16, color: foreground),
                  const SizedBox(width: WsMetrics.space2),
                ],
                Flexible(
                  child: ExcludeSemantics(
                    excluding: semanticsLabel != null,
                    child: Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: _buttonLabel.copyWith(color: foreground),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    if (onPressed == null) return Opacity(opacity: 0.45, child: button);
    final hint = shortcut;
    if (hint == null || hint.isEmpty || !ShortcutReveal.shown(context)) {
      return button;
    }
    return Stack(
      clipBehavior: Clip.none,
      fit: StackFit.passthrough,
      children: [
        button,
        Positioned(
          top: -9,
          right: -6,
          child: IgnorePointer(child: KeyBadge(hint)),
        ),
      ],
    );
  }
}

const _buttonLabel = TextStyle(
  fontWeight: WsMetrics.fwMedium,
  fontSize: WsMetrics.fsCompact,
);
const _buttonPadding = WsMetrics.space4;

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

class DangerButton extends StatelessWidget {
  const DangerButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
  });

  final String label;
  final String? shortcut;

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

class ActionDialog extends StatelessWidget {
  const ActionDialog({
    super.key,
    required this.title,
    required this.children,
    required this.actions,
    this.bindings = const {},
    this.cancelled,
    this.dismiss,
    this.dismissible = true,
  });

  final String title;
  final List<Widget> children;
  final List<Widget> actions;
  final Map<ShortcutActivator, VoidCallback> bindings;
  final Object? cancelled;

  final String? dismiss;

  final bool dismissible;

  @override
  Widget build(BuildContext context) => CallbackShortcuts(
    bindings: {
      ...bindings,
      if (dismissible)
        const Keys(LogicalKeyboardKey.escape).activator: () =>
            Navigator.of(context).pop(cancelled),
    },
    child: PopScope(
      canPop: dismissible,
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
                  OverflowBar(
                    alignment: MainAxisAlignment.end,
                    spacing: WsMetrics.space3,
                    overflowSpacing: WsMetrics.space3,
                    overflowDirection: VerticalDirection.up,
                    children: [
                      SecondaryButton(
                        label:
                            dismiss ?? (actions.isEmpty ? 'Close' : 'Cancel'),
                        shortcut: 'esc',
                        onPressed: dismissible
                            ? () => Navigator.of(context).pop(cancelled)
                            : null,
                      ),
                      ...actions.reversed,
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

class WarningNotice extends StatelessWidget {
  const WarningNotice(
    this.text, {
    super.key,
    this.danger = false,
    this.info = false,
  });

  final String text;

  final bool danger;

  final bool info;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: WsMetrics.space4,
        vertical: WsMetrics.space3,
      ),
      decoration: BoxDecoration(
        color: info
            ? ws.subtle
            : danger
            ? ws.dangerSubtle
            : ws.warningSubtle,
        borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
      ),
      child: Text(
        text,
        style: TextStyle(
          color: info
              ? ws.textMuted
              : danger
              ? ws.danger
              : ws.warning,
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
    this.semanticsLabel,
  });

  final String label;
  final IconData? icon;
  final String? shortcut;
  final Color? color;
  final VoidCallback? onPressed;
  final FocusNode? focusNode;
  final String? semanticsLabel;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    icon: icon,
    shortcut: shortcut,
    onPressed: onPressed,
    focusNode: focusNode,
    semanticsLabel: semanticsLabel,
    foreground: color ?? context.ws.text,
    background: context.ws.canvas,
    outline: context.ws.borderStrong,
  );
}

class GhostButton extends StatelessWidget {
  const GhostButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.icon,
    this.shortcut,
    this.focusNode,
    this.on = false,
  });

  final String label;
  final IconData? icon;
  final String? shortcut;
  final VoidCallback onPressed;
  final FocusNode? focusNode;
  final bool on;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return _Button(
      label: label,
      icon: icon,
      shortcut: shortcut,
      onPressed: onPressed,
      focusNode: focusNode,
      foreground: on ? ws.accent : ws.textMuted,
      background: on ? ws.accentSubtle : Colors.transparent,
      outline: on ? ws.accent.withValues(alpha: 0.4) : null,
    );
  }
}

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

bool writeText(TextEditingController controller, TextEditingValue value) {
  if (controller.text != value.text) {
    controller.value = value;
    return true;
  }
  if (value.selection.isValid) controller.selection = value.selection;
  return false;
}

TextEditingController adoptController(
  TextEditingController previous, {
  VoidCallback? listener,
  TextEditingController Function(TextEditingValue value)? create,
}) {
  if (listener != null) previous.removeListener(listener);
  final fresh = (create ?? TextEditingController.fromValue)(previous.value);
  if (listener != null) fresh.addListener(listener);
  WidgetsBinding.instance.addPostFrameCallback((_) => previous.dispose());
  return fresh;
}

Size measureText(String text, TextStyle style, TextScaler scaler) {
  final painter = TextPainter(
    text: TextSpan(text: text, style: style),
    textDirection: TextDirection.ltr,
    textScaler: scaler,
  )..layout();
  final size = painter.size;
  painter.dispose();
  return size;
}

Widget composingField(TextEditingController controller, Widget field) => Focus(
  canRequestFocus: false,
  skipTraversal: true,
  onKeyEvent: (_, _) => controller.value.composing.isCollapsed
      ? KeyEventResult.ignored
      : KeyEventResult.skipRemainingHandlers,
  child: field,
);

const activateKeys = <ShortcutActivator, Intent>{
  SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
};

class Segments<T> extends StatelessWidget {
  const Segments({
    super.key,
    required this.options,
    required this.selected,
    required this.onSelect,
    this.dense = false,
  });

  final List<(T, String)> options;
  final T selected;
  final ValueChanged<T> onSelect;

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
    return Semantics(
      selected: selected,
      inMutuallyExclusiveGroup: true,
      child: Shortcuts(
        shortcuts: activateKeys,
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(WsMetrics.radiusMd),
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
