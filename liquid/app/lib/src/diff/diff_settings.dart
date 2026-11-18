import 'package:flutter/foundation.dart';

import '../settings_file.dart';

enum DiffLayout { unified, split }

@immutable
class DiffSettings {
  const DiffSettings({
    this.layout = DiffLayout.unified,
    this.hideWhitespace = false,
    this.width,
  });

  factory DiffSettings.fromJson(Map<String, Object?> json) => DiffSettings(
    layout: DiffLayout.values.asNameMap()[json['layout']] ?? DiffLayout.unified,
    hideWhitespace: json['hideWhitespace'] == true,
    width: switch (json['width']) {
      final num width when width.isFinite => width.toDouble(),
      _ => null,
    },
  );

  final DiffLayout layout;

  final bool hideWhitespace;

  final double? width;

  DiffSettings copyWith({
    DiffLayout? layout,
    bool? hideWhitespace,
    double? width,
  }) => DiffSettings(
    layout: layout ?? this.layout,
    hideWhitespace: hideWhitespace ?? this.hideWhitespace,
    width: width ?? this.width,
  );

  Map<String, Object?> toJson() => {
    'layout': layout.name,
    'hideWhitespace': hideWhitespace,
    'width': ?width,
  };
}

class DiffSettingsController extends SavedSettings<DiffSettings> {
  DiffSettingsController([
    DiffSettings value = const DiffSettings(),
    SettingsFile? file,
  ]) : super(
         value,
         file == null ? null : (settings) => file.save(settings.toJson()),
         'the Diff settings',
       );

  static Future<DiffSettingsController> load(SettingsFile file) async {
    final json = await file.load();
    return DiffSettingsController(
      json == null ? const DiffSettings() : DiffSettings.fromJson(json),
      file,
    );
  }
}
