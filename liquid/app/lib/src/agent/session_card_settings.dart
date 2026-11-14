import 'package:flutter/foundation.dart';

import '../settings_file.dart';

/// The rows of the session card that can be hidden, in the card's order. Ids are stored, so
/// never rename one.
enum CardItem {
  branch('branch', 'Branch and pull request'),
  gitActions('gitActions', 'Git actions'),
  changes('changes', 'Changes'),
  figures('figures', 'Session figures'),
  context('context', 'Context window');

  const CardItem(this.id, this.label);

  final String id;

  /// The row's name in the card's menu.
  final String label;
}

/// Which rows the session card shows; every row shows by default, so only hidden rows are kept.
@immutable
class SessionCardSettings {
  const SessionCardSettings({this.hidden = const {}});

  /// Reads the hidden rows it knows from `json` and ignores the rest.
  factory SessionCardSettings.fromJson(Map<String, Object?> json) {
    final items = {for (final item in CardItem.values) item.id: item};
    return SessionCardSettings(
      hidden: {
        if (json['hidden'] case final List<Object?> ids)
          for (final id in ids) ?items[id],
      },
    );
  }

  final Set<CardItem> hidden;

  bool get isDefault => hidden.isEmpty;

  bool shows(CardItem item) => !hidden.contains(item);

  /// These settings with `item` shown or hidden; a choice back to the default is not kept.
  SessionCardSettings withItem(CardItem item, bool shown) =>
      SessionCardSettings(
        hidden: shown ? hidden.difference({item}) : {...hidden, item},
      );

  Map<String, Object?> toJson() => {
    'hidden': [
      for (final item in CardItem.values)
        if (hidden.contains(item)) item.id,
    ],
  };
}

/// The session card's settings in use; each change is saved.
class SessionCardSettingsController extends SavedSettings<SessionCardSettings> {
  SessionCardSettingsController([
    SessionCardSettings value = const SessionCardSettings(),
    SettingsFile? file,
  ]) : super(
         value,
         file == null ? null : (settings) => file.save(settings.toJson()),
         'the session card settings',
       );

  static Future<SessionCardSettingsController> load(SettingsFile file) async {
    final json = await file.load();
    return SessionCardSettingsController(
      json == null
          ? const SessionCardSettings()
          : SessionCardSettings.fromJson(json),
      file,
    );
  }
}
