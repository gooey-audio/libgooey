//! Macros: one normalized 0-1 control connected to many parameters.
//!
//! Each mapping stores the parameter value at macro 0 (`from`) and at macro 1
//! (`to`), so a macro keeps a durable reference to the parameters it drives.
//! A macro writes its parameters only when its value changes; a manual edit of
//! a mapped parameter therefore sticks until that macro moves again.

pub const MACRO_COUNT: usize = 16;
pub const MACRO_MAX_MAPPINGS: usize = 16;

/// Opaque parameter address. The engine that owns the parameters assigns the
/// meaning of `kind`, `index`, and `param`; this module only compares them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParamTarget {
    pub kind: u32,
    pub index: u32,
    pub param: u32,
}

impl ParamTarget {
    pub const fn new(kind: u32, index: u32, param: u32) -> Self {
        Self { kind, index, param }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MacroMapping {
    pub target: ParamTarget,
    /// Parameter value when the macro is at 0.
    pub from: f32,
    /// Parameter value when the macro is at 1. May be below `from`.
    pub to: f32,
}

impl MacroMapping {
    /// Parameter value for a macro position. The position is clamped to 0-1.
    pub fn value_at(&self, position: f32) -> f32 {
        let position = position.clamp(0.0, 1.0);
        self.from + (self.to - self.from) * position
    }
}

/// Fixed-capacity mapping list. `Copy` so it can cross the render boundary
/// inline without allocating or freeing on the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroDefinition {
    mappings: [MacroMapping; MACRO_MAX_MAPPINGS],
    len: usize,
}

impl Default for MacroDefinition {
    fn default() -> Self {
        Self {
            mappings: [MacroMapping::default(); MACRO_MAX_MAPPINGS],
            len: 0,
        }
    }
}

impl MacroDefinition {
    pub fn mappings(&self) -> &[MacroMapping] {
        &self.mappings[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn find(&self, target: ParamTarget) -> Option<&MacroMapping> {
        self.mappings().iter().find(|m| m.target == target)
    }

    /// Replace the mapping for the same target, or append a new one.
    /// Returns false only when a new mapping would exceed the capacity.
    pub fn upsert(&mut self, mapping: MacroMapping) -> bool {
        if let Some(existing) = self.mappings[..self.len]
            .iter_mut()
            .find(|m| m.target == mapping.target)
        {
            *existing = mapping;
            return true;
        }
        if self.len == MACRO_MAX_MAPPINGS {
            return false;
        }
        self.mappings[self.len] = mapping;
        self.len += 1;
        true
    }

    /// Remove the mapping for `target`, preserving the order of the rest.
    pub fn remove(&mut self, target: ParamTarget) -> bool {
        let Some(position) = self.mappings().iter().position(|m| m.target == target) else {
            return false;
        };
        self.mappings.copy_within(position + 1..self.len, position);
        self.len -= 1;
        self.mappings[self.len] = MacroMapping::default();
        true
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Render-owned macro state: definitions, values, and pending writes.
pub struct MacroBank {
    definitions: [MacroDefinition; MACRO_COUNT],
    values: [f32; MACRO_COUNT],
    dirty: [bool; MACRO_COUNT],
}

impl Default for MacroBank {
    fn default() -> Self {
        Self::new()
    }
}

impl MacroBank {
    pub fn new() -> Self {
        Self {
            definitions: [MacroDefinition::default(); MACRO_COUNT],
            values: [0.0; MACRO_COUNT],
            dirty: [false; MACRO_COUNT],
        }
    }

    /// Replace a definition without writing its parameters. Parameters follow
    /// the new mappings the next time the macro value changes.
    pub fn replace(&mut self, index: usize, definition: MacroDefinition) {
        if let Some(slot) = self.definitions.get_mut(index) {
            *slot = definition;
        }
    }

    pub fn definition(&self, index: usize) -> Option<&MacroDefinition> {
        self.definitions.get(index)
    }

    pub fn value(&self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or(0.0)
    }

    /// Set a macro position (clamped to 0-1). Marks the macro for writing only
    /// when the value actually changes.
    pub fn set_value(&mut self, index: usize, value: f32) {
        if index >= MACRO_COUNT || !value.is_finite() {
            return;
        }
        let value = value.clamp(0.0, 1.0);
        if self.values[index] != value {
            self.values[index] = value;
            self.dirty[index] = true;
        }
    }

    /// Mark a macro for writing even if its value is unchanged. Running
    /// motions use this to reassert their parameters over other writers.
    pub fn touch(&mut self, index: usize) {
        if let Some(dirty) = self.dirty.get_mut(index) {
            *dirty = true;
        }
    }

    /// Clear a macro's dirty flag, returning a copy of its definition and
    /// value when it needed writing. Returning a copy lets the owning engine
    /// write parameters without holding a borrow of the bank.
    pub fn take_dirty(&mut self, index: usize) -> Option<(MacroDefinition, f32)> {
        let dirty = self.dirty.get_mut(index)?;
        if !std::mem::take(dirty) {
            return None;
        }
        Some((self.definitions[index], self.values[index]))
    }

    /// Write every dirty macro's parameters through `write`, in macro index
    /// order (so a higher macro wins when two map the same parameter).
    pub fn flush(&mut self, mut write: impl FnMut(ParamTarget, f32)) {
        for index in 0..MACRO_COUNT {
            if let Some((definition, value)) = self.take_dirty(index) {
                for mapping in definition.mappings() {
                    write(mapping.target, mapping.value_at(value));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(param: u32) -> ParamTarget {
        ParamTarget::new(0, 0, param)
    }

    fn mapping(param: u32, from: f32, to: f32) -> MacroMapping {
        MacroMapping {
            target: target(param),
            from,
            to,
        }
    }

    #[test]
    fn mapping_interpolates_and_clamps_position() {
        let m = mapping(0, 0.2, 0.6);
        assert_eq!(m.value_at(0.0), 0.2);
        assert!((m.value_at(0.5) - 0.4).abs() < 1e-6);
        assert!((m.value_at(1.0) - 0.6).abs() < 1e-6);
        assert!((m.value_at(2.0) - 0.6).abs() < 1e-6);
        assert_eq!(m.value_at(-1.0), 0.2);
    }

    #[test]
    fn inverted_mapping_moves_downward() {
        let m = mapping(0, 0.9, 0.1);
        assert!((m.value_at(0.25) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn upsert_replaces_same_target_and_rejects_overflow() {
        let mut def = MacroDefinition::default();
        assert!(def.upsert(mapping(1, 0.0, 1.0)));
        assert!(def.upsert(mapping(1, 0.5, 0.5)));
        assert_eq!(def.len(), 1);
        assert_eq!(def.find(target(1)).unwrap().from, 0.5);
        for param in 2..(MACRO_MAX_MAPPINGS as u32 + 1) {
            assert!(def.upsert(mapping(param, 0.0, 1.0)));
        }
        assert_eq!(def.len(), MACRO_MAX_MAPPINGS);
        assert!(!def.upsert(mapping(99, 0.0, 1.0)));
        assert!(def.upsert(mapping(1, 0.1, 0.2)));
    }

    #[test]
    fn remove_preserves_order() {
        let mut def = MacroDefinition::default();
        for param in 0..4 {
            def.upsert(mapping(param, 0.0, 1.0));
        }
        assert!(def.remove(target(1)));
        assert!(!def.remove(target(1)));
        let params: Vec<u32> = def.mappings().iter().map(|m| m.target.param).collect();
        assert_eq!(params, vec![0, 2, 3]);
    }

    #[test]
    fn bank_writes_only_on_change() {
        let mut bank = MacroBank::new();
        let mut def = MacroDefinition::default();
        def.upsert(mapping(7, 0.0, 1.0));
        bank.replace(0, def);

        let mut writes = Vec::new();
        bank.flush(|t, v| writes.push((t.param, v)));
        assert!(writes.is_empty(), "replacing a definition does not write");

        bank.set_value(0, 0.5);
        bank.flush(|t, v| writes.push((t.param, v)));
        assert_eq!(writes, vec![(7, 0.5)]);

        writes.clear();
        bank.set_value(0, 0.5);
        bank.flush(|t, v| writes.push((t.param, v)));
        assert!(writes.is_empty(), "unchanged value does not write");

        bank.touch(0);
        bank.flush(|t, v| writes.push((t.param, v)));
        assert_eq!(writes, vec![(7, 0.5)]);
    }

    #[test]
    fn bank_clamps_and_ignores_non_finite_values() {
        let mut bank = MacroBank::new();
        bank.set_value(0, 3.0);
        assert_eq!(bank.value(0), 1.0);
        bank.set_value(0, f32::NAN);
        assert_eq!(bank.value(0), 1.0);
        bank.set_value(MACRO_COUNT, 0.5);
        assert_eq!(bank.value(MACRO_COUNT), 0.0);
    }
}
