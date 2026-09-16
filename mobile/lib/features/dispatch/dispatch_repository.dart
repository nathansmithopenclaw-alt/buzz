import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import '../../shared/relay/nostr_filters.dart';
import '../../shared/relay/nostr_models.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/relay/relay_session.dart';
import '../../shared/relay/signed_event_relay.dart';

class DispatchWorkspace {
  const DispatchWorkspace(this.id, this.name);
  final String id;
  final String name;
}

/// One retained signed edit per workspace. A failed acknowledgement is never
/// permission to create another event for the same intent.
class PendingDispatchEdit {
  PendingDispatchEdit(this.event, this.request, this.issueId);
  final NostrEvent event;
  final Map<String, dynamic> request;
  final String issueId;
  Map<String, dynamic> toJson() => {
    'event': event.toJson(),
    'request': request,
    'issueId': issueId,
  };
}

class DispatchRepository {
  DispatchRepository({
    required this.relay,
    required this.pubkey,
    required String nsec,
    required this.session,
    required this.isCurrent,
  }) : _nsec = nsec;
  final String relay;
  final String pubkey;
  final String _nsec;
  final RelaySessionNotifier session;
  final bool Function() isCurrent;
  bool _writing = false;

  String _key(String workspace) =>
      'dispatch-mobile-v1:${Uri.encodeComponent(relay)}:$pubkey:$workspace';
  void _fence() {
    if (!isCurrent()) {
      throw StateError(
        'The community or identity changed. Return to the original workspace to recover this edit.',
      );
    }
  }

  Future<List<DispatchWorkspace>> workspaces() async {
    _fence();
    final memberships = await session.queryRelay([
      NostrFilters.myChannels(pubkey),
    ]);
    _fence();
    if (memberships.length >= 500) {
      throw StateError(
        'Workspace discovery reached its read limit. Use Desktop to narrow the workspace.',
      );
    }
    final members = <String, NostrEvent>{};
    for (final event in memberships) {
      nostr.Event.fromJson(jsonEncode(event.toJson()));
      final id = event.getTagValue('d');
      if (id != null &&
          event.kind == 39002 &&
          event.tags.any(
            (tag) => tag.length > 1 && tag[0] == 'p' && tag[1] == pubkey,
          )) {
        members[id] = event;
      }
    }
    final ids = members.keys.toList();
    final populated = <String>{};
    for (var index = 0; index < ids.length; index += 20) {
      final batch = ids.skip(index).take(20);
      final found = await session.queryRelay([
        for (final id in batch)
          NostrFilter(
            kinds: [40020, 40026],
            tags: {
              '#h': [id],
            },
            limit: 1,
          ),
      ]);
      _fence();
      populated.addAll(
        found.map((event) => event.channelId).whereType<String>(),
      );
    }
    if (populated.isEmpty) return [];
    final metadata = await session.queryRelay([
      NostrFilters.channelMetadata(populated.toList()),
    ]);
    _fence();
    final names = <String, String>{};
    for (final event in metadata) {
      final id = event.getTagValue('d');
      if (id == null || event.pubkey != members[id]?.pubkey) continue;
      nostr.Event.fromJson(jsonEncode(event.toJson()));
      String? name = event.getTagValue('name');
      try {
        name ??=
            (jsonDecode(event.content) as Map<String, dynamic>)['name']
                as String?;
      } catch (_) {
        /* Tagged metadata is also valid. */
      }
      if (name != null && name.trim().isNotEmpty) names[id] = name;
    }
    return [
      for (final id in populated)
        DispatchWorkspace(id, names[id] ?? 'Workspace ${id.substring(0, 8)}'),
    ]..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
  }

  Future<Map<String, dynamic>> command(
    String workspace,
    Map<String, dynamic> request,
  ) async {
    _fence();
    final result = await SignedEventRelay(session: session, nsec: _nsec).submit(
      kind: EventKind.dispatchClient,
      content: jsonEncode({'v': 1, 'request': request}),
      tags: [
        ['h', workspace],
      ],
    );
    _fence();
    if (!result.content.startsWith('response:')) {
      throw StateError(
        'This relay has no compatible Dispatch mobile response. Update its Dispatch support; an empty workspace has not been confirmed.',
      );
    }
    final value = jsonDecode(result.content.substring(9));
    if (value is! Map<String, dynamic> || value['v'] != 1) {
      throw const FormatException('Invalid Dispatch response');
    }
    return value;
  }

  Future<Map<String, dynamic>> snapshot(
    String workspace, {
    String? issueId,
    String? after,
    bool includeClosed = false,
  }) async {
    final value = await command(workspace, {
      'operation': 'snapshot',
      'issue_id': issueId,
      'after': after,
      'include_closed': includeClosed,
    });
    if (value['workspace'] != workspace ||
        value['issues'] is! List ||
        value['decisions'] is! List ||
        !['member', 'maintainer'].contains(value['viewerRole'])) {
      throw const FormatException('Dispatch snapshot scope is invalid');
    }
    return value;
  }

  Future<Map<String, dynamic>> allOpen(String workspace) async {
    final first = await snapshot(workspace);
    final issues = [...(first['issues'] as List)];
    var after = first['nextAfter'] as String?;
    var pages = 1;
    final seen = <String>{};
    while (after != null) {
      if (pages++ >= 20 || !seen.add(after)) {
        throw StateError(
          'Mobile work history exceeded its bounded page limit. Open a specific issue on Desktop.',
        );
      }
      final next = await snapshot(workspace, after: after);
      if (first['snapshotToken'] != next['snapshotToken']) {
        throw StateError(
          'Work changed while loading. Refresh to see a consistent list.',
        );
      }
      issues.addAll(next['issues'] as List);
      after = next['nextAfter'] as String?;
    }
    return {...first, 'issues': issues, 'nextAfter': null};
  }

  Future<PendingDispatchEdit?> pending(String workspace) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.reload();
    _fence();
    final saved = prefs.getString(_key(workspace));
    if (saved == null) return null;
    final value = jsonDecode(saved) as Map<String, dynamic>;
    final event = NostrEvent.fromJson(value['event'] as Map<String, dynamic>);
    nostr.Event.fromJson(jsonEncode(event.toJson()));
    if (event.pubkey != pubkey ||
        event.channelId != workspace ||
        ![40021, 40026].contains(event.kind)) {
      throw const FormatException(
        'Retained edit scope does not match this workspace',
      );
    }
    final request = value['request'] as Map<String, dynamic>;
    validatePreparedDispatchEvent(event.toJson(), request, workspace, pubkey);
    return PendingDispatchEdit(event, request, value['issueId'] as String);
  }

  Future<void> edit(
    String workspace,
    String issueId,
    Map<String, dynamic> intent, {
    Map<String, dynamic>? decisionPayload,
  }) async {
    if (_writing) throw StateError('Another Dispatch edit is being checked.');
    _writing = true;
    try {
      if (await pending(workspace) != null) {
        throw StateError('Recover the saved edit before starting another.');
      }
      final request = {
        ...intent,
        'request_id': const Uuid().v4(),
        '_baseline_payload': ?decisionPayload,
      };
      final response = await command(
        workspace,
        {...request}..remove('_baseline_payload'),
      );
      final unsigned = response['unsigned'] as Map<String, dynamic>;
      validatePreparedDispatchEvent(unsigned, request, workspace, pubkey);
      _fence();
      final signed = nostr.Event.from(
        kind: unsigned['kind'] as int,
        content: unsigned['content'] as String,
        tags: (unsigned['tags'] as List)
            .map((tag) => (tag as List).cast<String>())
            .toList(),
        createdAt: unsigned['created_at'] as int,
        secretKey: nostr.Nip19.decode(payload: _nsec).data,
        verify: false,
      );
      final retained = PendingDispatchEdit(
        NostrEvent.fromJson(signed.toMap()),
        request,
        issueId,
      );
      final prefs = await SharedPreferences.getInstance();
      if (!await prefs.setString(
        _key(workspace),
        jsonEncode(retained.toJson()),
      )) {
        throw StateError(
          'The edit could not be retained safely. It has not been sent.',
        );
      }
      _fence();
      await _resume(workspace, retained);
    } finally {
      _writing = false;
    }
  }

  Future<String> savedDrafts(String workspace) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.reload();
    _fence();
    return prefs.getString('${_key(workspace)}:drafts') ?? '[]';
  }

  Future<void> keepPendingAsDraft(String workspace) async {
    if (_writing) throw StateError('Wait for the current check to finish.');
    _writing = true;
    try {
      final saved = await pending(workspace);
      if (saved == null) return;
      final prefs = await SharedPreferences.getInstance();
      final drafts = (jsonDecode(await savedDrafts(workspace)) as List);
      // An interrupted archive can leave both copies. Never duplicate a draft.
      if (!drafts.any((item) => item['event']['id'] == saved.event.id)) {
        if (drafts.length >= 20) {
          throw StateError(
            'Saved drafts are full. Copy and remove an older draft first.',
          );
        }
        drafts.add(saved.toJson());
      }
      _fence();
      if (!await prefs.setString(
        '${_key(workspace)}:drafts',
        jsonEncode(drafts),
      )) {
        throw StateError('The draft could not be preserved.');
      }
      _fence();
      if (!await prefs.remove(_key(workspace))) {
        throw StateError(
          'The draft is preserved, but its pending copy still needs recovery.',
        );
      }
    } finally {
      _writing = false;
    }
  }

  Future<void> removeSavedDraft(String workspace, String eventId) async {
    if (_writing) throw StateError('Wait for the current check to finish.');
    _writing = true;
    try {
      final drafts = (jsonDecode(await savedDrafts(workspace)) as List);
      drafts.removeWhere((item) => item['event']['id'] == eventId);
      final prefs = await SharedPreferences.getInstance();
      _fence();
      if (!await prefs.setString(
        '${_key(workspace)}:drafts',
        jsonEncode(drafts),
      )) {
        throw StateError('The saved draft could not be removed.');
      }
    } finally {
      _writing = false;
    }
  }

  Future<void> resume(String workspace) async {
    if (_writing) throw StateError('Another Dispatch edit is being checked.');
    _writing = true;
    try {
      final saved = await pending(workspace);
      if (saved != null) await _resume(workspace, saved);
    } finally {
      _writing = false;
    }
  }

  Future<void> _resume(String workspace, PendingDispatchEdit saved) async {
    var current = await snapshot(workspace, issueId: saved.issueId);
    bool accepted(Map<String, dynamic> snapshot) {
      if (saved.event.kind == 40021) {
        return (snapshot['issues'] as List).cast<Map<String, dynamic>>().any(
          (issue) =>
              issue['rootEvent'] == saved.event.id ||
              issue['tipEvent'] == saved.event.id ||
              (issue['acceptedEvents'] as List).contains(saved.event.id),
        );
      }
      return (snapshot['decisions'] as List).cast<Map<String, dynamic>>().any(
        (decision) =>
            (decision['accepted_events'] as List).contains(saved.event.id),
      );
    }

    if (!accepted(current)) {
      final request = saved.request;
      if (saved.event.kind == 40021) {
        final issues = (current['issues'] as List).cast<Map<String, dynamic>>();
        if (issues.isEmpty ||
            issues.first['tipEvent'] != request['expected_tip']) {
          throw StateError(
            'This issue changed before the saved edit was accepted. Keep this draft and reconcile it in Desktop.',
          );
        }
      } else {
        final decisions = (current['decisions'] as List)
            .cast<Map<String, dynamic>>()
            .where(
              (item) =>
                  (item['record'] as Map)['record_id'] == request['record_id'],
            );
        if (decisions.isEmpty ||
            decisions.first['event_id'] != request['expected_event']) {
          throw StateError(
            'This decision changed before the saved choice was accepted. Keep this draft and review the current decision.',
          );
        }
      }
      _fence();
      final origin = Uri.parse(relay);
      await publishSignedEventOnce(
        wsUrl: origin
            .replace(scheme: origin.scheme == 'https' ? 'wss' : 'ws')
            .toString(),
        nsec: _nsec,
        event: saved.event,
        beforePublish: isCurrent,
      );
      _fence();
      current = await snapshot(workspace, issueId: saved.issueId);
      if (!accepted(current)) {
        throw StateError(
          'The relay has not confirmed this edit in accepted history. The exact signed edit is saved for recovery.',
        );
      }
    }
    _fence();
    final prefs = await SharedPreferences.getInstance();
    if (!await prefs.remove(_key(workspace))) {
      throw StateError(
        'The edit is accepted, but its local recovery copy could not be cleared. Check recovery again.',
      );
    }
  }
}

String canonicalDispatchJson(Object? value) {
  Object? ordered(Object? item) {
    if (item is Map) {
      final keys = item.keys.cast<String>().toList()..sort();
      return {for (final key in keys) key: ordered(item[key])};
    }
    if (item is List) return item.map(ordered).toList();
    return item;
  }

  return jsonEncode(ordered(value));
}

/// Validate the requested intent again before signing server-prepared bytes.
void validatePreparedDispatchEvent(
  Map<String, dynamic> event,
  Map<String, dynamic> request,
  String workspace,
  String actor,
) {
  final tags = (event['tags'] as List)
      .map((tag) => (tag as List).cast<String>())
      .toList();
  final h = tags.where((tag) => tag.isNotEmpty && tag[0] == 'h').toList();
  final payload =
      jsonDecode(event['content'] as String) as Map<String, dynamic>;
  if (event['pubkey'] != actor ||
      h.length != 1 ||
      h.single.length != 2 ||
      h.single[1] != workspace ||
      payload['request_id'] != request['request_id']) {
    throw const FormatException(
      'Prepared edit does not match this identity or intent',
    );
  }
  if (request['operation'] == 'prepare_issue') {
    final set = payload['set'] as Map<String, dynamic>;
    final wanted = request['set'] as Map<String, dynamic>;
    final clear = (payload['clear'] as List).cast<String>();
    if (event['kind'] != 40021 ||
        payload['issue_id'] != request['issue_id'] ||
        payload['parent_event'] != request['expected_tip'] ||
        payload['reason'] != request['reason'] ||
        set.entries.any(
          (entry) =>
              !const {
                'identifier',
                'title',
                'body',
                'status',
                'priority',
                'assignee',
                'project_id',
                'due',
                'estimate_minutes',
                'home_channel_id',
                'workflow',
                'labels',
              }.contains(entry.key) ||
              (entry.value != null &&
                  (!['status', 'priority', 'assignee'].contains(entry.key) ||
                      wanted[entry.key] != entry.value)),
        ) ||
        wanted.entries.any(
          (entry) => entry.value != null && set[entry.key] != entry.value,
        ) ||
        clear.any(
          (field) =>
              !((field == 'assignee' && request['clear_assignee'] == true) ||
                  (field == 'priority' && request['clear_priority'] == true)),
        ) ||
        (request['clear_assignee'] == true && !clear.contains('assignee')) ||
        (request['clear_priority'] == true && !clear.contains('priority')) ||
        ((payload['relations'] as Map?)?.values.any(
              (values) => (values as List).isNotEmpty,
            ) ??
            false) ||
        ((payload['external_refs'] as Map?)?.values.any(
              (values) => (values as List).isNotEmpty,
            ) ??
            false)) {
      throw const FormatException(
        'Prepared issue edit changed the requested scope',
      );
    }
  } else if (request['operation'] == 'prepare_decision') {
    final decision = payload['payload'] as Map<String, dynamic>;
    final baseline = request['_baseline_payload'] as Map<String, dynamic>?;
    if (baseline == null ||
        canonicalDispatchJson({
              ...baseline,
              'selected_option': request['option'],
              'rationale': request['rationale'],
            }) !=
            canonicalDispatchJson(decision)) {
      throw const FormatException(
        'Prepared decision changed its reviewed options or evidence',
      );
    }
    if (event['kind'] != 40026 ||
        payload['record_id'] != request['record_id'] ||
        payload['parent_event'] != request['expected_event'] ||
        decision['type'] != 'decision' ||
        decision['decider'] != actor ||
        decision['selected_option'] != request['option'] ||
        decision['rationale'] != request['rationale']) {
      throw const FormatException(
        'Prepared decision changed the requested choice',
      );
    }
  } else {
    throw const FormatException('Unsupported prepared operation');
  }
}

final dispatchRepositoryProvider = Provider.autoDispose<DispatchRepository>((
  ref,
) {
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  if (config.nsec == null || pubkey == null) {
    throw StateError('Sign in to use Dispatch.');
  }
  var alive = true;
  ref.onDispose(() => alive = false);
  return DispatchRepository(
    relay: config.baseUrl,
    pubkey: pubkey,
    nsec: config.nsec!,
    session: ref.read(relaySessionProvider.notifier),
    isCurrent: () =>
        alive &&
        ref.read(relayConfigProvider).baseUrl == config.baseUrl &&
        ref.read(myPubkeyProvider) == pubkey,
  );
});
