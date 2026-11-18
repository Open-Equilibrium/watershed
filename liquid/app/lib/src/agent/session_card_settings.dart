import 'package:flutter/foundation.dart';

import '../settings_file.dart';

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

  final String label;

  final bool shownByDefault;
}

@immutable
class SessionCardSettings {
  const SessionCardSettings({this.hidden = const {}, this.shown = const {}});

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
