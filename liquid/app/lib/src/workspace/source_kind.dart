import 'package:flutter/material.dart';

import '../theme/icons.dart';

String fileSize(int bytes) => switch (bytes) {
  < 1024 => '$bytes B',
  < 1024 * 1024 => '${(bytes / 1024).round()} KB',
  _ => '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB',
};

String kindOf(String name) {
  final dot = name.lastIndexOf('.');
  return switch (dot < 0 ? '' : name.substring(dot + 1).toLowerCase()) {
    'csv' => 'CSV',
    'tsv' => 'TSV',
    'md' || 'markdown' => 'Markdown',
    'txt' => 'Text',
    'pdf' => 'PDF',
    'png' || 'jpg' || 'jpeg' || 'gif' || 'webp' || 'heic' => 'Image',
    _ => 'File',
  };
}

IconData sourceIcon(String name, {required bool folder}) {
  if (folder) return LucideIcons.folder;
  return switch (kindOf(name)) {
    'CSV' || 'TSV' => LucideIcons.fileSpreadsheet,
    'Image' => LucideIcons.image,
    'Markdown' || 'Text' => LucideIcons.fileText,
    _ => LucideIcons.file,
  };
}
