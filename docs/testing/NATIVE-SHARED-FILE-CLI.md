# Native shared-file CLI

The CLI uses the existing original GUI's authenticated file connection. Both the
CLI and running GUI must be the same executable. It does not open a window,
subscribe to media, grant permissions, publish local files or change preferences.
Silent command connections leave List/Fetch admission to the explicit command;
they do not enqueue a competing automatic list when opening or reusing a connection.
The GUI file panel retains its automatic listing behavior.

List one page of files the peer has explicitly shared:

```sh
weavelo --file-shared-list TARGET_DEVICE_ID --after-id 0 --request-id list-1
weavelo --file-status list-1
```

Wait for `file_request.state=completed`. `file_request.shared.entries` contains
at most eight opaque IDs, names and sizes. Use `shared.next_after_id` for another
page with a new request ID. An empty page means the peer currently shares no files
in this device group. Received files are not automatically shared.

Pull only a selected entry from that completed page, with its known content hash:

```sh
weavelo --file-shared-fetch TARGET_DEVICE_ID SHARED_FILE_ID \
  --from-list list-1 --sha256 EXPECTED_SHA256 \
  --request-id pull-1 --confirm-fetch
weavelo --file-status pull-1
```

The list receipt binds the peer, paired group, GUI generation, original connection
and selected entry. A changed connection requires a fresh list. No remote path is
accepted. The peer's existing process-local share catalog remains the authority;
revoked or unshared IDs are rejected by its existing protocol.

Fetch completion requires the original queued transfer ID, a matching incoming
completion event and an independent SHA-256/size check of that actual saved file
inside the configured receive folder. The receipt then has `completed`,
`remote_content_verified=true`, and `shared.received_path`. Existing files are
handled by the unchanged receiver's no-overwrite policy.

Reservations survive caller disconnects. Observe the same request ID after an
uncertain response; neither status queries nor `--file-resume` replay shared
requests. A queued response alone is not delivery or hash acceptance. For physical
acceptance, fetch only an explicitly identified test entry; do not pull user files
when no suitable test entry is listed. The peer must explicitly share a harmless
test file using its existing local file-sharing UI before such a pull is possible.
