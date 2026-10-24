import 'package:flutter_test/flutter_test.dart';
import 'package:liquid/src/app.dart';

void main() {
  testWidgets('starts without the native library', (tester) async {
    await tester.pumpWidget(const LiquidApp());
    expect(tester.takeException(), isNull);
  });
}
