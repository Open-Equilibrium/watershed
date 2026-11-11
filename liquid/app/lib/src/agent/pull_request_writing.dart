import 'dart:async';

import 'package:flutter/material.dart';

import '../rust/api/git.dart';
import '../session/pull_requests.dart';

/// Where the title and description came from.
enum TextSource { commit, written, failed }

/// The Chat's model writes the pull request's title and description into the fields the user has
/// not edited, once for each range of commits and, before a commit, selection of files.
class PullRequestWriting extends ChangeNotifier {
  PullRequestWriting({
    required this.pulls,
    required TextEditingController title,
    required TextEditingController body,
  }) : _titleField = title,
       _bodyField = body;

  final PullRequests pulls;
  final TextEditingController _titleField;
  final TextEditingController _bodyField;

  TextSource source = TextSource.commit;

  /// The model is still writing.
  bool writing = false;

  /// How many commits the model read.
  int written = 0;

  /// The model's text also describes uncommitted files.
  bool writtenFiles = false;

  /// What the model was last asked about.
  String? _asked;
  int _writes = 0;

  /// Runs while Create waits after the model changed the fields, so a click meant for the
  /// earlier text does not send unread text.
  Timer? _settling;

  /// What the dialog last put in the fields; a field that differs is the user's.
  String _filledTitle = '';
  String _filledBody = '';

  bool get settling => _settling != null;

  /// Puts the latest commit's title and description in the empty fields, before the model answers.
  void fillFromCommit(String title, String body) {
    _titleField.text = _filledTitle = title;
    _bodyField.text = _filledBody = body;
  }

  /// Asks the model through `write` unless it already wrote for `key`; `write` gives none when
  /// there is nothing to describe.
  void ask(
    BridgePullRequestPlan plan,
    String key,
    Future<BridgePullRequestText?> Function() write, {
    bool files = false,
  }) {
    if (key == _asked) return;
    _asked = key;
    final asked = ++_writes;
    writing = true;
    unawaited(() async {
      BridgePullRequestText? text;
      var failed = false;
      try {
        text = await write();
      } on Object {
        failed = true;
      }
      if (asked != _writes) return;
      writing = false;
      written = text?.commits.toInt() ?? 0;
      writtenFiles = files;
      source = failed
          ? TextSource.failed
          : text == null
          ? TextSource.commit
          : TextSource.written;
      // Without an answer, fields that still show earlier model text show the commit again.
      _fill(text?.title ?? plan.subject, text?.body ?? plan.body);
      notifyListeners();
    }());
  }

  /// Puts `title` and `body` in the fields the user has not edited.
  void _fill(String title, String body) {
    var changed = false;
    if (_titleField.text == _filledTitle && title != _filledTitle) {
      _titleField.text = _filledTitle = title;
      changed = true;
    }
    if (_bodyField.text == _filledBody && body != _filledBody) {
      _bodyField.text = _filledBody = body;
      changed = true;
    }
    if (!changed) return;
    _settling?.cancel();
    _settling = Timer(const Duration(milliseconds: 500), () {
      _settling = null;
      notifyListeners();
    });
  }

  /// Stops waiting for the model; what it would have written is dropped, and the same commits
  /// may be written for again.
  void stop() {
    if (!writing) return;
    _writes++;
    writing = false;
    _asked = null;
    unawaited(pulls.stopWriting().catchError((Object _) {}));
  }

  @override
  void dispose() {
    stop();
    _settling?.cancel();
    super.dispose();
  }
}
