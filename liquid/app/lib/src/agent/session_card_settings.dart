import 'package:flutter/foundation.dart';

import '../settings_file.dart';

/// The rows of the session card that can be hidden, in the card's order, which is also the rows
/// menu's. Ids are stored, so never rename one. Rows that T3 Code leaves off by default are
/// `shownByDefault: false`.
enum CardItem {
  workspace('workspace', 'Environment and worktree'),
  branch('branch', 'Branch and pull request'),
  gitActions('gitActions', 'Git actions'),
  changes('changes', 'Changes'),
  servers('servers', 'Servers'),
  automations('automations', 'Automations'),
  lineage('lineage', 'Lineage'),
  sideChats('sideChats', 'Side chats'),
  statistics('figures', 'Statistics'),
  usage('usage', 'Usage limits', shownByDefault: false),
  context('context', 'Context window', shownByDefault: false);

  const CardItem(this.id, this.label, {this.shownByDefault = true});

  final String id;

  /// The row's name in the card's menu.
  final String label;

  final bool shownByDefault;
}

/// Which rows the session card shows. Only the rows that differ from their default are kept:
/// `hidden` names the rows shown by default that the person hid, and `shown` the rows hidden by
/// default that the person showed.
@immutable
class SessionCardSettings {
  const SessionCardSettings({this.hidden = const {}, this.shown = const {}});

  /// Reads the choices it knows from `json` and ignores the rest. An entry in the wrong list is
  /// ignored, so a row keeps its default unless its own list names it.
  factory SessionCardSettings.fromJson(Map<String, Object?> json) {
    final items = {for (final item in CardItem.values) item.id: item};
    Set<CardItem> listed(String key, {required bool shownByDefault}) => {
      if (json[key] case final List<Object?> ids)
        for (final id in ids)
          if (items[id] case final item?
              when item.shownByDefault == shownByDefault)
            item,
    };
    return SessionCardSettings(
      hidden: listed('hidden', shownByDefault: true),
      shown: listed('shown', shownByDefault: false),
    );
  }

  final Set<CardItem> hidden;
  final Set<CardItem> shown;

  bool get isDefault => hidden.isEmpty && shown.isEmpty;

  bool shows(CardItem item) =>
      item.shownByDefault ? !hidden.contains(item) : shown.contains(item);

  /// These settings with `item` shown or hidden; a choice back to the default is not kept.
  SessionCardSettings withItem(CardItem item, bool visible) =>
      item.shownByDefault
      ? SessionCardSettings(
          hidden: visible ? hidden.difference({item}) : {...hidden, item},
          shown: shown,
        )
      : SessionCardSettings(
          hidden: hidden,
          shown: visible ? {...shown, item} : shown.difference({item}),
        );

  Map<String, Object?> toJson() => {
    'hidden': [
      for (final item in CardItem.values)
        if (hidden.contains(item)) item.id,
    ],
    'shown': [
      for (final item in CardItem.values)
        if (shown.contains(item)) item.id,
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
