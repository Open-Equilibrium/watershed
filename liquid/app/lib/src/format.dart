import 'rust/api/events.dart';

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;

/// A duration as milliseconds, seconds or minutes and seconds.
String formatDuration(int ms) {
  if (ms < 1000) return '$ms ms';
  // Seconds keep one decimal until they would round to a whole minute.
  if (ms < 59950) return '${(ms / 1000).toStringAsFixed(1)} s';
  final seconds = ms < 60000 ? 60 : ms ~/ 1000;
  return '${seconds ~/ 60}m ${(seconds % 60).toString().padLeft(2, '0')}s';
}

/// A count with thousands separators.
String formatCount(int count) => count.toString().replaceAllMapped(
  RegExp(r'\B(?=(\d{3})+(?!\d))'),
  (_) => ',',
);

/// A cost in millionths of a dollar; amounts under a cent keep three decimals.
String formatCost(int microUsd) =>
    '\$${(microUsd / 1e6).toStringAsFixed(microUsd < 10000 ? 3 : 2)}';

/// A count in thousands once it reaches them, such as `6.1k`.
String formatCompact(int count) =>
    count >= 1000 ? '${(count / 1000).toStringAsFixed(1)}k' : '$count';

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
