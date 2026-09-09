# Dictionary attachment and readiness

The dictionary loader constructs a `DictStore` on a worker thread. Conversion and
learning use the store attached to the current engine, not the worker's pending
result. Previously these two stages could remain disconnected after reloading:

* TSF cached `ready=true` indefinitely until its own reload callback. A host restart
  or engine replacement initiated elsewhere could leave that cache stale, so the
  client stopped asking the new engine to attach its dictionary.
* `engine_start_load_dict` cleared a completed pending dictionary whenever another
  start arrived before attachment, unnecessarily loading it again.
* The loader reported `ok` before attachment, and candidate lookups overwrote the
  dictionary status (including failure details) with search results.

The installed pre-fix DLL reproduced the second case: after `ok: mozc=true`, a
second start still left `engine_is_dict_ready` false. A regression test seeds a
completed store containing `けいてい → 径庭`, repeats start, then verifies readiness,
candidate lookup, idempotent polling and preservation of status/error details.

## Changed behavior

The loading flag and pending dictionary now share one mutex. A repeated start
attaches the completed store; it neither discards it nor starts a duplicate load.
Loading itself remains outside the mutex.

The host polls pending dictionary and model results before ordinary engine
requests, including candidate lookup and learning. Thus attachment does not rely
on a client explicitly sending a readiness poll. These host-side checks are local
DLL calls with nonblocking pending checks, not additional RPC round trips.

TSF also rechecks the current engine instead of using a previous successful poll
to skip all future checks. Its cached booleans now serve only as the last observed
state for the icon and transition logging. This adds readiness RPC calls on paths
that previously skipped them after initialization; it does not reload dictionaries
or run inference. Startup remains asynchronous and can legitimately report not
ready while files are loading.

Dictionary status distinguishes `loading`, `loaded; awaiting engine attachment`,
`ready: attached to engine`, and `failed at [...]`. Candidate lookup no longer
overwrites it. The raw store log says `loaded`, reserving engine readiness for the
attachment stage.

Model construction similarly reports attachment pending, and readiness includes
the model being used by background conversion. Readiness polling does not reclaim
or discard completed background candidates.

## Validation

```powershell
cargo test -p rakukan-engine -p rakukan-engine-abi -p rakukan-engine-rpc -p rakukan-tsf --release --lib --locked --offline
cargo build -p rakukan-engine -p rakukan-engine-host -p rakukan-tsf --release --locked --offline
$env:RAKUKAN_TEST_ENGINE_DLL = 'C:\path\to\rakukan_engine.dll'
# Optional: also load the installed model and check readiness during conversion.
$env:RAKUKAN_TEST_READY_MODEL = '1'
cargo test -p rakukan-engine-rpc --release --lib --locked --offline dictionary_is_attached_without_client_poll_after_engine_recreation -- --ignored --nocapture
cargo clippy -p rakukan-engine -p rakukan-engine-rpc -p rakukan-tsf --release --lib --locked --offline --no-deps -- -D warnings
```

The explicit integration test uses the installed dictionary without modifying
learning history. It recreates the engine three times and obtains `径庭` via the
normal host dispatch path without a client readiness poll. The optional model
check ensures ready stays true during background conversion and its candidates
remain available. The test pins the DLL until process exit when running its
process-lifetime background worker, matching the host's process restart policy.

This branch starts from the KV-cache main commit and is independent of the
candidate-window DPI branch. Combine the branches before deploying a TSF DLL
that needs both changes. Updating the engine DLL and host enables host-side
attachment even for older TSF clients; the TSF readiness-display change requires
applications to load the updated TSF DLL.
