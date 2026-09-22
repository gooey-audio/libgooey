#include "gooey.h"

#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int fail(const char *message) {
  fprintf(stderr, "live-control C ABI failure: %s\n", message);
  return 1;
}

int main(void) {
  GooeyEngine *engine = gooey_engine_new(44100.0f);
  if (engine == NULL) {
    return fail("engine creation returned null");
  }
  GooeyLiveControl *control = gooey_engine_live_control_new(engine);
  if (control == NULL) {
    gooey_engine_free(engine);
    return fail("live-control attachment returned null");
  }

  GooeyDrumPattern pattern;
  memset(&pattern, 0, sizeof(pattern));
  pattern.lanes[GOOEY_DRUM_LANE_KICK][0].enabled = 1;
  pattern.lanes[GOOEY_DRUM_LANE_KICK][0].velocity = 0.8f;

  const uint64_t generation =
      gooey_live_control_submit_drum_pattern(control, &pattern);
  if (generation == 0) {
    return fail("drum-pattern submission was rejected");
  }

  float boundary_frame[GOOEY_OUTPUT_CHANNELS] = {0.0f};
  gooey_engine_render(engine, boundary_frame, 1);
  if (gooey_live_control_get_last_applied_generation(control) != generation) {
    return fail("drum-pattern generation was not applied");
  }
  if (!gooey_engine_sequencer_get_instrument_step_enabled(
          engine, INSTRUMENT_KICK, 0)) {
    return fail("kick step zero is disabled after application");
  }
  const float velocity = gooey_engine_sequencer_get_instrument_step_velocity(
      engine, INSTRUMENT_KICK, 0);
  if (fabsf(velocity - 0.8f) > 0.000001f) {
    return fail("kick step zero has the wrong velocity");
  }

  gooey_engine_sequencer_start(engine);
  float output[4096 * GOOEY_OUTPUT_CHANNELS] = {0.0f};
  gooey_engine_render(engine, output, 4096);
  float peak = 0.0f;
  for (size_t i = 0; i < sizeof(output) / sizeof(output[0]); ++i) {
    if (!isfinite(output[i])) {
      return fail("rendered output contains a nonfinite sample");
    }
    const float magnitude = fabsf(output[i]);
    if (magnitude > peak) {
      peak = magnitude;
    }
  }
  if (peak <= 0.000001f) {
    return fail("enabled kick pattern rendered silence");
  }

  gooey_live_control_free(control);
  gooey_engine_free(engine);
  return 0;
}
