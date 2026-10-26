import 'rust/api/events.dart';

/// Tokens a model request read: fresh input plus cache reads and writes.
int promptTokens(BridgeUsage usage) =>
    usage.input + usage.cacheRead + usage.cacheWrite;

/// A duration as milliseconds, seconds or minutes and seconds.
String formatDuration(int ms) {
  if (ms < 1000) return '$ms ms';
  if (ms < 60000) return '${(ms / 1000).toStringAsFixed(1)} s';
  final seconds = ms ~/ 1000;
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
