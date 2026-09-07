use super::super::{
    command::Commands, event::Event, log::LogEntry, state::GameState, BuffId, PlayerId,
};
use super::{Buff, EventType, Priority};

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
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::PlayerAttack, Priority::Modify)]
    }

    fn on_event(
        &mut self,
        event: &mut Event,
        world: &GameState,
        _commands: &mut Commands,
        _buff_id: BuffId,
    ) {
        match event {
            Event::PlayerAttack {
                target_id, damage, ..
            } if *target_id == self.target_id => {
                let original_damage = *damage;
                let reduced_damage = (original_damage as f64 * (1.0 - self.reduction_ratio)) as u64;
                *damage = reduced_damage;

                world.log(LogEntry::DamageReduced {
                    target_id: self.target_id,
                    original: original_damage,
                    reduced: reduced_damage,
                });
            }
            _ => {}
        }
    }
}
