use colored::Colorize;

use super::super::{command::Commands, event::Event, world::World};
use super::{Buff, Priority, EventType};
use super::super::PlayerId;

#[derive(Debug)]
pub struct DamageReduction {
    pub target_id: PlayerId,
    pub reduction_ratio: f64,
    pub remaining_duration: Option<u32>,
}

impl DamageReduction {
    pub fn new(target_id: PlayerId, reduction_ratio: f64) -> Self {
        DamageReduction {
            target_id,
            reduction_ratio,
            remaining_duration: None,
        }
    }
}

impl Buff for DamageReduction {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::PlayerAttack, Priority::Modify)]
    }

    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        _commands: &mut Commands,
        _buff_id: super::super::BuffId,
    ) {
        match event {
            Event::PlayerAttack {
                target_id, damage, ..
            } if *target_id == self.target_id => {
                let original_damage = *damage;
                let reduced_damage = (original_damage as f64 * (1.0 - self.reduction_ratio)) as u64;
                *damage = reduced_damage;

                if let Some(player) = world.get_player(self.target_id) {
                    println!(
                        "{} 的减伤效果触发！伤害从 {} 降低到 {}",
                        player.name().blue(),
                        original_damage.to_string().bright_red(),
                        reduced_damage.to_string().yellow()
                    );
                }
            }
            _ => {}
        }
    }
}
