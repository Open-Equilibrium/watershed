import 'rust/api/events.dart';

int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;

String formatDuration(int ms) {
  if (ms < 1000) return '$ms ms';
  if (ms < 59950) return '${(ms / 1000).toStringAsFixed(1)} s';
  final seconds = ms < 60000 ? 60 : ms ~/ 1000;
  String two(int value) => value.toString().padLeft(2, '0');
  final hours = seconds ~/ 3600;
  if (hours > 0) return '${hours}h ${two(seconds % 3600 ~/ 60)}m';
  return '${seconds ~/ 60}m ${two(seconds % 60)}s';
}

String formatCount(int count) => count.toString().replaceAllMapped(
  RegExp(r'\B(?=(\d{3})+(?!\d))'),
  (_) => ',',
);

String formatCost(int microUsd) =>
    '\$${(microUsd / 1e6).toStringAsFixed(microUsd < 10000 ? 3 : 2)}';

String formatTokens(int count) {
  if (count < 1000) return '$count';
  if (count < 999500) return '${(count / 1000).toStringAsPrecision(3)} k';
  final millions = count / 1e6;
  return millions < 999.5
      ? '${millions.toStringAsPrecision(3)} M'
      : '${millions.round()} M';
}

String formatShare(int part, int whole) =>
    whole == 0 ? '0%' : '${(part * 100 / whole).round()}%';

String formatClock(DateTime at) =>
    '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}';

String formatClockMs(int ms) =>
    formatClock(DateTime.fromMillisecondsSinceEpoch(ms));

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

String plural(int count, String noun) => '$count $noun${count == 1 ? '' : 's'}';
