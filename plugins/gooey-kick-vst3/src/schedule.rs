use crate::{KickAdapter, ParamId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScheduledEvent {
    Parameter {
        offset: usize,
        id: ParamId,
        value: f32,
    },
    NoteOn {
        offset: usize,
        velocity: f32,
    },
}

impl ScheduledEvent {
    pub fn offset(self) -> usize {
        match self {
            Self::Parameter { offset, .. } | Self::NoteOn { offset, .. } => offset,
        }
    }

    pub fn apply(self, adapter: &mut KickAdapter) {
        match self {
            Self::Parameter { id, value, .. } => adapter.set_parameter(id, value),
            Self::NoteOn { velocity, .. } => adapter.trigger(velocity),
        }
    }
}

/// Renders a pre-sorted event slice. This helper is used by tests and standalone
/// paths; the VST3 processor walks host queues directly to avoid allocating.
pub fn render_scheduled(
    adapter: &mut KickAdapter,
    frames: usize,
    events: &[ScheduledEvent],
    mut output: impl FnMut(usize, f32),
) {
    let mut event_index = 0;
    for frame in 0..frames {
        while let Some(event) = events.get(event_index).copied() {
            if event.offset() != frame {
                break;
            }
            event.apply(adapter);
            event_index += 1;
        }
        output(frame, adapter.next_sample());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Parameters;

    #[test]
    fn midi_and_automation_apply_at_exact_offsets() {
        let mut adapter = KickAdapter::new(48_000.0, Parameters::default());
        let events = [
            ScheduledEvent::Parameter {
                offset: 2,
                id: ParamId::Output,
                value: 0.0,
            },
            ScheduledEvent::NoteOn {
                offset: 3,
                velocity: 1.0,
            },
        ];
        let mut output = [0.0; 8];
        render_scheduled(&mut adapter, output.len(), &events, |index, sample| {
            output[index] = sample
        });
        assert_eq!(output[..3], [0.0; 3]);
        assert_eq!(adapter.parameters().get(ParamId::Output), 0.0);
    }

    #[test]
    fn first_and_last_frame_events_are_not_shifted() {
        let mut adapter = KickAdapter::new(48_000.0, Parameters::default());
        let events = [
            ScheduledEvent::NoteOn {
                offset: 0,
                velocity: 1.0,
            },
            ScheduledEvent::Parameter {
                offset: 7,
                id: ParamId::Drive,
                value: 1.0,
            },
        ];
        render_scheduled(&mut adapter, 8, &events, |_, _| {});
        assert_eq!(adapter.sample_counter(), 8);
        assert_eq!(adapter.parameters().get(ParamId::Drive), 1.0);
    }
}
