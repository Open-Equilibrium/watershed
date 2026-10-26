import 'package:flutter/material.dart';

import 'agent/shortcut_reveal.dart';
import 'theme/theme.dart';
import 'theme/tokens.g.dart';

/// A contextual shortcut hint, shown only while the reveal key is held (ADR-0204). It takes no space, so
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
          height: 14 / 10,
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
    this.shortcut,
    this.focusNode,
  });

  final String label;
  final String? shortcut;
  final VoidCallback onPressed;
  final Color foreground;
  final Color background;
  final Color? outline;

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
          padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            // Centered when a layout stretches the button.
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Text(
                label,
                style: TextStyle(
                  color: foreground,
                  fontWeight: WsMetrics.fwMedium,
                  fontSize: WsMetrics.fsCompact,
                ),
              ),
            ],
          ),
        ),
      ),
    );
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
  final VoidCallback onPressed;
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

class SecondaryButton extends StatelessWidget {
  const SecondaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.shortcut,
    this.color,
    this.focusNode,
  });

  final String label;
  final String? shortcut;
  final Color? color;
  final VoidCallback onPressed;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) => _Button(
    label: label,
    shortcut: shortcut,
    onPressed: onPressed,
    focusNode: focusNode,
    foreground: color ?? context.ws.text,
    background: context.ws.canvas,
    outline: context.ws.borderStrong,
  );
}
