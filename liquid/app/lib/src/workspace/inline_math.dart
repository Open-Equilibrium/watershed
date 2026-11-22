import 'package:flutter/material.dart';
import 'package:flutter_math_fork/flutter_math.dart';

final inlineMath = RegExp(
  r'(?<![\\$])\$(?![\s$])((?:\\.|[^$\\])+?)(?<![\s\\])\$(?![\d$])',
);

class MathText extends TextEditingController {
  MathText.fromValue(super.value) : super.fromValue();

  bool focused = false;

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final text = this.text;
    final found = inlineMath.allMatches(text).toList();
    if (found.isEmpty || !value.composing.isCollapsed) {
      return super.buildTextSpan(
        context: context,
        style: style,
        withComposing: withComposing,
      );
    }
    final caret = selection.isValid ? selection.baseOffset : -1;
    final hidden = (style ?? const TextStyle()).copyWith(
      fontSize: 0.01,
      color: const Color(0x00000000),
      letterSpacing: 0,
      decoration: TextDecoration.none,
    );
    final children = <InlineSpan>[];
    var at = 0;
    for (final match in found) {
      final editing = focused && caret >= match.start && caret <= match.end;
      if (editing) continue;
      if (match.start > at) {
        children.add(TextSpan(text: text.substring(at, match.start)));
      }
      final latex = match[1]!;
      children
        ..add(
          WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: Math.tex(
              latex,
              mathStyle: MathStyle.text,
              textStyle: style,
              onErrorFallback: (_) => Text(match[0]!, style: style),
            ),
          ),
        )
        ..add(
          TextSpan(
            text: text.substring(match.start + 1, match.end),
            style: hidden,
          ),
        );
      at = match.end;
    }
    if (at < text.length) children.add(TextSpan(text: text.substring(at)));
    return TextSpan(style: style, children: children);
  }
}
