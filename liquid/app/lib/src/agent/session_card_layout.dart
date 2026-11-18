import 'dart:math' as math;
import 'dart:ui' show Size;

import '../theme/tokens.g.dart';

const double cardWidth = 280;

const cardTapGroup = Object();

const double cardInset = 12;

const double cardClearance = 32;

const double lanePadding = 48;

const double minChatWidth = 640;
const double minCardHeight = 160;

enum CardDensity { full, compact, essential }

CardDensity resolveCardDensity(
  double height, {
  required double full,
  required double compact,
}) {
  if (full == 0 || full <= height) return CardDensity.full;
  if (compact == 0 || compact <= height) return CardDensity.compact;
  return CardDensity.essential;
}

bool cardFitsBeside(Size canvas) =>
    canvas.height - 2 * cardInset >= minCardHeight &&
    canvas.width - cardWidth - cardInset - cardClearance - lanePadding >=
        minChatWidth;

({double left, double width}) chatLane(
  double canvasWidth, {
  required bool beside,
}) {
  final laneRight = beside
      ? canvasWidth - cardInset - cardWidth - cardClearance
      : canvasWidth - lanePadding;
  final width = math.min(WsMetrics.widthReading, laneRight - lanePadding);
  final left = math.max(
    lanePadding,
    math.min((canvasWidth - width) / 2, laneRight - width),
  );
  return (left: left, width: width);
}
