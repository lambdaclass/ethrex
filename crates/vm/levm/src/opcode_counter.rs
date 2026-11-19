//! Counts how many times each opcode runs.
//!
//! The counter lives in the [`GeneralizedDatabase`](crate::db::gen_db::GeneralizedDatabase),
//! so it sees every VM that executes over that database: the transactions of a
//! block and its system calls.

use crate::opcodes::Opcode;

/// How many times each opcode started executing, indexed by opcode byte.
///
/// An opcode counts when the interpreter reaches it, whether or not it then
/// succeeds, so one that runs out of gas or underflows the stack still counts.
/// Two things the dispatch loop goes through don't count, since neither is an
/// instruction of the code: a byte the fork doesn't define, and the implicit
/// `STOP` past the end of the code. These are the rules of the `CountTracer`
/// in execution-specs, which `--opcode.count` of its t8n reports.
#[derive(Debug, Clone)]
pub struct OpcodeCounter {
    counts: [u64; 256],
    /// The opcode [`Self::record`] counted last, until the next one starts.
    last: Option<u8>,
}

impl Default for OpcodeCounter {
    fn default() -> Self {
        Self {
            counts: [0; 256],
            last: None,
        }
    }
}

impl OpcodeCounter {
    /// Counts `opcode`. `in_code` says if the interpreter read it from the
    /// code, and is false for the implicit `STOP` past its end.
    pub fn record(&mut self, opcode: u8, in_code: bool) {
        if !in_code {
            self.last = None;
            return;
        }
        self.bump(opcode);
        self.last = Some(opcode);
    }

    /// Takes back the last [`Self::record`]. The handler of undefined opcodes
    /// calls it, since only there is it known that the fork has no such opcode.
    pub fn discard_last(&mut self) {
        if let Some(opcode) = self.last.take() {
            let count = self.count_mut(opcode);
            *count = count.saturating_sub(1);
        }
    }

    /// Counts the `JUMPDEST` that a taken `JUMP` or `JUMPI` lands on. The jump
    /// handler runs it inline, so the dispatch loop never sees it.
    pub fn record_fused_jumpdest(&mut self) {
        self.bump(Opcode::JUMPDEST.into());
    }

    /// The opcodes that ran at least once, in opcode order, with their counts.
    pub fn counts(&self) -> impl Iterator<Item = (u8, u64)> + '_ {
        (0..=u8::MAX)
            .zip(self.counts.iter().copied())
            .filter(|(_, count)| *count != 0)
    }

    fn bump(&mut self, opcode: u8) {
        let count = self.count_mut(opcode);
        *count = count.saturating_add(1);
    }

    fn count_mut(&mut self, opcode: u8) -> &mut u64 {
        #[expect(clippy::indexing_slicing, reason = "a u8 always indexes a 256 array")]
        &mut self.counts[usize::from(opcode)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_the_opcodes_read_from_the_code() {
        let mut counter = OpcodeCounter::default();
        counter.record(0x01, true);
        counter.record(0x01, true);
        counter.record(0x00, false);
        assert_eq!(counter.counts().collect::<Vec<_>>(), vec![(0x01, 2)]);
    }

    #[test]
    fn discards_an_undefined_opcode_once() {
        let mut counter = OpcodeCounter::default();
        counter.record(0x01, true);
        counter.record(0x0c, true);
        counter.discard_last();
        counter.discard_last();
        assert_eq!(counter.counts().collect::<Vec<_>>(), vec![(0x01, 1)]);
    }

    #[test]
    fn does_not_discard_across_an_implicit_stop() {
        let mut counter = OpcodeCounter::default();
        counter.record(0x01, true);
        counter.record(0x00, false);
        counter.discard_last();
        assert_eq!(counter.counts().collect::<Vec<_>>(), vec![(0x01, 1)]);
    }

    #[test]
    fn counts_the_fused_jumpdest() {
        let mut counter = OpcodeCounter::default();
        counter.record(0x56, true);
        counter.record_fused_jumpdest();
        assert_eq!(
            counter.counts().collect::<Vec<_>>(),
            vec![(0x56, 1), (0x5b, 1)]
        );
    }
}
