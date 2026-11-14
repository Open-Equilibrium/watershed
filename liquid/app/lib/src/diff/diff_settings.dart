import 'package:flutter/foundation.dart';

import '../settings_file.dart';

enum DiffLayout { unified, split }

/// How the Diff panel shows a run's changes; kept across restarts.
@immutable
class DiffSettings {
  const DiffSettings({
    this.layout = DiffLayout.unified,
    this.hideWhitespace = false,
    this.width,
  });

  /// Reads what it knows from `json` and keeps the defaults for the rest.
  factory DiffSettings.fromJson(Map<String, Object?> json) => DiffSettings(
    layout: DiffLayout.values.asNameMap()[json['layout']] ?? DiffLayout.unified,
    hideWhitespace: json['hideWhitespace'] == true,
    width: json['width'] is num ? (json['width']! as num).toDouble() : null,
  );

  final DiffLayout layout;

  /// Whether changes that are only whitespace are left out.
  final bool hideWhitespace;

  /// The panel's width as the maintainer dragged it; null until dragged.
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

/// The Diff panel's settings in use; each change is saved.
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
