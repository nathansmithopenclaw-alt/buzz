import 'dart:convert';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/dispatch/dispatch_capture.dart';
import 'package:buzz/shared/dispatch/dispatch_capture_sheet.dart';
import 'package:buzz/shared/dispatch/dispatch_repository.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const channel = '11111111-1111-4111-8111-111111111111';
final source = 'a' * 64;

class Repository extends Fake implements DispatchRepository {
  final List<List<String>> captures = [];
  bool fail = false;
  @override
  Future<void> capture(String channel, String source, String title) async {
    if (fail) throw StateError('Capture not accepted');
    captures.add([channel, source, title]);
  }
}

class RetainedCaptureRepository extends DispatchRepository {
  RetainedCaptureRepository(nostr.Keys keys)
    : super(
        relay: 'https://capture.invalid',
        pubkey: keys.public,
        nsec: keys.nsec,
        session: RelaySessionNotifier(),
        isCurrent: () => true,
      );
  bool failVerification = true;
  final List<String> checkedEvents = [];
  @override
  Future<Map<String, dynamic>> snapshot(
    String workspace, {
    String? issueId,
    String? after,
    bool includeClosed = false,
  }) async {
    final saved = await pending(workspace);
    if (saved != null) {
      checkedEvents.add(saved.event.id);
      if (failVerification) throw StateError('Acceptance unavailable');
    }
    return {
      'issues': [
        if (saved != null)
          {
            'issueId': saved.issueId,
            'sourceEventId': source,
            'rootEvent': saved.event.id,
            'tipEvent': saved.event.id,
            'acceptedEvents': [saved.event.id],
          },
      ],
      'decisions': [],
    };
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'capture retains signed bytes before verification and reuses them on retry',
    () async {
      SharedPreferences.setMockInitialValues({});
      final keys = nostr.Keys.generate();
      final repository = RetainedCaptureRepository(keys);
      await expectLater(
        repository.capture(channel, source, 'Original intent'),
        throwsStateError,
      );
      final saved = await repository.pending(channel);
      expect(saved, isNotNull);
      final event = nostr.Event.fromJson(jsonEncode(saved!.event.toJson()));
      expect(event.id, saved.event.id);
      expect(jsonDecode(saved.event.content)['title'], 'Original intent');
      repository.failVerification = false;
      await repository.capture(channel, source, 'A different retry title');
      expect(repository.checkedEvents, [saved.event.id, saved.event.id]);
      expect(await repository.pending(channel), isNull);
    },
  );
  test(
    'capture uses SDK request identity, same-channel triage and source tags',
    () {
      final root = dispatchCaptureRoot(channel, source, 'Follow up');
      expect(root['request_id'], '4155f154-f259-5902-bb4d-9c9f14baf4f6');
      expect(root['issue_id'], root['request_id']);
      expect(root['home_channel_id'], channel);
      expect(root['status'], 'triage');
      expect(root['body'], '');
      const seededChannel = '9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50';
      expect(
        dispatchCaptureRoot(
          seededChannel,
          source,
          'Follow up',
        )['home_channel_id'],
        seededChannel,
      );
      expect(
        dispatchCaptureTags(channel, root),
        contains(equals(['e', source, '', 'source'])),
      );
      final event = {
        'kind': 40020,
        'pubkey': 'actor',
        'content': jsonEncode(root),
        'tags': dispatchCaptureTags(channel, root),
      };
      final request = {
        'operation': 'capture',
        'request_id': root['request_id'],
        'root': root,
      };
      expect(
        () => validatePreparedDispatchEvent(event, request, channel, 'actor'),
        returnsNormally,
      );
      final changed = {...root, 'status': 'todo'};
      expect(
        () => validatePreparedDispatchEvent(
          {...event, 'content': jsonEncode(changed)},
          request,
          channel,
          'actor',
        ),
        throwsFormatException,
      );
      expect(
        () => dispatchCaptureRoot(channel, 'pending-message', 'Follow up'),
        throwsFormatException,
      );
    },
  );
  testWidgets(
    'capture sheet reviews title and preserves failed intent before success',
    (tester) async {
      final repository = Repository()..fail = true;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [dispatchRepositoryProvider.overrideWithValue(repository)],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: DispatchCaptureSheet(
                channel: channel,
                source: source,
                content: 'First line\nPrivate source details',
              ),
            ),
          ),
        ),
      );
      expect(find.text('First line'), findsOneWidget);
      await tester.tap(find.text('Create linked issue'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Capture not accepted'), findsOneWidget);
      expect(find.text('First line'), findsOneWidget);
      expect(repository.captures, isEmpty);
      repository.fail = false;
      await tester.tap(find.text('Create linked issue'));
      await tester.pumpAndSettle();
      expect(repository.captures, [
        [channel, source, 'First line'],
      ]);
      expect(
        find.text('Saved in Triage. Open Work to view the issue.'),
        findsOneWidget,
      );
    },
  );
}
