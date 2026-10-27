import 'package:flutter/foundation.dart';

import '../settings_file.dart';

enum DiffLayout { unified, split }

/// How the Diff tab shows a run's changes; kept across restarts (ADR-0212).
@immutable
class DiffSettings {
  const DiffSettings({
    this.layout = DiffLayout.unified,
    this.hideWhitespace = false,
  });

  /// Reads what it knows from `json` and keeps the defaults for the rest.
  factory DiffSettings.fromJson(Map<String, Object?> json) => DiffSettings(
    layout: DiffLayout.values.asNameMap()[json['layout']] ?? DiffLayout.unified,
    hideWhitespace: json['hideWhitespace'] == true,
  );

  final DiffLayout layout;

  /// Whether changes that are only whitespace are left out.
  final bool hideWhitespace;

  DiffSettings copyWith({DiffLayout? layout, bool? hideWhitespace}) =>
      DiffSettings(
        layout: layout ?? this.layout,
        hideWhitespace: hideWhitespace ?? this.hideWhitespace,
      );

  Map<String, Object?> toJson() => {
    'layout': layout.name,
    'hideWhitespace': hideWhitespace,
  };
}

/// The Diff tab's settings in use; each change is saved.
class DiffSettingsController extends ValueNotifier<DiffSettings> {
  DiffSettingsController([super.value = const DiffSettings(), this._file]);

  static Future<DiffSettingsController> load(SettingsFile file) async {
    final json = await file.load();
    return DiffSettingsController(
      json == null ? const DiffSettings() : DiffSettings.fromJson(json),
      file,
    );
  }

  final SettingsFile? _file;

  /// Uses `settings` at once; fails if they could not be saved.
  Future<void> change(DiffSettings settings) async {
    value = settings;
    await _file?.save(settings.toJson());
  }
}
