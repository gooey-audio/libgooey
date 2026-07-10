# Mixer Graph Migration Guide

This guide is for host apps that currently treat libgooey as one flat instrument
mix and do not yet model multi-channel submixing.

## What Changed

The engine still renders one interleaved stereo output buffer through
`gooey_engine_render`, so audio callback integration does not need to change.
Internally, instrument and loop sources now route through a host-configurable
mixer graph before the master bus.

Default graph layout:

| Track | Name | Sources |
| --- | --- | --- |
| 0 | Drums | `SOURCE_DRUMKIT` |
| 1 | Bass | `SOURCE_BASS` |
| 2 | Synth | `SOURCE_POLYSYNTH` |
| 3 | Loops | `SOURCE_GRANULATOR`, `SOURCE_LOOPMIXER` |

The default layout is intended to preserve the old app behavior until the host
starts changing track routing, track gain, mute/solo state, or track effects.

Registered sampler racks are intentionally not part of the default layout.
Each rack exposes `SOURCE_SAMPLER_BASE + rack_id`; add or choose a track and
route that source after `gooey_engine_sampler_register`. This keeps existing
four-track projects and source IDs unchanged.

## Minimal Migration

Apps that only need the old flat mix can keep their current render path:

```c
GooeyEngine *engine = gooey_engine_new(44100.0f);

// Existing instrument, sequencer, global-effect, and render calls still work.
gooey_engine_set_bpm(engine, 120.0f);
gooey_engine_sequencer_start(engine);
gooey_engine_render(engine, output, frame_count);
```

Recommended additions:

```c
// Optional: confirm the default graph is present after engine creation.
uint32_t tracks = gooey_engine_mixer_get_track_count(engine); // 4

// Optional: reset to default if the host loads saved projects from mixed app versions.
gooey_engine_mixer_reset_default_layout(engine);
```

## Adding A Submix UI

Treat mixer graph tracks as app-level buses. A simple v1 UI can show:

- Track name
- Gain
- Mute
- Solo
- Peak meter
- Track effect count

Example read loop:

```c
uint32_t count = gooey_engine_mixer_get_track_count(engine);
for (uint32_t track = 0; track < count; track++) {
    const char *name = gooey_engine_mixer_get_track_name(engine, track);
    float gain = gooey_engine_mixer_get_track_gain(engine, track);
    bool muted = gooey_engine_mixer_get_track_mute(engine, track);
    bool soloed = gooey_engine_mixer_get_track_solo(engine, track);
    float peak = gooey_engine_mixer_get_track_peak(engine, track); // read-and-reset
}
```

Example write controls:

```c
gooey_engine_mixer_set_track_gain(engine, 0, 0.85f);
gooey_engine_mixer_set_track_mute(engine, 1, true);
gooey_engine_mixer_set_track_solo(engine, 0, true);
```

Track names returned by `gooey_engine_mixer_get_track_name` are owned by the
engine. Do not free them. Treat the pointer as valid until the track is renamed,
the graph layout is cleared/reset, or the engine is freed.

## Custom Layouts

Apps that want their own bus layout should clear the graph, add tracks, and route
sources explicitly during project load or engine setup.

```c
gooey_engine_mixer_clear_layout(engine);

int drums = gooey_engine_mixer_add_track(engine, "Drum Bus");
int bass = gooey_engine_mixer_add_track(engine, "Bass Bus");
int loops = gooey_engine_mixer_add_track(engine, "Loops");

gooey_engine_mixer_route_source(engine, SOURCE_DRUMKIT, drums);
gooey_engine_mixer_route_source(engine, SOURCE_BASS, bass);
gooey_engine_mixer_route_source(engine, SOURCE_GRANULATOR, loops);
gooey_engine_mixer_route_source(engine, SOURCE_LOOPMIXER, loops);
```

If a source is unrouted, it is silent:

```c
gooey_engine_mixer_unroute_source(engine, SOURCE_POLYSYNTH);
```

Route queries return `-1` for invalid or unrouted sources:

```c
int track = gooey_engine_mixer_get_source_route(engine, SOURCE_BASS);
```

## Track Effects

Track effects use the same `EFFECT_*` ids and parameter ids as loop-channel
effects.

```c
int slot = gooey_engine_track_effect_add(engine, 0, EFFECT_LOWPASS_FILTER);
gooey_engine_track_effect_set_param(engine, 0, slot, FILTER_PARAM_CUTOFF, 800.0f);

gooey_engine_track_effect_add(engine, 1, EFFECT_DELAY);
gooey_engine_track_effect_set_param(engine, 1, 0, DELAY_PARAM_MIX, 0.25f);

uint32_t effect_count = gooey_engine_track_effect_count(engine, 1);
int effect_id = gooey_engine_track_effect_type_at(engine, 1, 0);
```

Invalid track/effect slots are safe no-ops or return `false`, `0`, or `-1`
depending on the function.

## Backward Compatibility Notes

- `gooey_engine_render` still writes interleaved stereo frames.
- Existing instrument APIs such as `gooey_engine_set_kick_param`,
  `gooey_engine_trigger_instrument`, sequencer APIs, LFO APIs, and global effects
  remain valid.
- Per-instrument gain/mute/solo still applies inside the drum kit/bass voice
  layer. Track gain/mute/solo applies later at the mixer graph bus layer.
- The drum kit source is the sum of kick, snare, hi-hat, and tom. Apps that need
  individual drum faders can keep using existing per-instrument gain controls.
- Project files that do not store graph layout can rely on the default layout.
  Project files that do store graph layout should recreate it with
  `clear_layout` + `add_track` + `route_source`.

## Manual Verification

Run the included example:

```bash
cargo run --example multi_channel_submix --features native,crossterm
```

It creates two mixer graph tracks:

- Track 1: a basic drum beat routed from `SOURCE_DRUMKIT`
- Track 2: a bass-synth loop routed from `SOURCE_BASS`

Use the terminal controls to adjust track gain, mute/solo, and track effects.
