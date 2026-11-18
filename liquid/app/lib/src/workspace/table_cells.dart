import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../rust/api/database.dart';
import '../theme/icons.dart';
import '../theme/theme.dart';
import '../theme/tokens.g.dart';
import 'file_cells.dart';

String numberText(NumberFormat format, double number) {
  if (!number.isFinite) return format.format(number);
  final [mantissa, ...exponent] = number.abs().toString().split('e');
  final [whole, ...fraction] = mantissa.split('.');
  var digits = whole + fraction.join();
  var point =
      whole.length + (exponent.isEmpty ? 0 : int.parse(exponent.single));
  if (point <= 0) {
    digits = '0' * (1 - point) + digits;
    point = 1;
  }
  digits = digits.padRight(point, '0');
  final integer = digits.substring(0, point);
  final decimals = digits.substring(point).replaceFirst(RegExp(r'0+$'), '');
  final zero = format.symbols.ZERO_DIGIT.codeUnitAt(0);
  String local(String digits) =>
      String.fromCharCodes(digits.codeUnits.map((unit) => unit - 0x30 + zero));
  final text = [
    if (integer.length <= 18)
      format.format(int.parse(integer))
    else
      local(integer),
    if (decimals.isNotEmpty) ...[format.symbols.DECIMAL_SEP, local(decimals)],
  ].join();
  return number < 0
      ? '${format.negativePrefix}$text${format.negativeSuffix}'
      : text;
}

String intlLocale(Locale locale, bool Function(String) exists) {
  final Locale(:languageCode, :scriptCode, :countryCode) = locale;
  return [
        locale.toString(),
        if (scriptCode != null) '${languageCode}_$scriptCode',
        if (countryCode != null) '${languageCode}_$countryCode',
        languageCode,
      ].where(exists).firstOrNull ??
      'en_US';
}

Widget cellText(String text, {TextStyle? style}) =>
    Text(text, maxLines: 1, overflow: TextOverflow.ellipsis, style: style);

class CellView extends StatelessWidget {
  const CellView({
    super.key,
    required this.cell,
    required this.kind,
    required this.options,
    required this.numbers,
    required this.days,
  });

  final BridgeCell? cell;

  final BridgeFieldKind kind;

  final Map<String, BridgeOption> options;
  final NumberFormat numbers;
  final DateFormat days;

  @override
  Widget build(BuildContext context) {
    final cell = this.cell;
    return switch (cell?.kind) {
      null || BridgeCellKind.empty when kind == BridgeFieldKind.checkbox =>
        const CellCheckbox(checked: false),
      null || BridgeCellKind.empty => const SizedBox.shrink(),
      BridgeCellKind.text => cellText(cell!.text ?? ''),
      BridgeCellKind.number => cellText(
        numberText(numbers, cell!.number),
        style: const TextStyle(fontFeatures: [FontFeature.tabularFigures()]),
      ),
      BridgeCellKind.select => switch (options[cell!.text]) {
        final option? => OptionTag(option),
        null => const SizedBox.shrink(),
      },
      BridgeCellKind.date => cellText(switch (DateTime.tryParse(
        cell!.text ?? '',
      )) {
        final day? => days.format(day),
        null => cell.text ?? '',
      }),
      BridgeCellKind.checkbox => CellCheckbox(checked: cell!.checked),
      BridgeCellKind.files => FileChips(cell!.files),
    };
  }
}

class OptionTag extends StatelessWidget {
  const OptionTag(this.option, {super.key});

  final BridgeOption option;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    final colors = [
      ws.tag0,
      ws.tag1,
      ws.tag2,
      ws.tag3,
      ws.tag4,
      ws.tag5,
      ws.tag6,
      ws.tag7,
      ws.tag8,
      ws.tag9,
    ];
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: WsMetrics.space2),
      decoration: BoxDecoration(
        color: colors[option.color % colors.length],
        borderRadius: BorderRadius.circular(WsMetrics.radiusSm),
      ),
      child: Text(
        option.name,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          fontSize: WsMetrics.fsCompact,
          height: 20 / WsMetrics.fsCompact,
          color: ws.tagText,
        ),
      ),
    );
  }
}

class CellCheckbox extends StatelessWidget {
  const CellCheckbox({super.key, required this.checked});

  final bool checked;

  @override
  Widget build(BuildContext context) {
    final ws = context.ws;
    return Semantics(
      checked: checked,
      child: Container(
        width: 14,
        height: 14,
        decoration: BoxDecoration(
          color: checked ? ws.accent : null,
          borderRadius: BorderRadius.circular(3),
          border: checked
              ? null
              : Border.all(color: ws.borderStrong, width: 1.5),
        ),
        child: checked
            ? Icon(LucideIcons.check, size: 10, color: ws.onAccent)
            : null,
      ),
    );
  }
}
