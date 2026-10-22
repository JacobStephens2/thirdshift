# Conversation Session-trace fixtures

`console-{claude,codex,agy,grok,muse,opencode}-trace.jsonl` are synthetic OTLP
log requests at the Console's on-disk seam. They use the Session recorder's
`thirdshift.events` JSON-array attribute and retain a Harness-shaped raw body.
All six carry the same user/assistant messages, Read call and result, unknown
event, and final message. A record can contain more than one normalized event.

Muse and OpenCode include cumulative snapshots of the same message ID, plus a
message with that ID in another parent scope. Codex includes a reasoning item
normalized as an assistant message; its raw item kind retains the Thinking view.

The raw bodies deliberately contain different text so a renderer that ignores
the normalized attributes fails the assertions. The unknown event's raw body
contains angle brackets, and the tool result contains a script tag, to check
escaping. These are reader fixtures, not recordings of actual Harness sessions;
the decoder and recorder contracts have their own tests.
