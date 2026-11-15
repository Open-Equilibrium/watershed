import 'rust/api/events.dart';

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;

/// A duration as milliseconds, seconds, minutes and seconds, or hours and minutes.
String formatDuration(int ms) {
  if (ms < 1000) return '$ms ms';
  // Seconds keep one decimal until they would round to a whole minute.
  if (ms < 59950) return '${(ms / 1000).toStringAsFixed(1)} s';
  final seconds = ms < 60000 ? 60 : ms ~/ 1000;
  String two(int value) => value.toString().padLeft(2, '0');
  final hours = seconds ~/ 3600;
  if (hours > 0) return '${hours}h ${two(seconds % 3600 ~/ 60)}m';
  return '${seconds ~/ 60}m ${two(seconds % 60)}s';
}

/// A count with thousands separators.
String formatCount(int count) => count.toString().replaceAllMapped(
  RegExp(r'\B(?=(\d{3})+(?!\d))'),
  (_) => ',',
);

/// A cost in millionths of a dollar; amounts under a cent keep three decimals.
String formatCost(int microUsd) =>
    '\$${(microUsd / 1e6).toStringAsFixed(microUsd < 10000 ? 3 : 2)}';

/// A token count with three significant digits and a unit, such as `212 k` or `1.85 M`.
/// From 999.5 M on, whole millions: three digits would need a larger unit, and
/// `toStringAsPrecision` switches to exponent notation there.
String formatTokens(int count) {
  if (count < 1000) return '$count';
  if (count < 999500) return '${(count / 1000).toStringAsPrecision(3)} k';
  final millions = count / 1e6;
  return millions < 999.5
      ? '${millions.toStringAsPrecision(3)} M'
      : '${millions.round()} M';
}

/// A part's share of a whole, rounded to a whole percent, such as `95%`.
String formatShare(int part, int whole) =>
    whole == 0 ? '0%' : '${(part * 100 / whole).round()}%';

/// A local time of day, such as `10:58`.
String formatClock(DateTime at) =>
    '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}';

/// A local time of day from Unix milliseconds.
String formatClockMs(int ms) =>
    formatClock(DateTime.fromMillisecondsSinceEpoch(ms));

/// How long before `now` the Unix millisecond time `ms` was, such as `5 h ago`.
String formatAgo(int ms, DateTime now) {
  final ago = now.difference(DateTime.fromMillisecondsSinceEpoch(ms));
  String count(int value, String unit) =>
      '$value $unit${value == 1 || unit == 'h' || unit == 'min' ? '' : 's'} ago';
  return switch (ago.inMinutes) {
    < 1 => 'now',
    < 60 => count(ago.inMinutes, 'min'),
    < 60 * 24 => count(ago.inHours, 'h'),
    < 60 * 24 * 14 => count(ago.inDays, 'day'),
    < 60 * 24 * 60 => count(ago.inDays ~/ 7, 'week'),
    < 60 * 24 * 365 => count(ago.inDays ~/ 30, 'month'),
    _ => count(ago.inDays ~/ 365, 'year'),
  };
}

/// A count and a noun with its regular plural.
String plural(int count, String noun) => '$count $noun${count == 1 ? '' : 's'}';
