import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/agent/session_card_layout.dart';

void main() {
  test(
    'the card sits beside the chat only where the chat lane keeps its width',
    () {
      // The chat lane keeps 640 px: the window minus 280 (card), 12 (gap), 32 (clearance) and 48
      // (lane padding) on each side that T3 Code counts.
      expect(cardFitsBeside(const Size(1011, 600)), isFalse);
      expect(cardFitsBeside(const Size(1012, 600)), isTrue);
    },
  );

  test('the card sits beside the chat only where it has 160 px of height', () {
    expect(cardFitsBeside(const Size(1600, 183)), isFalse);
    expect(cardFitsBeside(const Size(1600, 184)), isTrue);
  });

  test(
    'the density steps follow T3 Code: full, then compact, then essential',
    () {
      expect(
        resolveCardDensity(500, full: 300, compact: 80),
        CardDensity.full,
        reason: 'the full card fits',
      );
      expect(
        resolveCardDensity(100, full: 300, compact: 80),
        CardDensity.compact,
      );
      expect(
        resolveCardDensity(50, full: 300, compact: 80),
        CardDensity.essential,
      );
      expect(
        resolveCardDensity(50, full: 0, compact: 0),
        CardDensity.full,
        reason: 'a height not measured yet starts at full',
      );
    },
  );

  test(
    'the chat lane stays clear of a card beside it, as T3 Code lays it out',
    () {
      final centred = chatLane(1600, beside: false);
      expect(centred.width, 708);
      expect(centred.left, 446, reason: 'centred on the canvas');

      final beside = chatLane(1200, beside: true);
      expect(beside.width, 708);
      expect(
        beside.left,
        168,
        reason: 'pushed left to clear the card and its clearance',
      );

      final narrow = chatLane(1012, beside: true);
      expect(narrow.width, 640, reason: 'shrinks to the minimum lane width');
    },
  );
}
