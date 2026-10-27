// Generated from docs/design/tokens.css by scripts/design_tokens.py. Do not edit.

import 'package:flutter/animation.dart';
import 'package:flutter/painting.dart';

/// Type, spacing, radii, rows and motion shared by both themes.
abstract final class WsMetrics {
  static const List<String> fontUi = [
    'ui-sans-serif',
    '-apple-system',
    'BlinkMacSystemFont',
    'Segoe UI Variable Display',
    'Segoe UI',
    'Helvetica',
    'Apple Color Emoji',
    'Arial',
    'sans-serif',
  ];
  static const List<String> fontMono = [
    'ui-monospace',
    'SFMono-Regular',
    'Menlo',
    'Consolas',
    'monospace',
  ];
  static const double fsDisplay = 32;
  static const double lhDisplay = 40;
  static const double fsH1 = 24;
  static const double lhH1 = 32;
  static const double fsH2 = 20;
  static const double lhH2 = 28;
  static const double fsH3 = 16;
  static const double lhH3 = 22;
  static const double fsProse = 16;
  static const double lhProse = 24;
  static const double fsUi = 14;
  static const double lhUi = 20;
  static const double fsCompact = 13;
  static const double lhCompact = 20;
  static const double fsSmall = 12;
  static const double lhSmall = 18;
  static const double fsCaption = 11;
  static const double lhCaption = 14;
  static const double fsMono = 13;
  static const double lhMono = 20;
  static const FontWeight fwRegular = FontWeight.w400;
  static const FontWeight fwMedium = FontWeight.w500;
  static const FontWeight fwSemibold = FontWeight.w600;
  static const FontWeight fwBold = FontWeight.w700;
  static const double space1 = 4;
  static const double space2 = 6;
  static const double space3 = 8;
  static const double space4 = 12;
  static const double space5 = 16;
  static const double space6 = 20;
  static const double space7 = 24;
  static const double space8 = 32;
  static const double radiusSm = 4;
  static const double radiusMd = 6;
  static const double radiusLg = 8;
  static const double radiusXl = 12;
  static const double rowNav = 28;
  static const double rowTable = 34;
  static const Duration motionFast = Duration(milliseconds: 120);
  static const Duration motionPanel = Duration(milliseconds: 200);
  static const Cubic ease = Cubic(0, 0, 0.2, 1);
}

/// The colors and shadows of one theme.
final class WsPalette {
  const WsPalette({
    required this.canvas,
    required this.subtle,
    required this.surface,
    required this.hover,
    required this.selected,
    required this.titlebar,
    required this.border,
    required this.borderStrong,
    required this.text,
    required this.textMuted,
    required this.textFaint,
    required this.accent,
    required this.onAccent,
    required this.accentSubtle,
    required this.success,
    required this.successSubtle,
    required this.warning,
    required this.warningSubtle,
    required this.warningBorder,
    required this.danger,
    required this.dangerSubtle,
    required this.diffAdd,
    required this.diffAddStrong,
    required this.diffDel,
    required this.diffDelStrong,
    required this.code,
    required this.synKw,
    required this.synStr,
    required this.synCom,
    required this.synNum,
    required this.data1,
    required this.data2,
    required this.data3,
    required this.onData,
    required this.shadow,
  });

  final Color canvas;
  final Color subtle;
  final Color surface;
  final Color hover;
  final Color selected;
  final Color titlebar;
  final Color border;
  final Color borderStrong;
  final Color text;
  final Color textMuted;
  final Color textFaint;
  final Color accent;
  final Color onAccent;
  final Color accentSubtle;
  final Color success;
  final Color successSubtle;
  final Color warning;
  final Color warningSubtle;
  final Color warningBorder;
  final Color danger;
  final Color dangerSubtle;
  final Color diffAdd;
  final Color diffAddStrong;
  final Color diffDel;
  final Color diffDelStrong;
  final Color code;
  final Color synKw;
  final Color synStr;
  final Color synCom;
  final Color synNum;
  final Color data1;
  final Color data2;
  final Color data3;
  final Color onData;
  final List<BoxShadow> shadow;

  static const light = WsPalette(
    canvas: Color(0xFFFFFFFF),
    subtle: Color(0xFFF7F7F5),
    surface: Color(0xFFFFFFFF),
    hover: Color(0xFFEFEFED),
    selected: Color(0xFFE3EEFB),
    titlebar: Color(0xFFF7F7F5),
    border: Color(0xFFE9E9E7),
    borderStrong: Color(0xFFDCDBD8),
    text: Color(0xFF37352F),
    textMuted: Color(0xFF504F4C),
    textFaint: Color(0xFF6B6A67),
    accent: Color(0xFF1B63B7),
    onAccent: Color(0xFFFFFFFF),
    accentSubtle: Color(0xFFE7F0FB),
    success: Color(0xFF376E4F),
    successSubtle: Color(0xFFEDF3EC),
    warning: Color(0xFF9A4F00),
    warningSubtle: Color(0xFFFBECDD),
    warningBorder: Color(0xFFF2C9A0),
    danger: Color(0xFFB03A35),
    dangerSubtle: Color(0xFFFDEBEC),
    diffAdd: Color(0xFFE6FFEC),
    diffAddStrong: Color(0xFFCCFFD8),
    diffDel: Color(0xFFFFEBE9),
    diffDelStrong: Color(0xFFFFD7D5),
    code: Color(0xFFF7F6F3),
    synKw: Color(0xFFCF222E),
    synStr: Color(0xFF0A3069),
    synCom: Color(0xFF5F6873),
    synNum: Color(0xFF0550AE),
    data1: Color(0xFF2075D8),
    data2: Color(0xFF8A5FAD),
    data3: Color(0xFF3D7A58),
    onData: Color(0xFFFFFFFF),
    shadow: [
      BoxShadow(
        offset: Offset(0, 0),
        blurRadius: 0,
        spreadRadius: 1,
        color: Color(0x0D0F0F0F),
      ),
      BoxShadow(
        offset: Offset(0, 3),
        blurRadius: 6,
        spreadRadius: 0,
        color: Color(0x1A0F0F0F),
      ),
      BoxShadow(
        offset: Offset(0, 9),
        blurRadius: 24,
        spreadRadius: 0,
        color: Color(0x330F0F0F),
      ),
    ],
  );

  static const dark = WsPalette(
    canvas: Color(0xFF191919),
    subtle: Color(0xFF202020),
    surface: Color(0xFF252525),
    hover: Color(0xFF2C2C2C),
    selected: Color(0xFF1F3550),
    titlebar: Color(0xFF202020),
    border: Color(0xFF2F2F2F),
    borderStrong: Color(0xFF3A3A3A),
    text: Color(0xFFD4D4D4),
    textMuted: Color(0xFFB7B7B7),
    textFaint: Color(0xFF9D9D9D),
    accent: Color(0xFF4C9BEA),
    onAccent: Color(0xFF0B1626),
    accentSubtle: Color(0xFF1D2C3D),
    success: Color(0xFF4DAB9A),
    successSubtle: Color(0xFF1C302B),
    warning: Color(0xFFFFA344),
    warningSubtle: Color(0xFF3A2A17),
    warningBorder: Color(0xFF6A4A22),
    danger: Color(0xFFFF7369),
    dangerSubtle: Color(0xFF3F2523),
    diffAdd: Color(0xFF1A2E22),
    diffAddStrong: Color(0xFF23472F),
    diffDel: Color(0xFF3A1F1F),
    diffDelStrong: Color(0xFF5C2B2B),
    code: Color(0xFF202020),
    synKw: Color(0xFFFF7B72),
    synStr: Color(0xFFA5D6FF),
    synCom: Color(0xFF8B949E),
    synNum: Color(0xFF79C0FF),
    data1: Color(0xFF4C9BEA),
    data2: Color(0xFFB48CE6),
    data3: Color(0xFF4DAB9A),
    onData: Color(0xFF191919),
    shadow: [
      BoxShadow(
        offset: Offset(0, 0),
        blurRadius: 0,
        spreadRadius: 1,
        color: Color(0x12FFFFFF),
      ),
      BoxShadow(
        offset: Offset(0, 3),
        blurRadius: 6,
        spreadRadius: 0,
        color: Color(0x4D000000),
      ),
      BoxShadow(
        offset: Offset(0, 9),
        blurRadius: 24,
        spreadRadius: 0,
        color: Color(0x66000000),
      ),
    ],
  );
}
