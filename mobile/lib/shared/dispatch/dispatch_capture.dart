import 'dart:convert';
import 'package:crypto/crypto.dart';
import 'package:uuid/uuid.dart';

// The SDK namespace is arbitrary bytes, not an RFC UUID variant. The uuid
// package's v5 method rejects it, so apply RFC 9562 to the exact SDK bytes.
String _captureId(String source) {
  final namespace = Uuid.parse(
    '73632d64-6973-7061-7463-682d76310000',
    validate: false,
  );
  final bytes = sha1
      .convert([...namespace, ...utf8.encode('pin:$source')])
      .bytes
      .toList();
  bytes[6] = (bytes[6] & 0x0f) | 0x50;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  return Uuid.unparse(bytes.sublist(0, 16));
}

/// Uses the SDK pin-capture namespace so desktop/mobile/reactions deduplicate.
Map<String, dynamic> dispatchCaptureRoot(
  String channel,
  String source,
  String title,
) {
  if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(source) ||
      !RegExp(
        r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
      ).hasMatch(channel) ||
      title.trim().isEmpty ||
      title.length > 256) {
    throw const FormatException(
      'Capture requires a delivered message, channel, and title.',
    );
  }
  final id = _captureId(source);
  final link = Uri(
    scheme: 'buzz',
    host: 'message',
    queryParameters: {'channel': channel, 'id': source},
  ).toString();
  return {
    'v': 1,
    'op': 'create',
    'issue_id': id,
    'identifier': 'D-${id.replaceAll('-', '')}',
    'title': title.trim(),
    'body': '',
    'status': 'triage',
    'priority': null,
    'assignee': null,
    'project_id': null,
    'due': null,
    'estimate_minutes': null,
    'home_channel_id': channel,
    'source_event_id': source,
    'workflow': 'default-v1',
    'labels': <String>[],
    'external_refs': [
      {'type': 'conversation', 'ref': link},
    ],
    'request_id': id,
    'import': null,
  };
}

List<List<String>> dispatchCaptureTags(
  String channel,
  Map<String, dynamic> root,
) => [
  ['h', channel],
  ['d', root['issue_id'] as String],
  ['t', 'buzz-tracker'],
  ['request', root['request_id'] as String],
  ['subject', root['title'] as String],
  ['status', 'triage'],
  ['identifier', root['identifier'] as String],
  ['home', channel],
  ['e', root['source_event_id'] as String, '', 'source'],
  [
    'external_ref',
    'conversation',
    (root['external_refs'] as List).first['ref'] as String,
  ],
];
