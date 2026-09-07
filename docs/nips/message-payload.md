# New-message payload validation

`draft` `relay` `client`

This document specifies Buzz's payload rule for new channel messages. It adds
no event kind, tag, endpoint, or negotiation flag.

## Motivation

An empty variable or a failed command producing no stdout can otherwise publish
an empty message successfully. CLI validation gives the sender an actionable
error, SDK validation protects other callers, and relay validation covers clients
that construct signed events directly.

## Contract

The following new-message kinds MUST contain non-whitespace text or at least one
valid media attachment:

| Kind | Message |
| --- | --- |
| 9 | Stream message, including replies |
| 40002 | Legacy stream message, including replies |
| 45001 | Forum post |
| 45003 | Forum comment |

For this rule, whitespace is the Unicode `White_Space` property, as implemented
by Rust's `str::trim`. Validation MUST NOT trim, normalize, or rewrite the event
content. Markdown indentation and trailing spaces are preserved. A zero-width
space (U+200B), for example, is not Unicode whitespace and is not rejected by this
rule; this is not a visual-emptiness or spam filter.

A relay MUST validate `imeta` metadata and its stored blob reference before
treating it as an attachment. An empty `imeta` tag, malformed metadata, or a
reference to a missing or inaccessible blob MUST NOT exempt a blank message.
Existing media validation and tenant scoping continue to apply to text-bearing
messages as well.

The shared ingest path MUST enforce this rule for HTTP `POST /events` and
WebSocket `EVENT` publication before storage. An invalid blank payload yields
HTTP 400 or a WebSocket `OK` response with `false` and an `invalid:` reason.
Clients SHOULD treat this as an input error and request text or an attachment
instead of retrying the unchanged event.

SDK new-message builders MUST reject blank content without a usable `imeta`
HTTP(S) URL. This is local structural validation: the SDK cannot establish blob
existence or tenant access; the relay remains authoritative for those checks.

Other event kinds are unaffected. In particular, an edit (kind 40003) may clear
existing text, and this rule does not constrain the empty content of auth,
deletion, reaction, or metadata events. Existing stored messages are not rewritten.

## CLI behavior

`buzz messages send` validates the resolved literal or stdin content before
network requests. Blank content without `--file` is a non-retryable `user_error`
with exit status 1. Specifying a file permits blank input at this early step;
upload failure still aborts publication. To send a captionless file:

```sh
buzz messages send --channel <UUID> --content "" --file picture.png
```

`--content -` reads through EOF and preserves the bytes read. It has no fixed
first-byte timeout: a slow producer is not an empty producer. A consumer cannot
infer whether a producer that wrote partial nonempty output eventually failed.
When only a complete successful report should be posted, stage it first:

```sh
report=$(mktemp) || exit 1
trap 'rm -f "$report"' EXIT
if generate_report >"$report"; then
  buzz messages send --channel <UUID> --content - <"$report"
else
  exit 1
fi
```

## Verification

`crates/buzz-cli/tests/message_payload.rs` runs real CLI subprocesses against a
recording HTTP fixture. `crates/buzz-sdk/tests/message_payload.rs` exercises all
three public builders. Both are included in `just test-unit`.

`crates/buzz-relay/tests/message_payload.rs` sends signed events through real
authenticated HTTP and WebSocket routes and checks database persistence. It
requires a disposable PostgreSQL database (`BUZZ_TEST_DATABASE_URL`), Redis and
S3-compatible media storage, and is explicitly run in the Relay E2E CI job:

```sh
cargo test -p buzz-relay --test message_payload -- --ignored --nocapture
```
