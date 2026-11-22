import 'package:flutter_test/flutter_test.dart';
import 'package:super_editor/super_editor.dart';

// Documents an upstream bug the randomized test hit: attributed_text 0.4.7
// leaves two adjacent bold spans unmerged after editing, and adding bold over
// both then drops the mark from the last character. Flips when fixed.
void main() {
  test('attributed_text 0.4.7 drops bold when it overlaps two unmerged adjacent spans', () {
    final text = AttributedText(
      'abcdefg',
      AttributedSpans(
        attributions: [
          const SpanMarker(attribution: boldAttribution, offset: 0, markerType: SpanMarkerType.start),
          const SpanMarker(attribution: boldAttribution, offset: 4, markerType: SpanMarkerType.end),
          const SpanMarker(attribution: boldAttribution, offset: 5, markerType: SpanMarkerType.start),
          const SpanMarker(attribution: boldAttribution, offset: 5, markerType: SpanMarkerType.end),
        ],
      ),
    );
    text.addAttribution(boldAttribution, const SpanRange(5, 6));
    expect(text.getAllAttributionsAt(6), isNot(contains(boldAttribution)));
  });
}
