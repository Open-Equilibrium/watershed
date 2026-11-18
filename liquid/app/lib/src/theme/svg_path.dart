import 'dart:math';
import 'dart:ui';

Path svgPath(String data) {
  final path = Path();
  final scan = _Scan(data);
  var x = 0.0;
  var y = 0.0;
  var startX = 0.0;
  var startY = 0.0;
  var controlX = 0.0;
  var controlY = 0.0;
  var previous = '';
  var command = '';
  while (scan.skipSeparators()) {
    if (scan.atLetter) {
      command = scan.letter();
    } else if (command == 'Z' || command == 'z') {
      throw FormatException('Numbers after a close in $data');
    } else if (command == 'M' || command == 'm') {
      command = command == 'M' ? 'L' : 'l';
    }
    final relative = command.toLowerCase() == command;
    final baseX = relative ? x : 0.0;
    final baseY = relative ? y : 0.0;
    switch (command.toUpperCase()) {
      case 'M':
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.moveTo(x, y);
        startX = x;
        startY = y;
      case 'L':
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.lineTo(x, y);
      case 'H':
        x = baseX + scan.number();
        path.lineTo(x, y);
      case 'V':
        y = baseY + scan.number();
        path.lineTo(x, y);
      case 'C':
        final x1 = baseX + scan.number();
        final y1 = baseY + scan.number();
        controlX = baseX + scan.number();
        controlY = baseY + scan.number();
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.cubicTo(x1, y1, controlX, controlY, x, y);
      case 'S':
        final reflected = previous == 'C' || previous == 'S';
        final x1 = reflected ? 2 * x - controlX : x;
        final y1 = reflected ? 2 * y - controlY : y;
        controlX = baseX + scan.number();
        controlY = baseY + scan.number();
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.cubicTo(x1, y1, controlX, controlY, x, y);
      case 'Q':
        controlX = baseX + scan.number();
        controlY = baseY + scan.number();
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.quadraticBezierTo(controlX, controlY, x, y);
      case 'T':
        final reflected = previous == 'Q' || previous == 'T';
        final x1 = reflected ? 2 * x - controlX : x;
        final y1 = reflected ? 2 * y - controlY : y;
        controlX = x1;
        controlY = y1;
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.quadraticBezierTo(controlX, controlY, x, y);
      case 'A':
        final radiusX = scan.number().abs();
        final radiusY = scan.number().abs();
        final rotation = scan.number();
        final largeArc = scan.flag();
        final clockwise = scan.flag();
        x = baseX + scan.number();
        y = baseY + scan.number();
        path.arcToPoint(
          Offset(x, y),
          radius: Radius.elliptical(radiusX, radiusY),
          rotation: rotation * pi / 180,
          largeArc: largeArc,
          clockwise: clockwise,
        );
      case 'Z':
        path.close();
        x = startX;
        y = startY;
      default:
        throw FormatException('Unknown command "$command" in $data');
    }
    previous = command.toUpperCase();
  }
  return path;
}

class _Scan {
  _Scan(this._data);

  final String _data;
  var _at = 0;

  bool skipSeparators() {
    while (_at < _data.length && ' \t\r\n,'.contains(_data[_at])) {
      _at++;
    }
    return _at < _data.length;
  }

  bool get atLetter {
    if (_at >= _data.length) return false;
    final unit = _data.codeUnitAt(_at);
    return (unit >= 65 && unit <= 90) || (unit >= 97 && unit <= 122);
  }

  String letter() => _data[_at++];

  double number() {
    skipSeparators();
    final start = _at;
    if (_at < _data.length && '+-'.contains(_data[_at])) _at++;
    _digits();
    if (_at < _data.length && _data[_at] == '.') {
      _at++;
      _digits();
    }
    if (_at < _data.length && 'eE'.contains(_data[_at])) {
      _at++;
      if (_at < _data.length && '+-'.contains(_data[_at])) _at++;
      _digits();
    }
    final text = _data.substring(start, _at);
    final value = double.tryParse(
      text.replaceFirstMapped(
        RegExp(r'^([+-]?)\.'),
        (match) => '${match[1]}0.',
      ),
    );
    if (value == null) {
      throw FormatException('Expected a number in $_data at $start');
    }
    return value;
  }

  bool flag() {
    skipSeparators();
    final digit = _data[_at++];
    if (digit == '0') return false;
    if (digit == '1') return true;
    throw FormatException('Expected an arc flag in $_data');
  }

  void _digits() {
    while (_at < _data.length &&
        _data.codeUnitAt(_at) >= 48 &&
        _data.codeUnitAt(_at) <= 57) {
      _at++;
    }
  }
}
