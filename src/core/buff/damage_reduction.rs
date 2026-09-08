use super::super::operation::DamageContext;
use super::super::{log::LogEntry, state::GameState, BuffId, PlayerId};
use super::{Buff, Priority};

#[derive(Debug)]
pub struct DamageReduction {
    pub target_id: PlayerId,
    pub reduction_ratio: f64,
}

impl DamageReduction {
    pub fn new(target_id: PlayerId, reduction_ratio: f64) -> Self {
        DamageReduction {
            target_id,
            reduction_ratio,
        }
    }
}

impl Buff for DamageReduction {
    fn owner(&self) -> Option<PlayerId> {
        Some(self.target_id)
    }

    fn damage_modification(&self) -> Option<Priority> {
        Some(Priority::Modify)
    }

    fn modify_damage(&self, context: &mut DamageContext, world: &GameState, _buff_id: BuffId) {
        if context.target_id != self.target_id {
            return;
        }
        let original = context.amount;
        let reduced = (original as f64 * (1.0 - self.reduction_ratio)) as u64;
        context.reduce_to(reduced);
        world.log(LogEntry::DamageReduced {
            target_id: self.target_id,
            original,
            reduced,
        });
    }
}
