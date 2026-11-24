import 'package:flutter/gestures.dart';

class BoardClaim extends OneSequenceGestureRecognizer {
  bool Function(PointerDownEvent event) claims = (_) => false;
  bool panZooms = true;
  void Function(Offset pan, double scale, Offset focal)? onPanZoom;

  Offset _pan = Offset.zero;
  double _scale = 1;

  @override
  bool isPointerAllowed(PointerDownEvent event) => claims(event);

  @override
  void addAllowedPointer(PointerDownEvent event) {
    startTrackingPointer(event.pointer, event.transform);
    resolve(GestureDisposition.accepted);
  }

  @override
  bool isPointerPanZoomAllowed(PointerPanZoomStartEvent event) => panZooms;

  @override
  void addAllowedPointerPanZoom(PointerPanZoomStartEvent event) {
    startTrackingPointer(event.pointer, event.transform);
    resolve(GestureDisposition.accepted);
    _pan = Offset.zero;
    _scale = 1;
  }

  @override
  void handleEvent(PointerEvent event) {
    if (event is PointerPanZoomUpdateEvent) {
      onPanZoom?.call(
        event.pan - _pan,
        event.scale / _scale,
        event.localPosition,
      );
      _pan = event.pan;
      _scale = event.scale;
    } else if (event is PointerUpEvent ||
        event is PointerCancelEvent ||
        event is PointerPanZoomEndEvent) {
      stopTrackingPointer(event.pointer);
    }
  }

  @override
  void didStopTrackingLastPointer(int pointer) {}

  @override
  String get debugDescription => 'whiteboard pointers';
}
