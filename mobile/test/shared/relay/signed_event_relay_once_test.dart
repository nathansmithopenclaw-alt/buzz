import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  Future<(HttpServer, List<Map<String, dynamic>>)> server({
    bool reject = false,
    void Function()? beforeAuth,
    bool dropAck = false,
  }) async {
    final relay = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final writes = <Map<String, dynamic>>[];
    relay.transform(WebSocketTransformer()).listen((socket) {
      socket.add(jsonEncode(['AUTH', 'disposable-test-challenge']));
      socket.listen((raw) {
        final frame = jsonDecode(raw as String) as List;
        final event = frame[1] as Map<String, dynamic>;
        nostr.Event.fromJson(jsonEncode(event));
        if (frame[0] == 'AUTH') {
          beforeAuth?.call();
          socket.add(jsonEncode(['OK', event['id'], true, '']));
        } else if (frame[0] == 'EVENT') {
          writes.add(event);
          if (!dropAck) {
            socket.add(
              jsonEncode([
                'OK',
                event['id'],
                !reject,
                reject ? 'restricted: fixture rejection' : 'saved',
              ]),
            );
            if (!reject) {
              socket.add(
                jsonEncode([
                  'OK',
                  event['id'],
                  true,
                  'duplicate acknowledgement',
                ]),
              );
            }
          }
        }
      }, onError: (_) {});
    });
    return (relay, writes);
  }

  final key = nostr.Keys.generate();
  final signed = NostrEvent.fromJson(
    nostr.Event.from(
      kind: 40021,
      content: 'retained signed edit',
      tags: [
        ['h', 'fixture-workspace'],
      ],
      secretKey: nostr.Nip19.decode(payload: key.nsec).data,
      createdAt: 1700000000,
    ).toMap(),
  );

  test(
    'retries preserve exact retained bytes and tolerate duplicate acknowledgements',
    () async {
      final (relay, writes) = await server();
      try {
        for (var attempt = 0; attempt < 2; attempt++) {
          final receipt = await publishSignedEventOnce(
            wsUrl: 'ws://127.0.0.1:${relay.port}',
            nsec: key.nsec,
            event: signed,
          );
          expect(receipt.id, signed.id);
        }
        expect(writes, [signed.toJson(), signed.toJson()]);
      } finally {
        await relay.close(force: true);
      }
    },
  );

  test(
    'community change while authenticating prevents any event publication',
    () async {
      var current = true;
      final (relay, writes) = await server(beforeAuth: () => current = false);
      try {
        await expectLater(
          publishSignedEventOnce(
            wsUrl: 'ws://127.0.0.1:${relay.port}',
            nsec: key.nsec,
            event: signed,
            beforePublish: () => current,
          ),
          throwsStateError,
        );
        expect(writes, isEmpty);
      } finally {
        await relay.close(force: true);
      }
    },
  );

  test('rejection and uncertain acknowledgement remain failures', () async {
    for (final dropAck in [false, true]) {
      final (relay, writes) = await server(reject: !dropAck, dropAck: dropAck);
      try {
        await expectLater(
          publishSignedEventOnce(
            wsUrl: 'ws://127.0.0.1:${relay.port}',
            nsec: key.nsec,
            event: signed,
            timeout: const Duration(seconds: 1),
          ),
          dropAck ? throwsA(isA<TimeoutException>()) : throwsException,
        );
        expect(writes, [signed.toJson()]);
      } finally {
        await relay.close(force: true);
      }
    }
  });
}
