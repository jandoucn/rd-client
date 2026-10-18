import 'package:flutter/material.dart';
import 'package:flutter_hbb/mobile/widgets/gesture_help.dart';
import 'package:flutter_test/flutter_test.dart';

// Distinct output proves that labels pass through the translation boundary.
String _localized(String key) => 'localized:$key';

Future<void> _pumpCards(WidgetTester tester,
    {required bool android, required bool touchMode}) async {
  await tester.pumpWidget(MaterialApp(
    home: Scaffold(
      body: GestureHelpCards(
        width: 100,
        android: android,
        touchMode: touchMode,
        translator: _localized,
      ),
    ),
  ));
  expect(tester.takeException(), isNull);
}

void _expectCard(String from, String to, IconData icon) {
  final card = find.ancestor(
    of: find.text(_localized(to)),
    matching: find.byType(GestureInfo),
  );
  expect(card, findsOneWidget);
  expect(find.descendant(of: card, matching: find.text(_localized(from))),
      findsOneWidget);
  expect(find.descendant(of: card, matching: find.byIcon(icon)), findsOneWidget);
}

void _expectZoom() {
  _expectCard('Pinch to Zoom', 'Canvas Zoom', GestureIcons.iconGesturePinch);
}

void main() {
  for (final touchMode in [false, true]) {
    testWidgets('Android wheel uses localized two-finger vertical card '
        'touchMode=$touchMode', (tester) async {
      await _pumpCards(tester, android: true, touchMode: touchMode);
      _expectCard('Two-finger vertical swipe', 'Mouse Wheel',
          GestureIcons.iconGestureFDragUpDown_);
      expect(find.text(_localized('Three-Finger vertically')), findsNothing);
      expect(find.byIcon(GestureIcons.iconGestureFThreeFingers), findsNothing);
      _expectZoom();
    });
  }

  testWidgets('Android mouse omits canvas move and retains pinch zoom',
      (tester) async {
    await _pumpCards(tester, android: true, touchMode: false);
    expect(find.text(_localized('Canvas Move')), findsNothing);
    expect(find.text(_localized('Two-Finger Move')), findsNothing);
    expect(find.byType(GestureInfo), findsNWidgets(5));
    _expectZoom();
  });

  testWidgets('Android touch omits canvas move and retains pinch zoom',
      (tester) async {
    await _pumpCards(tester, android: true, touchMode: true);
    expect(find.text(_localized('Canvas Move')), findsNothing);
    expect(find.text(_localized('Two-Finger Move')), findsNothing);
    _expectCard('One-Finger Move', 'Mouse Drag',
        GestureIcons.iconGestureFSwipeRight);
    expect(find.byType(GestureInfo), findsNWidgets(5));
    _expectZoom();
  });

  for (final touchMode in [false, true]) {
    testWidgets('non-Android keeps original cards touchMode=$touchMode',
        (tester) async {
      await _pumpCards(tester, android: false, touchMode: touchMode);
      _expectCard('Three-Finger vertically', 'Mouse Wheel',
          GestureIcons.iconGestureFThreeFingers);
      _expectCard('Two-Finger Move', 'Canvas Move', GestureIcons.iconGestureFDrag);
      _expectCard(touchMode ? 'One-Finger Move' : 'Double Tap & Move',
          'Mouse Drag', GestureIcons.iconGestureFSwipeRight);
      _expectCard('One-Finger Tap', 'Left Mouse', GestureIcons.iconMobileTouch);
      _expectCard('One-Long Tap', 'Right Mouse', GestureIcons.iconGesturePressHold);
      expect(find.text(_localized('Two-finger vertical swipe')), findsNothing);
      expect(find.byType(GestureInfo), findsNWidgets(6));
      _expectZoom();
    });
  }
}
