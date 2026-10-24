import 'package:flutter/material.dart';

import 'tokens.g.dart';

/// Watershed's design tokens for the current brightness (ADR-0194).
@immutable
class WsTheme extends ThemeExtension<WsTheme> {
  const WsTheme(this.palette);

  final WsPalette palette;

  @override
  WsTheme copyWith({WsPalette? palette}) => WsTheme(palette ?? this.palette);

  // Themes switch as a whole; interpolating every token would animate the switch.
  @override
  WsTheme lerp(WsTheme? other, double t) => t < 0.5 ? this : (other ?? this);
}

extension WsThemeContext on BuildContext {
  /// The palette of the current theme.
  WsPalette get ws => Theme.of(this).extension<WsTheme>()!.palette;
}

/// The Material theme for `brightness`, built only from Watershed tokens.
ThemeData liquidTheme(Brightness brightness) {
  final palette = brightness == Brightness.dark
      ? WsPalette.dark
      : WsPalette.light;
  TextStyle style(
    double size,
    double lineHeight,
    FontWeight weight,
    Color color,
  ) => TextStyle(
    fontSize: size,
    height: lineHeight / size,
    fontWeight: weight,
    color: color,
  );
  return ThemeData(
    brightness: brightness,
    scaffoldBackgroundColor: palette.canvas,
    colorScheme: ColorScheme.fromSeed(
      seedColor: palette.accent,
      brightness: brightness,
      primary: palette.accent,
      onPrimary: palette.onAccent,
      surface: palette.canvas,
      onSurface: palette.text,
      error: palette.danger,
    ),
    dividerColor: palette.border,
    splashFactory: NoSplash.splashFactory,
    textTheme: TextTheme(
      displaySmall: style(
        WsMetrics.fsDisplay,
        WsMetrics.lhDisplay,
        WsMetrics.fwBold,
        palette.text,
      ),
      headlineSmall: style(
        WsMetrics.fsH1,
        WsMetrics.lhH1,
        WsMetrics.fwSemibold,
        palette.text,
      ),
      titleLarge: style(
        WsMetrics.fsH2,
        WsMetrics.lhH2,
        WsMetrics.fwSemibold,
        palette.text,
      ),
      titleMedium: style(
        WsMetrics.fsH3,
        WsMetrics.lhH3,
        WsMetrics.fwSemibold,
        palette.text,
      ),
      bodyLarge: style(
        WsMetrics.fsProse,
        WsMetrics.lhProse,
        WsMetrics.fwRegular,
        palette.text,
      ),
      bodyMedium: style(
        WsMetrics.fsUi,
        WsMetrics.lhUi,
        WsMetrics.fwRegular,
        palette.text,
      ),
      bodySmall: style(
        WsMetrics.fsSmall,
        WsMetrics.lhSmall,
        WsMetrics.fwRegular,
        palette.textMuted,
      ),
      labelMedium: style(
        WsMetrics.fsUi,
        WsMetrics.lhUi,
        WsMetrics.fwMedium,
        palette.text,
      ),
    ),
    extensions: [WsTheme(palette)],
  );
}

/// Monospace text for code, paths and tool calls.
TextStyle monoStyle(BuildContext context, {Color? color}) => TextStyle(
  fontFamily: 'Menlo',
  fontFamilyFallback: WsMetrics.fontMono,
  fontSize: WsMetrics.fsMono,
  height: WsMetrics.lhMono / WsMetrics.fsMono,
  color: color ?? context.ws.text,
);
