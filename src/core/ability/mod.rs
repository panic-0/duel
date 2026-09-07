pub mod attack;
pub use attack::Attack;

use super::{
    buff::{Buff, Priority},
    command::Commands,
    event::{Event, EventType},
    world::World,
    BuffId, PlayerId,
};

pub trait Ability: std::fmt::Debug {
    fn apply(&self, source_id: PlayerId, world: &World, commands: &mut Commands);
}

#[derive(Debug)]
pub struct Abilities {
    source_id: PlayerId,
    abilities: Vec<Box<dyn Ability>>,
}

impl Abilities {
    pub fn new(source_id: PlayerId, abilities: Vec<Box<dyn Ability>>) -> Self {
        Abilities {
            source_id,
            abilities,
        }
    }
}

impl Buff for Abilities {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Turn, Priority::Default)]
    }

    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        commands: &mut Commands,
        _buff_id: super::BuffId,
    ) {
        match *event {
            Event::Turn {
                round: _,
                player_id,
            } if player_id == self.source_id => {
                for ability in &self.abilities {
                    ability.apply(self.source_id, world, commands);
                }
            }
            _ => {}
        }
    }
}
