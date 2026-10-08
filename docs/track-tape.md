# Track tape: mobile continuous performance capture

Tide needs synchronized continuous audio takes, with an editable mixer on replay. This additive C ABI captures one to four existing mixer tracks from their pre-strip accumulators; it does not depend on the desktop Studio prototype or its session format.

Attach `gooey_engine_track_tape_new` once while rendering is stopped. Track indices must be distinct and valid. One serialized UI producer sends commands. One file worker drains capture and feeds playback. Each PCM frame is finite float stereo pairs in configured track order (six channels for the legacy constructor, up to eight for the track-list constructor). The endpoint and engine retain independent shared ownership; workers must join before freeing their endpoint. No recorder path allocates, waits, locks or performs I/O during render.

`gooey_track_tape_command` returns a generation, or zero on invalid input/full queue. Commands are 0 return to live, 1 arm at the specified absolute transport beat, 2 stop, 3 play the supplied frame count from the beginning, 4 arm at the next four-beat boundary computed by render. Arm starts only while transport is running. Use command 4 on a running transport, or command 1 at beat four after reset for count-in. A late absolute arm moves to the following bar. Read applied generation first, then state: the acquire acknowledgement makes that command’s earlier state publication visible. Reading state first can pair an old stopped/ended state with a new arm/play acknowledgement.

States are 0 live, 1 armed, 2 capture, 3 stopped, 4 playback, 5 ended, 6 capture overflow and 7 playback underrun. Frames count only completely captured/replayed synchronized frames. Capture overflow retains the contiguous prefix. Playback underrun feeds silence and reports failure. Playback/end replace all live graph inputs with the configured recorded tracks or silence; existing racks and master process once and may decay after EOF. The monitor click remains outside capture.

The worker writes a single interleaved stream to preserve lane alignment. It must prepare a new file before arming, drain until an acknowledged stop is observed, and retain the old take until new frames arrive. Cancel the previous file feeder before returning to Live and awaiting acknowledgement, then feed playback before issuing Play and refill before its bounded buffer empties. At 48 kHz, each 131072-frame ring provides about 2.73 seconds of capacity. Retiring or clearing data belongs to the worker/control side.

`gooey_engine_track_tape_flush_stopped` applies pending tape and live controls after the host synchronously stops its audio callback. Never call it concurrently with render. It permits finalization after an OS interruption stops issuing callbacks.

## Queued instrument/mixer edits

`gooey_live_control_edit` extends the existing single-producer endpoint with primitive edits, returning the same generation acknowledgements. Parameters are `(op, a, b, c, x, y)`; unspecified fields are zero. Existing setter bounds apply.

| Op | Meaning | Arguments |
|---|---|---|
| 0 | Track pan | a track, x pan |
| 1/2 | Track mute/solo | a track, x boolean |
| 3 | Master gain | x linear gain |
| 4 | Global effect enable | a effect, x boolean |
| 5 | Global effect parameter | a effect, b parameter, x value |
| 6 | Drum step | a channel, b step, x enabled, y velocity |
| 7/8 | Set/clear step blend | a channel, b step, x/y blend |
| 9 | Enable/set voice blend | a channel, x/y blend |
| 10/11/12 | Voice gain/mute/solo | a channel, x value |
| 13 | Voice tuning | a channel, x normalized tuning |
| 14/15 | Set/clear parameter lock; clear all | a channel, b parameter, x value or -1 to clear |
| 17/18/19/20 | LFO enable/timing/amount/offset | a LFO; b timing or x value |
| 21 | Replace one LFO route | a LFO, b channel, c parameter, x depth |
| 22/23 | BPM/swing | x value |
| 24/25 | Reset/start; stop transport | none |
| 26 | Sequencer trigger enable | x boolean |
| 27 | Bass note step | a instrument, b step, c MIDI note or 128 for rest |
| 29 | Monitor metronome | x boolean |
| 30/31 | Piano parameter/velocity mode | a piano, b parameter, x value |
| 32 | Chord-loop piano strength | x value |
| 33 | Bass preset | a preset |

`gooey_live_control_replace_voice` prepares instrument DSP/blender on control, swaps it at a render boundary, and sends old DSP back to the control retirement ring. Reserved swap credits guarantee bounded retirement even with concurrent production. Channel sequencer/mixer/tuning remain intact; parameter locks reset. `gooey_live_control_get_channel_param` reads published render snapshots, avoiding concurrent reads of mutable instrument state. `gooey_live_control_set_master_order` queues all nine unique master effect IDs; the limiter remains final.

The queued track rack API now accepts every existing track effect, including delay tone/ping-pong, compressor, distortion, tilt and plate controls. Validation rejects invalid/discrete values and unsupported limiter racks before preparation. No existing C signature changes.

## Validation

From libgooey run `cargo test --no-default-features --features ios --lib` and `cargo test --no-default-features --features ios --release --test track_tape`. Integration tests consume the same C ABI as Swift and check stereo/dry capture, click exclusion, exact boundary capture, single mixer application, one-shot EOF, lifecycle and prepared percussion swaps. Build iOS and both simulator static libraries with `--no-default-features --features ios`.

The eleven C integration tests additionally cover ten-minute bounded capture, exact overflow prefixes, stopped-callback interruption finalization and zero render allocations/frees while swapping voices, editing LFO routes, capturing and replaying. Queued transport/tempo edits apply directly on render and never enter the legacy host mixer queue.

## Configurable lanes

Use `gooey_engine_track_tape_new_with_tracks(engine, tracks, count)` before rendering with one to four distinct valid bus IDs. Query `gooey_track_tape_get_channel_count` for drain/feed strides; it returns twice the configured track count, or zero for null. The original three-track constructor and its six-channel wire layout remain unchanged. Ring capacity is measured in complete frames for every layout.

## AI clip lifecycle

`gooey_engine_clip_load_and_launch` prepares/copies interleaved PCM on control and submits a slot replacement and launch as one transaction. Queue rejection leaves the prior projection intact. The existing launch quantization and phase-alignment policy applies. Pitch-stretcher scratch is created with each loop channel and retained across replacements. Clip slots and active buffers retire into bounded preallocated storage; command application waits when retirement is full. Call `gooey_engine_reclaim_loop_buffers` periodically on the serialized control thread to reclaim these buffers, including while the UI is idle. This never performs PCM destruction on render.
