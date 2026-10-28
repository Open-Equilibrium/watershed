import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/theme/theme.dart';
import 'package:liquid/src/theme/tokens.g.dart';

void main() {
  test('each brightness uses its own palette from the tokens', () {
    for (final (brightness, palette) in [
      (Brightness.light, WsPalette.light),
      (Brightness.dark, WsPalette.dark),
    ]) {
      final theme = liquidTheme(brightness);
      expect(theme.extension<WsTheme>()!.palette, same(palette));
      expect(theme.scaffoldBackgroundColor, palette.canvas);
      expect(theme.colorScheme.primary, palette.accent);
      expect(theme.textTheme.bodyMedium!.fontSize, WsMetrics.fsUi);
      expect(
        theme.textTheme.bodyLarge!.height,
        WsMetrics.lhProse / WsMetrics.fsProse,
      );
    }
  });
}
