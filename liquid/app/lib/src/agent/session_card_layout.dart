import 'dart:math' as math;
import 'dart:ui' show Size;

import '../theme/tokens.g.dart';

// The sizes of T3 Code's thread details card (threadDetailsCardLayout.ts, chatCanvasLayout.ts).
const double cardWidth = 280;

/// The gap from the card to the top and right edges of the chat.
const double cardInset = 12;

/// The space the chat lane keeps clear of a card beside it.
const double cardClearance = 32;

/// The chat lane's padding on each side, as T3 Code's chat lane has at desktop widths.
const double lanePadding = 48;

const double minChatWidth = 640;
const double minCardHeight = 160;

/// How much of the card T3 Code shows: the full card, then the compact one, then the essential
/// rows only.
enum CardDensity { full, compact, essential }

/// The densest step whose content fits `height`; content not measured yet counts as fitting.
CardDensity resolveCardDensity(
  double height, {
  required double full,
  required double compact,
}) {
  if (full == 0 || full <= height) return CardDensity.full;
  if (compact == 0 || compact <= height) return CardDensity.compact;
  return CardDensity.essential;
}

/// Whether the card sits beside the chat rather than over it as a popover: the chat lane keeps
/// its minimum width beside the card, and the card its minimum height.
bool cardFitsBeside(Size canvas) =>
    canvas.height - 2 * cardInset >= minCardHeight &&
    canvas.width - cardWidth - cardInset - cardClearance - lanePadding >=
        minChatWidth;

/// The chat's reading lane in a canvas `canvasWidth` wide: centred, and pushed left to clear the
/// card when the card sits beside the chat.
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

/// Where the card sits: beside the chat, or as a popover over it. Written by the layout, read by
/// a press outside the card.
class CardPlacement {
  bool beside = true;
}
